pub mod edge;
pub mod node;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::Directed;
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;

use bm25::SearchEngineBuilder;
use edge::EdgeKind;
use node::{ExternalPackageInfo, FileInfo, GraphNode, SymbolInfo, SymbolKind};

/// The in-memory code graph: a directed petgraph StableGraph with O(1) lookup indexes.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CodeGraph {
    /// The underlying directed graph, parameterised over node and edge kinds.
    pub graph: StableGraph<GraphNode, EdgeKind, Directed>,
    /// Maps file paths to their node indices for O(1) lookup.
    pub file_index: HashMap<PathBuf, NodeIndex>,
    /// Maps symbol names to all node indices bearing that name (one name may appear in many files).
    pub symbol_index: HashMap<String, Vec<NodeIndex>>,
    /// Maps external package names to their node indices for deduplication.
    pub external_index: HashMap<String, NodeIndex>,
    /// Maps Rust built-in crate names (`"std"`, `"core"`, `"alloc"`) to their node indices.
    /// Used to deduplicate `GraphNode::Builtin` nodes — one per crate name.
    pub builtin_index: HashMap<String, NodeIndex>,
    /// Transient BM25 full-text search index over symbol names.
    /// Not serialized — rebuilt after cache load and watcher events. Used by plan 20-01.
    #[serde(skip)]
    pub bm25_index: Option<bm25::SearchEngine<u32>>,
}

impl Clone for CodeGraph {
    /// Clone the graph data structures but reset bm25_index to None.
    /// Call `rebuild_bm25_index()` after cloning if BM25 search is needed.
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            file_index: self.file_index.clone(),
            symbol_index: self.symbol_index.clone(),
            external_index: self.external_index.clone(),
            builtin_index: self.builtin_index.clone(),
            bm25_index: None,
        }
    }
}

impl CodeGraph {
    /// Create an empty code graph.
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            file_index: HashMap::new(),
            symbol_index: HashMap::new(),
            external_index: HashMap::new(),
            builtin_index: HashMap::new(),
            bm25_index: None,
        }
    }

    /// Add a file node to the graph. Returns the new node's index.
    /// If the file has already been added, returns the existing index.
    ///
    /// `crate_name` is `None` for TypeScript/JavaScript files. Callers that process Rust
    /// files may update the `FileInfo.crate_name` field after calling `add_file`.
    pub fn add_file(&mut self, path: PathBuf, language: &str) -> NodeIndex {
        if let Some(&existing) = self.file_index.get(&path) {
            return existing;
        }
        let info = FileInfo {
            path: path.clone(),
            language: language.to_owned(),
            crate_name: None,
            kind: node::FileKind::Source,
        };
        let idx = self.graph.add_node(GraphNode::File(info));
        self.file_index.insert(path, idx);
        idx
    }

    /// Add a non-parsed file node to the graph. Returns the new node's index.
    /// If the file has already been added, returns the existing index.
    ///
    /// Non-parsed files have no symbol extraction or import resolution.
    /// They appear as File nodes with a kind tag (doc, config, ci, asset, other).
    pub fn add_non_parsed_file(&mut self, path: PathBuf, kind: node::FileKind) -> NodeIndex {
        if let Some(&existing) = self.file_index.get(&path) {
            return existing;
        }
        let info = FileInfo {
            path: path.clone(),
            language: String::new(),
            crate_name: None,
            kind,
        };
        let idx = self.graph.add_node(GraphNode::File(info));
        self.file_index.insert(path, idx);
        idx
    }

    /// Add a top-level symbol node for `file_idx` with a `Contains` edge from the file.
    /// Returns the symbol's node index.
    pub fn add_symbol(&mut self, file_idx: NodeIndex, info: SymbolInfo) -> NodeIndex {
        let name = info.name.clone();
        let sym_idx = self.graph.add_node(GraphNode::Symbol(info));
        self.graph.add_edge(file_idx, sym_idx, EdgeKind::Contains);
        self.symbol_index.entry(name).or_default().push(sym_idx);
        sym_idx
    }

    /// Add a child symbol node (e.g. interface property, class method) with a `ChildOf` edge
    /// from the parent symbol node.
    /// Returns the child symbol's node index.
    pub fn add_child_symbol(&mut self, parent_idx: NodeIndex, info: SymbolInfo) -> NodeIndex {
        let name = info.name.clone();
        let child_idx = self.graph.add_node(GraphNode::Symbol(info));
        self.graph
            .add_edge(child_idx, parent_idx, EdgeKind::ChildOf);
        self.symbol_index.entry(name).or_default().push(child_idx);
        child_idx
    }

    /// Number of file nodes in the graph.
    pub fn file_count(&self) -> usize {
        self.file_index.len()
    }

    /// Number of symbol nodes in the graph (excludes file nodes).
    pub fn symbol_count(&self) -> usize {
        self.graph
            .node_indices()
            .filter(|&i| matches!(self.graph[i], GraphNode::Symbol(_)))
            .count()
    }

    /// Return a count of symbols broken down by kind.
    pub fn symbols_by_kind(&self) -> HashMap<SymbolKind, usize> {
        let mut map: HashMap<SymbolKind, usize> = HashMap::new();
        for idx in self.graph.node_indices() {
            if let GraphNode::Symbol(ref info) = self.graph[idx] {
                *map.entry(info.kind.clone()).or_insert(0) += 1;
            }
        }
        map
    }

    // -------------------------------------------------------------------------
    // Phase 2: helper methods for new edge and node types
    // -------------------------------------------------------------------------

    /// Add a `ResolvedImport` edge from `from` to `to`.
    /// `specifier` is the original raw import string as written in source.
    pub fn add_resolved_import(&mut self, from: NodeIndex, to: NodeIndex, specifier: &str) {
        self.graph.add_edge(
            from,
            to,
            EdgeKind::ResolvedImport {
                specifier: specifier.to_owned(),
            },
        );
    }

    /// Add (or reuse) an `ExternalPackage` node for `name` and add a `ResolvedImport` edge
    /// from `from` to it. `specifier` is the original import string.
    ///
    /// If a node for this package name already exists in the graph, it is reused and no
    /// duplicate node is created.
    ///
    /// Returns the `NodeIndex` of the external package node.
    pub fn add_external_package(
        &mut self,
        from: NodeIndex,
        name: &str,
        specifier: &str,
    ) -> NodeIndex {
        let pkg_idx = if let Some(&existing) = self.external_index.get(name) {
            existing
        } else {
            let info = ExternalPackageInfo {
                name: name.to_owned(),
                version: None,
            };
            let idx = self.graph.add_node(GraphNode::ExternalPackage(info));
            self.external_index.insert(name.to_owned(), idx);
            idx
        };
        self.graph.add_edge(
            from,
            pkg_idx,
            EdgeKind::ResolvedImport {
                specifier: specifier.to_owned(),
            },
        );
        pkg_idx
    }

    /// Add (or reuse) a `Builtin` node for a Rust built-in crate (`std`, `core`, `alloc`) and
    /// add a `ResolvedImport` edge from `from` to it.
    ///
    /// Builtin nodes are terminal — the resolver stops at them, just like `ExternalPackage`.
    /// `name` should be the crate-level name (e.g. `"std"`), not the full path.
    /// `specifier` is the original use path as written in source (e.g. `"std::collections::HashMap"`).
    ///
    /// Returns the `NodeIndex` of the Builtin node (deduped by name).
    pub fn add_builtin_node(&mut self, from: NodeIndex, name: &str, specifier: &str) -> NodeIndex {
        let node_idx = if let Some(&existing) = self.builtin_index.get(name) {
            existing
        } else {
            let idx = self.graph.add_node(GraphNode::Builtin {
                name: name.to_owned(),
            });
            self.builtin_index.insert(name.to_owned(), idx);
            idx
        };
        self.graph.add_edge(
            from,
            node_idx,
            EdgeKind::ResolvedImport {
                specifier: specifier.to_owned(),
            },
        );
        node_idx
    }

    /// Add an `UnresolvedImport` node (a sentinel capturing an unresolvable import) and a
    /// `ResolvedImport` edge from `from` to it.
    ///
    /// Returns the `NodeIndex` of the new unresolved-import node.
    pub fn add_unresolved_import(
        &mut self,
        from: NodeIndex,
        specifier: &str,
        reason: &str,
    ) -> NodeIndex {
        let node = GraphNode::UnresolvedImport {
            specifier: specifier.to_owned(),
            reason: reason.to_owned(),
        };
        let idx = self.graph.add_node(node);
        self.graph.add_edge(
            from,
            idx,
            EdgeKind::ResolvedImport {
                specifier: specifier.to_owned(),
            },
        );
        idx
    }

    /// Add a `Calls` edge from `caller` to `callee`.
    pub fn add_calls_edge(&mut self, caller: NodeIndex, callee: NodeIndex) {
        self.graph.add_edge(caller, callee, EdgeKind::Calls);
    }

    /// Add an `Extends` edge from `child` to `parent`.
    pub fn add_extends_edge(&mut self, child: NodeIndex, parent: NodeIndex) {
        self.graph.add_edge(child, parent, EdgeKind::Extends);
    }

    /// Add an `Implements` edge from `class_idx` to `iface_idx`.
    pub fn add_implements_edge(&mut self, class_idx: NodeIndex, iface_idx: NodeIndex) {
        self.graph
            .add_edge(class_idx, iface_idx, EdgeKind::Implements);
    }

    /// Add a `BarrelReExportAll` edge from `barrel` to `source`.
    pub fn add_barrel_reexport_all(&mut self, barrel: NodeIndex, source: NodeIndex) {
        self.graph
            .add_edge(barrel, source, EdgeKind::BarrelReExportAll);
    }

    /// Remove a file and all its owned nodes/edges from the graph.
    ///
    /// Removes: the file node, all Symbol nodes connected via Contains edges,
    /// all child symbols (via ChildOf edges from those symbols), and all edges
    /// to/from any of these nodes. Also cleans up file_index and symbol_index.
    pub fn remove_file_from_graph(&mut self, path: &Path) {
        let file_idx = match self.file_index.remove(path) {
            Some(idx) => idx,
            None => return, // file not in graph
        };

        // Collect symbol nodes owned by this file (Contains edges from file)
        let mut nodes_to_remove = vec![file_idx];
        let symbol_indices: Vec<NodeIndex> = self
            .graph
            .edges(file_idx)
            .filter(|e| matches!(e.weight(), EdgeKind::Contains))
            .map(|e| e.target())
            .collect();

        for &sym_idx in &symbol_indices {
            nodes_to_remove.push(sym_idx);
            // Also collect child symbols (ChildOf edges pointing TO this symbol)
            let children: Vec<NodeIndex> = self
                .graph
                .edges_directed(sym_idx, petgraph::Direction::Incoming)
                .filter(|e| matches!(e.weight(), EdgeKind::ChildOf))
                .map(|e| e.source())
                .collect();
            nodes_to_remove.extend(children);
        }

        // Clean up symbol_index for all symbol nodes being removed
        for &node_idx in &nodes_to_remove {
            if let Some(GraphNode::Symbol(info)) = self.graph.node_weight(node_idx) {
                let name = info.name.clone();
                if let Some(indices) = self.symbol_index.get_mut(&name) {
                    indices.retain(|&i| i != node_idx);
                    if indices.is_empty() {
                        self.symbol_index.remove(&name);
                    }
                }
            }
        }

        // Remove all nodes (StableGraph removes associated edges automatically)
        for node_idx in nodes_to_remove {
            self.graph.remove_node(node_idx);
        }
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Split camelCase and snake_case identifiers into space-separated words.
/// Prepends the original name so exact matches still score high.
/// "authHandler" -> "authHandler auth handler"
/// "parse_token" -> "parse_token parse token"
/// Used by rebuild_bm25_index (plan 20-01).
pub fn split_identifier(name: &str) -> String {
    let mut words = Vec::new();
    let mut current_word = String::new();

    for ch in name.chars() {
        if ch == '_' {
            if !current_word.is_empty() {
                words.push(current_word.clone());
                current_word.clear();
            }
        } else if ch.is_uppercase() && !current_word.is_empty() {
            words.push(current_word.clone());
            current_word.clear();
            current_word.push(ch);
        } else {
            current_word.push(ch);
        }
    }
    if !current_word.is_empty() {
        words.push(current_word);
    }

    let split_lower = words
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} {}", name, split_lower)
}

impl CodeGraph {
    /// Rebuild the BM25 full-text search index from the current symbol_index.
    /// Called after graph construction, cache load, and watcher events. Used by plan 20-01.
    pub fn rebuild_bm25_index(&mut self) {
        let mut engine = SearchEngineBuilder::<u32>::with_avgdl(8.0).build();
        for (name, indices) in &self.symbol_index {
            let processed = split_identifier(name);
            for &idx in indices {
                engine.upsert(bm25::Document {
                    id: idx.index() as u32,
                    contents: processed.clone(),
                });
            }
        }
        self.bm25_index = Some(engine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node::SymbolKind;

    #[test]
    fn test_add_file_and_symbol() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("test.ts"), "typescript");
        let s = graph.add_symbol(
            f,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );
        assert_eq!(graph.file_count(), 1, "should have one file node");
        assert_eq!(graph.symbol_count(), 1, "should have one symbol node");
        // Verify edge exists: file -> symbol (Contains)
        assert!(
            graph.graph.contains_edge(f, s),
            "Contains edge should exist from file to symbol"
        );
    }

    #[test]
    fn test_add_duplicate_file_returns_same_index() {
        let mut graph = CodeGraph::new();
        let idx1 = graph.add_file(PathBuf::from("app.ts"), "typescript");
        let idx2 = graph.add_file(PathBuf::from("app.ts"), "typescript");
        assert_eq!(
            idx1, idx2,
            "duplicate add_file should return the same index"
        );
        assert_eq!(graph.file_count(), 1);
    }

    #[test]
    fn test_add_child_symbol() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("types.ts"), "typescript");
        let iface = graph.add_symbol(
            f,
            SymbolInfo {
                name: "IUser".into(),
                kind: SymbolKind::Interface,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );
        let prop = graph.add_child_symbol(
            iface,
            SymbolInfo {
                name: "name".into(),
                kind: SymbolKind::Property,
                line: 2,
                col: 2,
                ..Default::default()
            },
        );
        assert_eq!(
            graph.symbol_count(),
            2,
            "should count both interface and property"
        );
        // ChildOf edge goes from child to parent
        assert!(
            graph.graph.contains_edge(prop, iface),
            "ChildOf edge should go from child to parent"
        );
    }

    #[test]
    fn test_symbols_by_kind() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("mod.ts"), "typescript");
        let kinds = [
            SymbolKind::Function,
            SymbolKind::Function,
            SymbolKind::Class,
        ];
        for kind in kinds {
            graph.add_symbol(
                f,
                SymbolInfo {
                    name: "x".into(),
                    kind,
                    line: 1,
                    ..Default::default()
                },
            );
        }
        let breakdown = graph.symbols_by_kind();
        assert_eq!(breakdown.get(&SymbolKind::Function), Some(&2));
        assert_eq!(breakdown.get(&SymbolKind::Class), Some(&1));
    }

    // -------------------------------------------------------------------------
    // Phase 2 tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_add_external_package_dedup() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("src/app.ts"), "typescript");

        let idx1 = graph.add_external_package(f, "react", "react");
        let idx2 = graph.add_external_package(f, "react", "react");
        assert_eq!(
            idx1, idx2,
            "add_external_package with the same package name should return the same NodeIndex"
        );

        // Verify the node exists and is an ExternalPackage
        match &graph.graph[idx1] {
            GraphNode::ExternalPackage(info) => {
                assert_eq!(info.name, "react");
            }
            other => panic!("expected ExternalPackage node, got {:?}", other),
        }
    }

    #[test]
    fn test_add_unresolved_import_creates_node_and_edge() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("src/app.ts"), "typescript");

        let unresolved_idx = graph.add_unresolved_import(f, "./missing-module", "file not found");

        // Verify node is an UnresolvedImport
        match &graph.graph[unresolved_idx] {
            GraphNode::UnresolvedImport { specifier, reason } => {
                assert_eq!(specifier, "./missing-module");
                assert_eq!(reason, "file not found");
            }
            other => panic!("expected UnresolvedImport node, got {:?}", other),
        }

        // Verify edge exists from file to unresolved node
        assert!(
            graph.graph.contains_edge(f, unresolved_idx),
            "edge should exist from file to UnresolvedImport node"
        );
    }

    #[test]
    fn test_add_resolved_import_creates_edge() {
        let mut graph = CodeGraph::new();
        let f1 = graph.add_file(PathBuf::from("src/app.ts"), "typescript");
        let f2 = graph.add_file(PathBuf::from("src/utils.ts"), "typescript");

        graph.add_resolved_import(f1, f2, "./utils");

        assert!(
            graph.graph.contains_edge(f1, f2),
            "ResolvedImport edge should exist from importing file to target file"
        );
    }

    #[test]
    fn test_add_builtin_node_dedup() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("src/main.rs"), "rust");

        let idx1 = graph.add_builtin_node(f, "std", "std::collections::HashMap");
        let idx2 = graph.add_builtin_node(f, "std", "std::fmt::Debug");
        assert_eq!(
            idx1, idx2,
            "add_builtin_node with the same crate name should return the same NodeIndex"
        );

        // Verify it's a Builtin node with name "std"
        match &graph.graph[idx1] {
            GraphNode::Builtin { name } => {
                assert_eq!(name, "std");
            }
            other => panic!("expected Builtin node, got {:?}", other),
        }

        // Different builtin crate should get a different node
        let idx3 = graph.add_builtin_node(f, "core", "core::mem::size_of");
        assert_ne!(
            idx1, idx3,
            "different builtin names should have different nodes"
        );
    }

    #[test]
    fn test_file_info_has_crate_name_field() {
        let mut graph = CodeGraph::new();
        let idx = graph.add_file(PathBuf::from("src/main.rs"), "rust");
        match &graph.graph[idx] {
            GraphNode::File(fi) => {
                assert!(fi.crate_name.is_none(), "crate_name should default to None");
            }
            other => panic!("expected File node, got {:?}", other),
        }
    }

    // -------------------------------------------------------------------------
    // BM25 index tests (Phase 20 Plan 01)
    // -------------------------------------------------------------------------

    #[test]
    fn test_split_identifier_camel_case() {
        let result = super::split_identifier("authHandler");
        assert_eq!(result, "authHandler auth handler");
    }

    #[test]
    fn test_split_identifier_snake_case() {
        let result = super::split_identifier("parse_token");
        assert_eq!(result, "parse_token parse token");
    }

    #[test]
    fn test_split_identifier_already_lowercase() {
        let result = super::split_identifier("main");
        assert_eq!(result, "main main");
    }

    #[test]
    fn test_rebuild_bm25_index_empty_graph() {
        let mut graph = CodeGraph::new();
        graph.rebuild_bm25_index();
        assert!(graph.bm25_index.is_some());
        // Search on empty index returns empty results
        let results = graph.bm25_index.as_ref().unwrap().search("anything", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rebuild_bm25_index_populates_from_symbols() {
        let mut graph = CodeGraph::new();
        let f = graph.add_file(PathBuf::from("src/auth.ts"), "typescript");

        let auth_idx = graph.add_symbol(
            f,
            SymbolInfo {
                name: "authHandler".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "loginService".into(),
                kind: SymbolKind::Class,
                line: 10,
                ..Default::default()
            },
        );
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "parseToken".into(),
                kind: SymbolKind::Function,
                line: 20,
                ..Default::default()
            },
        );

        graph.rebuild_bm25_index();
        assert!(graph.bm25_index.is_some());

        // Search for "auth" — should return authHandler node index
        let results = graph.bm25_index.as_ref().unwrap().search("auth", 10);
        assert!(
            !results.is_empty(),
            "search for 'auth' should return results"
        );
        let found_ids: Vec<u32> = results.iter().map(|r| r.document.id).collect();
        assert!(
            found_ids.contains(&(auth_idx.index() as u32)),
            "authHandler node index should be in BM25 results for 'auth'"
        );
    }
}

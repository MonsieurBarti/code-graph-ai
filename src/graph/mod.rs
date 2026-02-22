pub mod edge;
pub mod node;

use std::collections::HashMap;
use std::path::PathBuf;

use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::Directed;

use edge::EdgeKind;
use node::{FileInfo, GraphNode, SymbolInfo, SymbolKind};

/// The in-memory code graph: a directed petgraph StableGraph with O(1) lookup indexes.
pub struct CodeGraph {
    /// The underlying directed graph, parameterised over node and edge kinds.
    pub graph: StableGraph<GraphNode, EdgeKind, Directed>,
    /// Maps file paths to their node indices for O(1) lookup.
    pub file_index: HashMap<PathBuf, NodeIndex>,
    /// Maps symbol names to all node indices bearing that name (one name may appear in many files).
    pub symbol_index: HashMap<String, Vec<NodeIndex>>,
}

impl CodeGraph {
    /// Create an empty code graph.
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            file_index: HashMap::new(),
            symbol_index: HashMap::new(),
        }
    }

    /// Add a file node to the graph. Returns the new node's index.
    /// If the file has already been added, returns the existing index.
    pub fn add_file(&mut self, path: PathBuf, language: &str) -> NodeIndex {
        if let Some(&existing) = self.file_index.get(&path) {
            return existing;
        }
        let info = FileInfo {
            path: path.clone(),
            language: language.to_owned(),
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
        self.graph.add_edge(child_idx, parent_idx, EdgeKind::ChildOf);
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
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
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
                col: 0,
                is_exported: true,
                is_default: false,
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
        assert_eq!(idx1, idx2, "duplicate add_file should return the same index");
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
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );
        let prop = graph.add_child_symbol(
            iface,
            SymbolInfo {
                name: "name".into(),
                kind: SymbolKind::Property,
                line: 2,
                col: 2,
                is_exported: false,
                is_default: false,
            },
        );
        assert_eq!(graph.symbol_count(), 2, "should count both interface and property");
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
                    col: 0,
                    is_exported: false,
                    is_default: false,
                },
            );
        }
        let breakdown = graph.symbols_by_kind();
        assert_eq!(breakdown.get(&SymbolKind::Function), Some(&2));
        assert_eq!(breakdown.get(&SymbolKind::Class), Some(&1));
    }
}

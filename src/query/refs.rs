use std::collections::HashSet;
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

/// The kind of reference a file or symbol has to the queried symbol.
#[derive(Debug, Clone)]
pub enum RefKind {
    /// A file imports the file that defines the symbol (via a ResolvedImport edge).
    Import,
    /// A symbol calls the queried symbol (via a Calls edge).
    Call,
}

/// A single reference result to a queried symbol.
#[derive(Debug, Clone)]
pub struct RefResult {
    /// Absolute path of the file that contains the reference.
    pub file_path: PathBuf,
    /// Whether the reference is an import or a call.
    pub ref_kind: RefKind,
    /// Caller symbol name (only for `RefKind::Call` references).
    pub symbol_name: Option<String>,
    /// 1-based line of the caller symbol (only for `RefKind::Call` references).
    pub line: Option<usize>,
}

/// Find all files and symbols that reference any of the given symbol node indices.
///
/// Produces two classes of results:
/// - **Import refs**: files that have a `ResolvedImport` edge to the file containing the symbol.
/// - **Call refs**: symbol nodes that have a `Calls` edge to the queried symbol.
///
/// Results are sorted by file path for deterministic output.
pub fn find_refs(
    graph: &CodeGraph,
    _symbol_name: &str,
    symbol_indices: &[NodeIndex],
    project_root: &Path,
) -> Vec<RefResult> {
    let _ = project_root; // used by callers for relativizing; kept for API consistency

    // Step 1: Collect all file NodeIndices that define any of the matched symbols.
    let mut defining_files: HashSet<NodeIndex> = HashSet::new();
    for &sym_idx in symbol_indices {
        if let Some(file_idx) = find_containing_file_idx(graph, sym_idx) {
            defining_files.insert(file_idx);
        }
    }

    let mut results: Vec<RefResult> = Vec::new();
    let mut import_ref_files_seen: HashSet<NodeIndex> = HashSet::new();

    // Step 2: Import references — files with a ResolvedImport edge to any defining file.
    for &file_idx in graph.file_index.values() {
        // Skip the defining files themselves.
        if defining_files.contains(&file_idx) {
            continue;
        }

        let mut found_import = false;
        for edge_ref in graph.graph.edges_directed(file_idx, Direction::Outgoing) {
            if matches!(edge_ref.weight(), EdgeKind::ResolvedImport { .. }) {
                let target = edge_ref.target();
                if defining_files.contains(&target) {
                    found_import = true;
                    break;
                }
            }
        }

        if found_import && !import_ref_files_seen.contains(&file_idx) {
            import_ref_files_seen.insert(file_idx);
            if let GraphNode::File(ref fi) = graph.graph[file_idx] {
                results.push(RefResult {
                    file_path: fi.path.clone(),
                    ref_kind: RefKind::Import,
                    symbol_name: None,
                    line: None,
                });
            }
        }
    }

    // Step 3: Call references — symbols with a Calls edge pointing to the queried symbols.
    for &sym_idx in symbol_indices {
        for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
            if matches!(edge_ref.weight(), EdgeKind::Calls) {
                let caller_idx = edge_ref.source();
                // The caller can be a Symbol node or a File node (for file-level calls).
                let (caller_name, caller_line, file_path) = match &graph.graph[caller_idx] {
                    GraphNode::Symbol(info) => {
                        // Find the file containing the caller symbol.
                        let fp = find_file_path_of_node(graph, caller_idx);
                        (Some(info.name.clone()), Some(info.line), fp)
                    }
                    GraphNode::File(fi) => {
                        // A file-level Calls edge (resolver adds these for unscoped calls).
                        (None, None, Some(fi.path.clone()))
                    }
                    _ => continue,
                };

                if let Some(fp) = file_path {
                    results.push(RefResult {
                        file_path: fp,
                        ref_kind: RefKind::Call,
                        symbol_name: caller_name,
                        line: caller_line,
                    });
                }
            }
        }
    }

    // Sort by file path for deterministic output.
    results.sort_by(|a, b| a.file_path.cmp(&b.file_path));

    results
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Return the NodeIndex of the File node that contains `sym_idx` via a Contains edge.
fn find_containing_file_idx(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<NodeIndex> {
    // Direct Contains edge: File -> Symbol.
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge_ref.weight(), EdgeKind::Contains) {
            let source = edge_ref.source();
            if matches!(graph.graph[source], GraphNode::File(_)) {
                return Some(source);
            }
        }
    }

    // Child symbol: look for ChildOf edge from child to parent, then Contains on parent.
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
        if matches!(edge_ref.weight(), EdgeKind::ChildOf) {
            let parent_idx = edge_ref.target();
            if let Some(file_idx) = find_containing_file_idx(graph, parent_idx) {
                return Some(file_idx);
            }
        }
    }

    None
}

/// Return the file path of a node (Symbol or File) by walking Contains edges.
fn find_file_path_of_node(graph: &CodeGraph, node_idx: NodeIndex) -> Option<PathBuf> {
    match &graph.graph[node_idx] {
        GraphNode::File(fi) => Some(fi.path.clone()),
        GraphNode::Symbol(_) => {
            let file_idx = find_containing_file_idx(graph, node_idx)?;
            if let GraphNode::File(fi) = &graph.graph[file_idx] {
                Some(fi.path.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::{
        CodeGraph,
        node::{SymbolInfo, SymbolKind},
    };

    fn make_graph() -> (CodeGraph, PathBuf) {
        let root = PathBuf::from("/proj");
        (CodeGraph::new(), root)
    }

    /// Build a graph where:
    ///   defining.ts defines symbol `foo`
    ///   importer.ts has a ResolvedImport edge to defining.ts
    ///   unrelated.ts has no edge to defining.ts
    fn graph_with_import_ref() -> (CodeGraph, PathBuf, NodeIndex) {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let defining = graph.add_file(root.join("defining.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            defining,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        let importer = graph.add_file(root.join("importer.ts"), "typescript");
        graph.add_resolved_import(importer, defining, "./defining");

        let _unrelated = graph.add_file(root.join("unrelated.ts"), "typescript");

        (graph, root, foo_sym)
    }

    #[test]
    fn test_import_ref_shows_importer() {
        let (graph, root, foo_sym) = graph_with_import_ref();
        let results = find_refs(&graph, "foo", &[foo_sym], &root);

        let import_refs: Vec<_> = results
            .iter()
            .filter(|r| matches!(r.ref_kind, RefKind::Import))
            .collect();

        assert_eq!(
            import_refs.len(),
            1,
            "exactly one import reference expected"
        );
        assert!(
            import_refs[0].file_path.ends_with("importer.ts"),
            "importer.ts should appear as import ref"
        );
    }

    #[test]
    fn test_unrelated_file_not_in_refs() {
        let (graph, root, foo_sym) = graph_with_import_ref();
        let results = find_refs(&graph, "foo", &[foo_sym], &root);

        let has_unrelated = results
            .iter()
            .any(|r| r.file_path.ends_with("unrelated.ts"));
        assert!(!has_unrelated, "unrelated.ts should NOT appear in refs");
    }

    #[test]
    fn test_call_edge_produces_call_ref() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        // foo in defining.ts
        let defining = graph.add_file(root.join("defining.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            defining,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        // bar in caller.ts calls foo
        let caller_file = graph.add_file(root.join("caller.ts"), "typescript");
        let bar_sym = graph.add_symbol(
            caller_file,
            SymbolInfo {
                name: "bar".into(),
                kind: SymbolKind::Function,
                line: 5,
                col: 0,
                is_exported: false,
                is_default: false,
            },
        );
        graph.add_calls_edge(bar_sym, foo_sym);

        let results = find_refs(&graph, "foo", &[foo_sym], &root);
        let call_refs: Vec<_> = results
            .iter()
            .filter(|r| matches!(r.ref_kind, RefKind::Call))
            .collect();

        assert_eq!(call_refs.len(), 1, "one call reference expected");
        assert_eq!(call_refs[0].symbol_name.as_deref(), Some("bar"));
        assert_eq!(call_refs[0].line, Some(5));
        assert!(call_refs[0].file_path.ends_with("caller.ts"));
    }

    #[test]
    fn test_defining_file_excluded_from_import_refs() {
        let (graph, root, foo_sym) = graph_with_import_ref();
        let results = find_refs(&graph, "foo", &[foo_sym], &root);

        let has_defining = results
            .iter()
            .any(|r| r.file_path.ends_with("defining.ts") && matches!(r.ref_kind, RefKind::Import));
        assert!(
            !has_defining,
            "defining.ts should NOT appear as an import ref of its own symbol"
        );
    }

    #[test]
    fn test_no_refs_returns_empty() {
        let (graph, root) = make_graph();
        let results = find_refs(&graph, "nothing", &[], &root);
        assert!(results.is_empty(), "no symbol indices => no refs");
    }

    #[test]
    fn test_import_ref_deduplication() {
        // Two import specifiers from same file to defining file => only one import ref.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let defining = graph.add_file(root.join("defining.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            defining,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        let importer = graph.add_file(root.join("importer.ts"), "typescript");
        // Two separate ResolvedImport edges (could happen if both named and default imported).
        graph.add_resolved_import(importer, defining, "./defining");
        graph.add_resolved_import(importer, defining, "./defining");

        let results = find_refs(&graph, "foo", &[foo_sym], &root);
        let import_refs: Vec<_> = results
            .iter()
            .filter(|r| matches!(r.ref_kind, RefKind::Import))
            .collect();

        assert_eq!(
            import_refs.len(),
            1,
            "multiple edges to same file => deduplicated to one import ref"
        );
    }
}

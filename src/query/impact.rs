use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

/// A single file in the blast-radius (impact) result set.
#[derive(Debug, Clone)]
pub struct ImpactResult {
    /// Absolute path to the affected file.
    pub file_path: PathBuf,
    /// BFS depth from the defining file(s) of the queried symbol (for --tree view).
    pub depth: usize,
}

/// Compute the blast radius of changing the given symbols.
///
/// Performs a custom BFS on INCOMING `ResolvedImport` edges only (not Calls, Contains, etc.),
/// starting from the file(s) that define the queried symbols.
///
/// Returns all transitively dependent files sorted by depth (ascending) then by path.
pub fn blast_radius(
    graph: &CodeGraph,
    symbol_indices: &[NodeIndex],
    project_root: &Path,
) -> Vec<ImpactResult> {
    let _ = project_root; // kept for API consistency with find_refs

    // Step 1: Collect starting file indices — the file(s) that define the queried symbols.
    let mut starting_files: HashSet<NodeIndex> = HashSet::new();
    for &sym_idx in symbol_indices {
        if let Some(file_idx) = find_containing_file_idx(graph, sym_idx) {
            starting_files.insert(file_idx);
        }
    }

    if starting_files.is_empty() {
        return Vec::new();
    }

    // Step 2: Custom BFS following only incoming ResolvedImport edges (reverse import graph).
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut depths: HashMap<NodeIndex, usize> = HashMap::new();

    // Seed with starting files at depth 0.
    for &start_idx in &starting_files {
        queue.push_back(start_idx);
        visited.insert(start_idx);
        depths.insert(start_idx, 0);
    }

    while let Some(current) = queue.pop_front() {
        let current_depth = depths[&current];

        // Walk INCOMING edges to find files that import this file.
        for edge_ref in graph.graph.edges_directed(current, Direction::Incoming) {
            if matches!(edge_ref.weight(), EdgeKind::ResolvedImport { .. }) {
                let source = edge_ref.source();
                // Only follow File nodes — skip Symbol, ExternalPackage, UnresolvedImport.
                if !visited.contains(&source) && matches!(graph.graph[source], GraphNode::File(_)) {
                    visited.insert(source);
                    depths.insert(source, current_depth + 1);
                    queue.push_back(source);
                }
            }
        }
    }

    // Step 3: Collect results, excluding the starting files themselves.
    let mut results: Vec<ImpactResult> = visited
        .iter()
        .filter(|&&idx| !starting_files.contains(&idx))
        .filter_map(|&idx| {
            if let GraphNode::File(ref fi) = graph.graph[idx] {
                Some(ImpactResult {
                    file_path: fi.path.clone(),
                    depth: depths[&idx],
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by depth ascending, then by file path for deterministic output.
    results.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.file_path.cmp(&b.file_path)));

    results
}

// ---------------------------------------------------------------------------
// Private helper
// ---------------------------------------------------------------------------

/// Return the NodeIndex of the File node that contains `sym_idx` via a Contains or ChildOf edge.
fn find_containing_file_idx(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<NodeIndex> {
    // Direct Contains edge: File -> Symbol (incoming to symbol).
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge_ref.weight(), EdgeKind::Contains) {
            let source = edge_ref.source();
            if matches!(graph.graph[source], GraphNode::File(_)) {
                return Some(source);
            }
        }
    }

    // Child symbol: ChildOf edge from child (outgoing) to parent symbol, then Contains on parent.
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

    /// Build a three-file graph:
    ///   a.ts defines `foo`; b.ts imports a.ts; c.ts imports b.ts (transitive).
    fn three_file_chain() -> (
        CodeGraph,
        PathBuf,
        NodeIndex,
        NodeIndex,
        NodeIndex,
        NodeIndex,
    ) {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        let b_file = graph.add_file(root.join("b.ts"), "typescript");
        graph.add_resolved_import(b_file, a_file, "./a");

        let c_file = graph.add_file(root.join("c.ts"), "typescript");
        graph.add_resolved_import(c_file, b_file, "./b");

        (graph, root, foo_sym, a_file, b_file, c_file)
    }

    #[test]
    fn test_direct_importer_in_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_b = results.iter().any(|r| r.file_path.ends_with("b.ts"));
        assert!(
            has_b,
            "b.ts directly imports a.ts and must appear in blast radius"
        );
    }

    #[test]
    fn test_transitive_importer_in_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_c = results.iter().any(|r| r.file_path.ends_with("c.ts"));
        assert!(
            has_c,
            "c.ts transitively imports a.ts and must appear in blast radius"
        );
    }

    #[test]
    fn test_defining_file_excluded_from_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_a = results.iter().any(|r| r.file_path.ends_with("a.ts"));
        assert!(
            !has_a,
            "a.ts defines foo and should NOT appear in its own blast radius"
        );
    }

    #[test]
    fn test_non_importing_file_excluded() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        // unrelated.ts has no edge to a.ts.
        let _unrelated = graph.add_file(root.join("unrelated.ts"), "typescript");

        let results = blast_radius(&graph, &[foo_sym], &root);
        let has_unrelated = results
            .iter()
            .any(|r| r.file_path.ends_with("unrelated.ts"));
        assert!(
            !has_unrelated,
            "unrelated.ts should not appear in blast radius"
        );
    }

    #[test]
    fn test_depth_tracking() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        // b.ts is at depth 1 (directly imports a.ts), c.ts is at depth 2.
        let b_result = results
            .iter()
            .find(|r| r.file_path.ends_with("b.ts"))
            .unwrap();
        let c_result = results
            .iter()
            .find(|r| r.file_path.ends_with("c.ts"))
            .unwrap();

        assert_eq!(b_result.depth, 1, "b.ts should be at depth 1");
        assert_eq!(c_result.depth, 2, "c.ts should be at depth 2");
    }

    #[test]
    fn test_calls_edges_not_followed_in_blast_radius() {
        // A Calls edge from caller.ts to foo should NOT make caller.ts appear in blast radius.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        // caller.ts has a Calls edge to foo but NOT a ResolvedImport edge to a.ts.
        let caller_file = graph.add_file(root.join("caller.ts"), "typescript");
        graph.add_calls_edge(caller_file, foo_sym);

        let results = blast_radius(&graph, &[foo_sym], &root);
        let has_caller = results.iter().any(|r| r.file_path.ends_with("caller.ts"));
        assert!(
            !has_caller,
            "Calls edge should not be followed in blast radius BFS"
        );
    }
}

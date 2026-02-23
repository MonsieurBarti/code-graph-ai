use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::Directed;
use petgraph::algo::kosaraju_scc;
use petgraph::graph::Graph;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

/// A set of files forming a circular dependency cycle.
#[derive(Debug, Clone)]
pub struct CircularDep {
    /// Files forming the cycle, ordered deterministically by path.
    /// The first file is repeated at the end to close the visual cycle.
    pub files: Vec<PathBuf>,
}

/// Detect circular dependencies in the project's import graph.
///
/// Uses Kosaraju's SCC algorithm on a file-only subgraph containing only
/// `ResolvedImport` edges (not `BarrelReExportAll`, `Calls`, or others).
/// SCCs with more than one node are circular dependency cycles.
///
/// Returns cycles sorted by the first file path in each cycle.
pub fn find_circular(graph: &CodeGraph, project_root: &Path) -> Vec<CircularDep> {
    let _ = project_root; // kept for API consistency

    // Step 1: Build a regular (non-stable) petgraph Graph containing ONLY file nodes
    // and ResolvedImport edges. This is required for kosaraju_scc.
    let mut file_graph: Graph<NodeIndex, (), Directed> = Graph::new();
    // Maps original StableGraph NodeIndex -> new Graph NodeIndex
    let mut orig_to_new: HashMap<NodeIndex, petgraph::graph::NodeIndex> = HashMap::new();
    // Maps new Graph NodeIndex -> original NodeIndex (for path lookup)
    let mut new_to_orig: HashMap<petgraph::graph::NodeIndex, NodeIndex> = HashMap::new();

    // Add a node for each file in the original graph.
    for &orig_idx in graph.file_index.values() {
        let new_idx = file_graph.add_node(orig_idx);
        orig_to_new.insert(orig_idx, new_idx);
        new_to_orig.insert(new_idx, orig_idx);
    }

    // Add only ResolvedImport edges between file nodes.
    for edge_ref in graph.graph.edge_references() {
        if matches!(edge_ref.weight(), EdgeKind::ResolvedImport { .. }) {
            let src_orig = edge_ref.source();
            let dst_orig = edge_ref.target();
            // Only add if both endpoints are file nodes (skip edges to ExternalPackage/Unresolved).
            if let (Some(&src_new), Some(&dst_new)) =
                (orig_to_new.get(&src_orig), orig_to_new.get(&dst_orig))
            {
                file_graph.add_edge(src_new, dst_new, ());
            }
        }
    }

    // Step 2: Run Kosaraju's SCC algorithm.
    let sccs = kosaraju_scc(&file_graph);

    // Step 3: Filter to SCCs with more than one node (actual cycles).
    let mut cycles: Vec<CircularDep> = sccs
        .into_iter()
        .filter(|scc| scc.len() > 1)
        .filter_map(|scc| {
            let mut file_paths: Vec<PathBuf> = scc
                .iter()
                .filter_map(|&new_idx| {
                    let orig_idx = new_to_orig.get(&new_idx)?;
                    if let GraphNode::File(ref fi) = graph.graph[*orig_idx] {
                        Some(fi.path.clone())
                    } else {
                        None
                    }
                })
                .collect();

            if file_paths.is_empty() {
                return None;
            }

            // Sort files within the cycle by path for deterministic output.
            file_paths.sort();

            // Close the visual cycle by appending the first file at the end: a -> b -> c -> a.
            let first = file_paths[0].clone();
            file_paths.push(first);

            Some(CircularDep { files: file_paths })
        })
        .collect();

    // Sort cycles by the first file in each cycle.
    cycles.sort_by(|a, b| a.files[0].cmp(&b.files[0]));

    cycles
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

    #[test]
    fn test_two_file_mutual_cycle_detected() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let b_file = graph.add_file(root.join("b.ts"), "typescript");

        // a imports b, b imports a — mutual cycle.
        graph.add_resolved_import(a_file, b_file, "./b");
        graph.add_resolved_import(b_file, a_file, "./a");

        let cycles = find_circular(&graph, &root);
        assert_eq!(cycles.len(), 1, "one cycle expected");
        // The cycle should contain both a.ts and b.ts (plus one repeated to close it = 3 entries).
        assert_eq!(
            cycles[0].files.len(),
            3,
            "cycle should have 3 entries (2 files + closing)"
        );
        let paths: Vec<_> = cycles[0]
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"a.ts"));
        assert!(paths.contains(&"b.ts"));
        // First and last should be the same (cycle closed).
        assert_eq!(
            cycles[0].files[0],
            cycles[0].files[cycles[0].files.len() - 1]
        );
    }

    #[test]
    fn test_three_file_cycle_detected() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let b_file = graph.add_file(root.join("b.ts"), "typescript");
        let c_file = graph.add_file(root.join("c.ts"), "typescript");

        // a -> b -> c -> a forms a 3-cycle.
        graph.add_resolved_import(a_file, b_file, "./b");
        graph.add_resolved_import(b_file, c_file, "./c");
        graph.add_resolved_import(c_file, a_file, "./a");

        let cycles = find_circular(&graph, &root);
        assert_eq!(cycles.len(), 1, "one 3-cycle expected");
        // 3 unique files + 1 closing = 4 entries.
        assert_eq!(cycles[0].files.len(), 4);
    }

    #[test]
    fn test_no_cycle_in_acyclic_graph() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let b_file = graph.add_file(root.join("b.ts"), "typescript");
        let c_file = graph.add_file(root.join("c.ts"), "typescript");

        // a -> b -> c (no cycle).
        graph.add_resolved_import(a_file, b_file, "./b");
        graph.add_resolved_import(b_file, c_file, "./c");

        let cycles = find_circular(&graph, &root);
        assert!(cycles.is_empty(), "no cycles expected in a DAG");
    }

    #[test]
    fn test_barrel_reexport_edges_excluded_from_cycle_detection() {
        // A BarrelReExportAll edge should NOT count as a cycle, per the plan.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let index_file = graph.add_file(root.join("index.ts"), "typescript");
        let utils_file = graph.add_file(root.join("utils.ts"), "typescript");

        // Barrel: index re-exports all from utils.
        graph.add_barrel_reexport_all(index_file, utils_file);
        // utils also imports from index (would be a cycle only with ResolvedImport).
        graph.add_resolved_import(utils_file, index_file, "./index");

        // Since BarrelReExportAll is excluded, there should be no cycle detected.
        // (utils -> index via ResolvedImport, but index -> utils only via BarrelReExportAll)
        let cycles = find_circular(&graph, &root);
        assert!(
            cycles.is_empty(),
            "BarrelReExportAll edges must not contribute to cycle detection"
        );
    }

    #[test]
    fn test_external_package_edges_excluded() {
        // ResolvedImport to an ExternalPackage should not create a false cycle.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        // Add an external package (e.g. react) — this creates a ResolvedImport edge
        // but ExternalPackage nodes are NOT in orig_to_new (only file nodes are).
        graph.add_external_package(a_file, "react", "react");

        let cycles = find_circular(&graph, &root);
        assert!(
            cycles.is_empty(),
            "external package edges should not create cycles"
        );
    }

    #[test]
    fn test_symbol_with_child_symbol_defining_file() {
        // Sanity: adding symbols doesn't interfere with circular detection.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let b_file = graph.add_file(root.join("b.ts"), "typescript");

        graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "Foo".into(),
                kind: SymbolKind::Class,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        // Mutual cycle.
        graph.add_resolved_import(a_file, b_file, "./b");
        graph.add_resolved_import(b_file, a_file, "./a");

        let cycles = find_circular(&graph, &root);
        assert_eq!(
            cycles.len(),
            1,
            "symbols should not interfere with cycle detection"
        );
    }
}

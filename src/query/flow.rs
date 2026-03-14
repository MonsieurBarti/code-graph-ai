use std::collections::{HashSet, VecDeque};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single call-chain path from entry to target symbol.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FlowPath {
    /// Ordered list of symbol names from entry to target (inclusive).
    pub hops: Vec<String>,
    /// Number of hops (edges) in this path, i.e. `hops.len() - 1`.
    pub depth: usize,
}

/// Result of a flow trace query.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FlowResult {
    /// All discovered paths from entry to target, up to `max_paths`.
    pub paths: Vec<FlowPath>,
    /// When no direct path exists: the closest shared dependency symbol name, if any.
    pub shared_dependency: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Trace call-chain paths from `entry` to `target` symbol using BFS.
///
/// Returns up to `max_paths` distinct paths. Each path visits at most `max_depth` hops.
/// Cycle safety is guaranteed: each path tracks its own visited set, so a symbol cannot
/// appear twice in a single path — but different paths may share prefixes.
///
/// When no paths are found, `shared_dependency` is populated with the closest symbol
/// reachable from both `entry` and `target` (forward BFS intersection), if any.
pub fn trace_flow(
    graph: &CodeGraph,
    entry: &str,
    target: &str,
    max_paths: usize,
    max_depth: usize,
) -> FlowResult {
    // Resolve entry and target symbol names to NodeIndex.
    let entry_indices = match graph.symbol_index.get(entry) {
        Some(v) if !v.is_empty() => v.clone(),
        _ => {
            return FlowResult {
                paths: Vec::new(),
                shared_dependency: None,
            };
        }
    };
    let target_indices: HashSet<NodeIndex> = match graph.symbol_index.get(target) {
        Some(v) if !v.is_empty() => v.iter().cloned().collect(),
        _ => {
            return FlowResult {
                paths: Vec::new(),
                shared_dependency: None,
            };
        }
    };

    // Use the first occurrence of entry and target (by NodeIndex order).
    let entry_idx = entry_indices[0];

    // BFS queue: (current_node, path_so_far, visited_for_this_path)
    // path_so_far includes current_node.
    let mut queue: VecDeque<(NodeIndex, Vec<NodeIndex>, HashSet<NodeIndex>)> = VecDeque::new();
    {
        let mut init_visited = HashSet::new();
        init_visited.insert(entry_idx);
        queue.push_back((entry_idx, vec![entry_idx], init_visited));
    }

    let mut found_paths: Vec<FlowPath> = Vec::new();

    while let Some((current, path, visited)) = queue.pop_front() {
        // Check if we reached any target node.
        if target_indices.contains(&current) && path.len() > 1 {
            // Convert NodeIndex path to symbol names.
            let hops: Vec<String> = path
                .iter()
                .map(|&idx| node_symbol_name(graph, idx))
                .collect();
            let depth = hops.len() - 1;
            found_paths.push(FlowPath { hops, depth });

            if found_paths.len() >= max_paths {
                break;
            }
            // Don't expand from target — we found a complete path.
            continue;
        }

        // Depth cap: path already has max_depth+1 nodes (entry + max_depth hops).
        if path.len() > max_depth {
            continue;
        }

        // Expand via Calls and ResolvedImport outgoing edges.
        for edge_ref in graph.graph.edges_directed(current, Direction::Outgoing) {
            if !matches!(
                edge_ref.weight(),
                EdgeKind::Calls | EdgeKind::ResolvedImport { .. }
            ) {
                continue;
            }

            let neighbor = edge_ref.target();

            // Cycle safety: skip if neighbor is already in this path's visited set.
            if visited.contains(&neighbor) {
                continue;
            }

            // Only follow Symbol nodes (or File nodes for ResolvedImport).
            match &graph.graph[neighbor] {
                GraphNode::Symbol(_) | GraphNode::File(_) => {}
                _ => continue,
            }

            let mut new_path = path.clone();
            new_path.push(neighbor);
            let mut new_visited = visited.clone();
            new_visited.insert(neighbor);

            queue.push_back((neighbor, new_path, new_visited));
        }
    }

    // If no paths found, compute shared dependency via forward BFS intersection.
    let shared_dependency = if found_paths.is_empty() {
        find_shared_dependency(graph, entry_idx, &target_indices, max_depth)
    } else {
        None
    };

    FlowResult {
        paths: found_paths,
        shared_dependency,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Compute the closest shared dependency reachable from both `entry_idx` and any target.
///
/// Uses single-source BFS from both entry and target, then finds the intersection.
/// Returns the name of the node closest (shallowest) in the intersection, if any.
fn find_shared_dependency(
    graph: &CodeGraph,
    entry_idx: NodeIndex,
    target_indices: &HashSet<NodeIndex>,
    max_depth: usize,
) -> Option<String> {
    let from_entry = reachable_set(graph, entry_idx, max_depth);

    let mut shared_name: Option<String> = None;
    let mut best_depth = usize::MAX;

    for &target_idx in target_indices {
        let from_target = reachable_set(graph, target_idx, max_depth);

        for (node, depth) in &from_entry {
            if from_target.contains_key(node) && *depth < best_depth {
                best_depth = *depth;
                shared_name = Some(node_symbol_name(graph, *node));
            }
        }
    }

    shared_name
}

/// BFS from `start`, following Calls + ResolvedImport outgoing edges.
/// Returns a map of NodeIndex -> depth from start, up to max_depth.
fn reachable_set(
    graph: &CodeGraph,
    start: NodeIndex,
    max_depth: usize,
) -> std::collections::HashMap<NodeIndex, usize> {
    let mut visited: std::collections::HashMap<NodeIndex, usize> = std::collections::HashMap::new();
    let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
    queue.push_back((start, 0));
    visited.insert(start, 0);

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for edge_ref in graph.graph.edges_directed(current, Direction::Outgoing) {
            if !matches!(
                edge_ref.weight(),
                EdgeKind::Calls | EdgeKind::ResolvedImport { .. }
            ) {
                continue;
            }
            let neighbor = edge_ref.target();
            if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                e.insert(depth + 1);
                queue.push_back((neighbor, depth + 1));
            }
        }
    }

    visited
}

/// Get the symbol name from a node index. Falls back to "?" for non-symbol nodes.
fn node_symbol_name(graph: &CodeGraph, idx: NodeIndex) -> String {
    match &graph.graph[idx] {
        GraphNode::Symbol(info) => info.name.clone(),
        GraphNode::File(fi) => fi
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string(),
        _ => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::node::{SymbolInfo, SymbolKind};

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }

    /// Build a linear chain: A -> B -> C via Calls edges.
    fn graph_linear_chain() -> (crate::graph::CodeGraph, PathBuf) {
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f = g.add_file(r.join("src/main.rs"), "rust");
        let a = g.add_symbol(
            f,
            SymbolInfo {
                name: "A".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        let b = g.add_symbol(
            f,
            SymbolInfo {
                name: "B".into(),
                kind: SymbolKind::Function,
                line: 5,
                ..Default::default()
            },
        );
        let c = g.add_symbol(
            f,
            SymbolInfo {
                name: "C".into(),
                kind: SymbolKind::Function,
                line: 10,
                ..Default::default()
            },
        );

        g.add_calls_edge(a, b);
        g.add_calls_edge(b, c);

        (g, r)
    }

    #[test]
    fn test_trace_flow_basic() {
        let (graph, r) = graph_linear_chain();
        let _ = r;
        let result = trace_flow(&graph, "A", "C", 3, 20);

        assert_eq!(result.paths.len(), 1, "expected exactly 1 path A->B->C");
        assert_eq!(result.paths[0].hops, vec!["A", "B", "C"]);
        assert_eq!(result.paths[0].depth, 2);
    }

    #[test]
    fn test_trace_flow_multiple_paths() {
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f = g.add_file(r.join("src/main.rs"), "rust");
        let a = g.add_symbol(
            f,
            SymbolInfo {
                name: "A".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        let b = g.add_symbol(
            f,
            SymbolInfo {
                name: "B".into(),
                kind: SymbolKind::Function,
                line: 5,
                ..Default::default()
            },
        );
        let d = g.add_symbol(
            f,
            SymbolInfo {
                name: "D".into(),
                kind: SymbolKind::Function,
                line: 8,
                ..Default::default()
            },
        );
        let c = g.add_symbol(
            f,
            SymbolInfo {
                name: "C".into(),
                kind: SymbolKind::Function,
                line: 10,
                ..Default::default()
            },
        );

        // Two distinct paths: A->B->C and A->D->C
        g.add_calls_edge(a, b);
        g.add_calls_edge(b, c);
        g.add_calls_edge(a, d);
        g.add_calls_edge(d, c);

        let result = trace_flow(&g, "A", "C", 3, 20);
        assert_eq!(result.paths.len(), 2, "expected 2 paths (A->B->C, A->D->C)");
    }

    #[test]
    fn test_trace_flow_max_paths() {
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f = g.add_file(r.join("src/main.rs"), "rust");
        let a = g.add_symbol(
            f,
            SymbolInfo {
                name: "A".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        let c = g.add_symbol(
            f,
            SymbolInfo {
                name: "C".into(),
                kind: SymbolKind::Function,
                line: 10,
                ..Default::default()
            },
        );

        // 4 intermediaries → 4 paths A->Bx->C
        for i in 0..4 {
            let b = g.add_symbol(
                f,
                SymbolInfo {
                    name: format!("B{i}"),
                    kind: SymbolKind::Function,
                    line: 20 + i,
                    ..Default::default()
                },
            );
            g.add_calls_edge(a, b);
            g.add_calls_edge(b, c);
        }

        let result = trace_flow(&g, "A", "C", 3, 20);
        assert_eq!(
            result.paths.len(),
            3,
            "max_paths=3 should return at most 3 paths"
        );
    }

    #[test]
    fn test_trace_flow_cycle_safety() {
        // Graph: A->B->C->A (cycle). trace_flow("A","C") must terminate.
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f = g.add_file(r.join("src/main.rs"), "rust");
        let a = g.add_symbol(
            f,
            SymbolInfo {
                name: "A".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        let b = g.add_symbol(
            f,
            SymbolInfo {
                name: "B".into(),
                kind: SymbolKind::Function,
                line: 5,
                ..Default::default()
            },
        );
        let c = g.add_symbol(
            f,
            SymbolInfo {
                name: "C".into(),
                kind: SymbolKind::Function,
                line: 10,
                ..Default::default()
            },
        );

        g.add_calls_edge(a, b);
        g.add_calls_edge(b, c);
        g.add_calls_edge(c, a); // cycle back

        // Must not hang — cycle safety via per-path visited set.
        let result = trace_flow(&g, "A", "C", 3, 20);
        // There should be exactly 1 path A->B->C (the cycle doesn't produce more).
        assert_eq!(
            result.paths.len(),
            1,
            "cycle graph: should find path A->B->C"
        );
        assert_eq!(result.paths[0].hops, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_trace_flow_depth_cap() {
        // Chain of 25 hops, max_depth=20 → no path should be returned.
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f = g.add_file(r.join("src/main.rs"), "rust");
        let mut prev = g.add_symbol(
            f,
            SymbolInfo {
                name: "S0".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );

        for i in 1..=25 {
            let next = g.add_symbol(
                f,
                SymbolInfo {
                    name: format!("S{i}"),
                    kind: SymbolKind::Function,
                    line: i + 1,
                    ..Default::default()
                },
            );
            g.add_calls_edge(prev, next);
            prev = next;
        }

        // max_depth=20 but target is at depth 25 — should return empty.
        let result = trace_flow(&g, "S0", "S25", 3, 20);
        assert!(
            result.paths.is_empty(),
            "depth-25 chain with max_depth=20 should return no paths"
        );
    }

    #[test]
    fn test_trace_flow_no_path() {
        // Disconnected symbols — no path, shared_dependency may or may not be found.
        let (graph, _) = graph_linear_chain();
        // D doesn't exist in the graph — no path from A to D.
        let result = trace_flow(&graph, "A", "NonExistent", 3, 20);
        assert!(
            result.paths.is_empty(),
            "no path to unknown symbol expected"
        );
    }

    #[test]
    fn test_trace_flow_follows_calls_and_imports() {
        // A --Calls--> B --ResolvedImport--> C (file)
        // (For file targets, the path includes file node names.)
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let fa = g.add_file(r.join("src/a.rs"), "rust");
        let fb = g.add_file(r.join("src/b.rs"), "rust");

        let a = g.add_symbol(
            fa,
            SymbolInfo {
                name: "funcA".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        let b = g.add_symbol(
            fa,
            SymbolInfo {
                name: "funcB".into(),
                kind: SymbolKind::Function,
                line: 5,
                ..Default::default()
            },
        );

        g.add_calls_edge(a, b);
        g.add_resolved_import(fa, fb, "./b");

        // At minimum, the Calls path funcA -> funcB should be traced.
        let result = trace_flow(&g, "funcA", "funcB", 3, 20);
        assert!(
            !result.paths.is_empty(),
            "Calls edge from funcA to funcB should be traceable"
        );
        assert_eq!(result.paths[0].hops, vec!["funcA", "funcB"]);
    }
}

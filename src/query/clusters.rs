use std::collections::BTreeMap;
use std::path::Path;

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A community cluster of symbols detected via label propagation.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterResult {
    /// The cluster label (typically the directory prefix of member symbols).
    pub label: String,
    /// Total number of member symbols in this cluster.
    pub member_count: usize,
    /// Top symbols by incoming edge count (reference count), up to 5.
    pub top_symbols: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Detect functional clusters (communities) in the code graph using label propagation.
///
/// Algorithm:
/// 1. Collect all symbol NodeIndex entries, applying scope filter if provided.
/// 2. Initialize each symbol's label from its containing file's directory prefix.
/// 3. Iterate label propagation: each node takes the majority label of its neighbours
///    (Calls + ChildOf edges), with lexicographic tie-breaking.
/// 4. Stop early on convergence or after `max_iterations`.
/// 5. Group by final label, pick top symbols by incoming edge count.
///
/// Returns a Vec<ClusterResult> sorted by label for deterministic output.
pub fn find_clusters(
    graph: &CodeGraph,
    root: &Path,
    scope: Option<&Path>,
    max_iterations: usize,
) -> Vec<ClusterResult> {
    // Step 1: apply scope filter (same pattern as dead_code.rs)
    let abs_scope: Option<std::path::PathBuf> = scope.map(|s| {
        if s.is_absolute() {
            s.to_path_buf()
        } else {
            root.join(s)
        }
    });
    let in_scope = |path: &Path| -> bool {
        match &abs_scope {
            None => true,
            Some(scope_path) => path.starts_with(scope_path),
        }
    };

    // Step 2: collect symbol nodes with their initial labels (directory prefix of file path)
    // Use BTreeMap for deterministic iteration order by NodeIndex.
    let mut labels: BTreeMap<NodeIndex, String> = BTreeMap::new();

    for file_idx in graph.file_index.values() {
        // Get the file path.
        let file_path = match &graph.graph[*file_idx] {
            GraphNode::File(fi) => fi.path.clone(),
            _ => continue,
        };

        // Apply scope filter.
        if !in_scope(&file_path) {
            continue;
        }

        // Compute initial label from the file's parent directory relative to root.
        let label = directory_label(&file_path, root);

        // Collect all symbol nodes contained in this file.
        for edge_ref in graph.graph.edges_directed(*file_idx, Direction::Outgoing) {
            if matches!(edge_ref.weight(), EdgeKind::Contains) {
                let sym_idx = edge_ref.target();
                if matches!(graph.graph[sym_idx], GraphNode::Symbol(_)) {
                    labels.insert(sym_idx, label.clone());
                }
            }
        }
    }

    if labels.is_empty() {
        return Vec::new();
    }

    // Step 3: label propagation iterations
    for _ in 0..max_iterations {
        let mut changed = false;

        // Snapshot current labels so all updates use the same generation.
        let snapshot: BTreeMap<NodeIndex, String> = labels.clone();

        for (&node, current_label) in &mut labels {
            // Collect neighbour labels from Calls + ChildOf edges (both directions).
            let mut neighbor_labels: BTreeMap<String, usize> = BTreeMap::new();

            // Outgoing edges: Calls (this symbol calls others), ChildOf (this is a child)
            for edge_ref in graph.graph.edges_directed(node, Direction::Outgoing) {
                let neighbor = edge_ref.target();
                if matches!(edge_ref.weight(), EdgeKind::Calls | EdgeKind::ChildOf)
                    && let Some(nlabel) = snapshot.get(&neighbor)
                {
                    *neighbor_labels.entry(nlabel.clone()).or_insert(0) += 1;
                }
            }

            // Incoming edges: Calls (others call this), ChildOf (this has children)
            for edge_ref in graph.graph.edges_directed(node, Direction::Incoming) {
                let neighbor = edge_ref.source();
                if matches!(edge_ref.weight(), EdgeKind::Calls | EdgeKind::ChildOf)
                    && let Some(nlabel) = snapshot.get(&neighbor)
                {
                    *neighbor_labels.entry(nlabel.clone()).or_insert(0) += 1;
                }
            }

            // No neighbors? Keep current label (directory initialization is dominant).
            if neighbor_labels.is_empty() {
                continue;
            }

            // Majority label with lexicographic tie-breaking (BTreeMap already sorted).
            let max_count = *neighbor_labels.values().max().unwrap_or(&0);
            let majority = neighbor_labels
                .iter()
                .filter(|(_, count)| **count == max_count)
                .map(|(lbl, _)| lbl.clone())
                .next() // BTreeMap is sorted — first is lexicographically smallest
                .unwrap_or_else(|| current_label.clone());

            if majority != *current_label {
                *current_label = majority;
                changed = true;
            }
        }

        // Convergence: stop early if no label changed in this iteration.
        if !changed {
            break;
        }
    }

    // Step 5: group by final label, compute top symbols
    let mut groups: BTreeMap<String, Vec<NodeIndex>> = BTreeMap::new();
    for (node_idx, label) in &labels {
        groups.entry(label.clone()).or_default().push(*node_idx);
    }

    // Build ClusterResult for each group, sorted by label (BTreeMap guarantees this).
    groups
        .into_iter()
        .map(|(label, members)| {
            let member_count = members.len();

            // Rank by incoming edge count (reference count) — how many edges point TO this symbol.
            let mut scored: Vec<(NodeIndex, usize)> = members
                .iter()
                .map(|&idx| {
                    let incoming = graph.graph.edges_directed(idx, Direction::Incoming).count();
                    (idx, incoming)
                })
                .collect();

            // Sort descending by incoming count, then by symbol name for determinism.
            scored.sort_by(|a, b| {
                b.1.cmp(&a.1).then_with(|| {
                    let name_a = symbol_name(graph, a.0);
                    let name_b = symbol_name(graph, b.0);
                    name_a.cmp(&name_b)
                })
            });

            let top_symbols: Vec<String> = scored
                .iter()
                .take(5)
                .map(|&(idx, _)| symbol_name(graph, idx))
                .collect();

            ClusterResult {
                label,
                member_count,
                top_symbols,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Compute the directory-prefix label for a file path relative to the project root.
///
/// Examples:
///   `/proj/src/auth/auth.rs` with root `/proj` → `"auth"`
///   `/proj/src/api/routes.rs` with root `/proj` → `"api"`
///   `/proj/src/main.rs` with root `/proj` → `"src"`
///   `/proj/main.rs` with root `/proj` → `"core"`
fn directory_label(file_path: &Path, root: &Path) -> String {
    // Relative path components.
    let rel = file_path.strip_prefix(root).unwrap_or(file_path);
    let components: Vec<_> = rel.components().collect();

    // If the file is directly under root, use parent dir name (or "core").
    if components.len() <= 1 {
        return "core".to_string();
    }

    // Skip components that are pure "src" or "lib" wrappers to get the meaningful label.
    // components[0] is the first directory component.
    let mut start = 0;
    for (i, comp) in components.iter().enumerate() {
        let s = comp.as_os_str().to_str().unwrap_or("");
        if s == "src" || s == "lib" {
            start = i + 1;
        } else {
            break;
        }
    }

    // The first non-src/lib component is the label.
    if start < components.len() - 1 {
        // There's a meaningful directory component after src/lib.
        components[start]
            .as_os_str()
            .to_str()
            .unwrap_or("core")
            .to_string()
    } else {
        // The file is directly inside src/ (e.g. src/main.rs).
        "core".to_string()
    }
}

/// Get the symbol name from a node index.
fn symbol_name(graph: &CodeGraph, idx: NodeIndex) -> String {
    match &graph.graph[idx] {
        GraphNode::Symbol(info) => info.name.clone(),
        _ => String::new(),
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

    // Build a graph with symbols in src/auth/ and src/api/ directories.
    fn graph_with_auth_and_api() -> crate::graph::CodeGraph {
        let root = root();
        let mut g = crate::graph::CodeGraph::new();

        let auth_file = g.add_file(root.join("src/auth/auth.rs"), "rust");
        g.add_symbol(
            auth_file,
            SymbolInfo {
                name: "authenticate".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        g.add_symbol(
            auth_file,
            SymbolInfo {
                name: "authorize".into(),
                kind: SymbolKind::Function,
                line: 10,
                ..Default::default()
            },
        );

        let api_file = g.add_file(root.join("src/api/routes.rs"), "rust");
        g.add_symbol(
            api_file,
            SymbolInfo {
                name: "get_users".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );

        g
    }

    #[test]
    fn test_find_clusters_basic() {
        let graph = graph_with_auth_and_api();
        let r = root();
        let clusters = find_clusters(&graph, &r, None, 10);

        // Should produce clusters labeled "auth" and "api".
        assert!(!clusters.is_empty(), "expected at least one cluster");
        let labels: Vec<&str> = clusters.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"auth"),
            "expected 'auth' cluster, got: {labels:?}"
        );
        assert!(
            labels.contains(&"api"),
            "expected 'api' cluster, got: {labels:?}"
        );

        let auth_cluster = clusters.iter().find(|c| c.label == "auth").unwrap();
        assert_eq!(auth_cluster.member_count, 2, "auth should have 2 members");

        let api_cluster = clusters.iter().find(|c| c.label == "api").unwrap();
        assert_eq!(api_cluster.member_count, 1, "api should have 1 member");
    }

    #[test]
    fn test_find_clusters_determinism() {
        let graph = graph_with_auth_and_api();
        let r = root();
        let c1 = find_clusters(&graph, &r, None, 10);
        let c2 = find_clusters(&graph, &r, None, 10);
        assert_eq!(
            c1, c2,
            "find_clusters must be deterministic — two calls differ"
        );
    }

    #[test]
    fn test_find_clusters_with_scope() {
        let graph = graph_with_auth_and_api();
        let r = root();
        let scope = PathBuf::from("src/auth");
        let clusters = find_clusters(&graph, &r, Some(&scope), 10);

        // Only auth symbols are in scope.
        let labels: Vec<&str> = clusters.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"auth"),
            "expected 'auth' cluster in scoped result"
        );
        assert!(
            !labels.contains(&"api"),
            "api cluster should be excluded by scope filter"
        );
    }

    #[test]
    fn test_find_clusters_empty_graph() {
        let g = crate::graph::CodeGraph::new();
        let r = root();
        let clusters = find_clusters(&g, &r, None, 10);
        assert!(clusters.is_empty(), "empty graph should produce empty Vec");
    }

    #[test]
    fn test_find_clusters_isolated_nodes() {
        // Symbols with no Calls/ChildOf edges still form clusters by directory prefix.
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let f1 = g.add_file(r.join("src/utils/helper.rs"), "rust");
        g.add_symbol(
            f1,
            SymbolInfo {
                name: "helper_fn".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );

        let clusters = find_clusters(&g, &r, None, 10);
        assert_eq!(clusters.len(), 1, "one directory => one cluster");
        assert_eq!(clusters[0].label, "utils");
        assert!(clusters[0].top_symbols.contains(&"helper_fn".to_string()));
    }
}

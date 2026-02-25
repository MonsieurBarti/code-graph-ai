pub mod dot;
pub mod mermaid;
pub mod model;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::graph::CodeGraph;
use crate::graph::node::GraphNode;
use crate::resolver::cargo_workspace::discover_rust_workspace_members;
use crate::resolver::rust_mod_tree::build_mod_tree;

use model::{ExportFormat, ExportParams, ExportResult, Granularity};

/// Export the code graph to DOT or Mermaid format.
///
/// Steps:
/// 1. Build a module path map (file → Rust module path) for Rust projects.
/// 2. Apply filters: exclusions, --root path prefix, --symbol neighborhood BFS.
/// 3. Count visible nodes/edges for the chosen granularity.
/// 4. Check scale guards and emit warnings if thresholds are exceeded.
/// 5. Dispatch to the appropriate renderer.
/// 6. Return ExportResult with content, counts, and warnings.
pub fn export_graph(graph: &CodeGraph, params: &ExportParams) -> anyhow::Result<ExportResult> {
    // Step 1: Build module path map from Rust workspace members.
    let module_path_map = build_module_path_map(graph, &params.project_root);

    // Step 2: Build the set of visible nodes (applying all filters).
    let visible_nodes = build_visible_nodes(graph, params)?;

    // Step 3: Count nodes and edges at the chosen granularity.
    let (node_count, edge_count) = count_nodes_edges(graph, params, &visible_nodes);

    // Step 4: Scale guards — produce warnings (already eprintln'd here, also in result.warnings).
    let mut warnings: Vec<String> = Vec::new();

    if params.format == ExportFormat::Mermaid && edge_count > 500 {
        let msg = format!(
            "Large graph: {} edges may render poorly in Mermaid. \
             Consider --granularity file or --format dot.",
            edge_count
        );
        eprintln!("Warning: {}", msg);
        warnings.push(msg);
    }

    if params.granularity == Granularity::Symbol && node_count > 200 {
        let msg = format!(
            "Large symbol graph: {} nodes. Consider --granularity file or --granularity package \
             for better readability.",
            node_count
        );
        eprintln!("Warning: {}", msg);
        warnings.push(msg);
    }

    // Step 5: Dispatch to renderer.
    let content = match params.format {
        ExportFormat::Dot => {
            dot::render_dot(graph, params, &module_path_map, &visible_nodes)
        }
        ExportFormat::Mermaid => {
            mermaid::render_mermaid(graph, params, &module_path_map, &visible_nodes)
        }
    };

    Ok(ExportResult {
        content,
        node_count,
        edge_count,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a map from file path → Rust module path string by walking workspace crate roots.
///
/// Used to annotate Rust file/symbol nodes with their canonical module path (e.g. `crate::parser`).
fn build_module_path_map(graph: &CodeGraph, project_root: &PathBuf) -> HashMap<PathBuf, String> {
    let mut map: HashMap<PathBuf, String> = HashMap::new();

    let workspace_members = discover_rust_workspace_members(project_root);
    if workspace_members.is_empty() {
        return map;
    }

    for (crate_name, crate_root) in &workspace_members {
        let tree = build_mod_tree(crate_name, crate_root);
        // reverse_map: PathBuf (file) → String (module path).
        for (file_path, mod_path) in &tree.reverse_map {
            map.insert(file_path.clone(), mod_path.clone());
        }
        // Also fill from mod_map in case reverse_map missed any entries.
        for (_mod_path, file_path) in &tree.mod_map {
            map.entry(file_path.clone()).or_insert_with(|| _mod_path.clone());
        }
    }

    // Suppress unused graph variable warning (graph is passed for potential future use).
    let _ = graph;
    map
}

/// Determine which nodes are visible given the current filter params.
///
/// Order of operations (per research recommendation):
/// 1. Build excluded set from --exclude glob patterns.
/// 2. Apply --root path prefix filter.
/// 3. Apply --symbol BFS neighborhood filter.
///
/// All filters are applied to file nodes; symbol/package granularity inherits
/// visibility from their parent file nodes.
fn build_visible_nodes(
    graph: &CodeGraph,
    params: &ExportParams,
) -> anyhow::Result<HashSet<NodeIndex>> {
    // Build the excluded file paths set first.
    let excluded_files = build_excluded_files(graph, params)?;

    // Start with all file node indices.
    let all_file_nodes: HashSet<NodeIndex> = graph
        .graph
        .node_indices()
        .filter(|idx| matches!(graph.graph[*idx], GraphNode::File(_)))
        .collect();

    // Apply --root filter.
    let after_root: HashSet<NodeIndex> = all_file_nodes
        .into_iter()
        .filter(|idx| {
            if let GraphNode::File(ref fi) = graph.graph[*idx] {
                // If root_filter is set, only keep files under that prefix.
                if let Some(ref root) = params.root_filter {
                    return fi.path.starts_with(root)
                        || fi.path.starts_with(params.project_root.join(root));
                }
            }
            true
        })
        .collect();

    // Apply --exclude: remove excluded files.
    let after_exclude: HashSet<NodeIndex> = after_root
        .into_iter()
        .filter(|idx| !excluded_files.contains(idx))
        .collect();

    // Apply --symbol BFS neighborhood filter.
    let visible_files: HashSet<NodeIndex> = if let Some(ref sym_name) = params.symbol_filter {
        apply_symbol_bfs_filter(graph, params, sym_name, &after_exclude)
    } else {
        after_exclude
    };

    // Expand to include all symbol nodes that belong to visible files.
    let mut visible: HashSet<NodeIndex> = visible_files.clone();

    // Add symbol nodes contained in visible files (for symbol granularity).
    for file_idx in &visible_files {
        for edge in graph.graph.edges_directed(*file_idx, petgraph::Direction::Outgoing) {
            if let crate::graph::edge::EdgeKind::Contains = edge.weight() {
                visible.insert(edge.target());
            }
        }
    }

    Ok(visible)
}

/// Build a set of NodeIndices for files that match any --exclude glob pattern.
fn build_excluded_files(
    graph: &CodeGraph,
    params: &ExportParams,
) -> anyhow::Result<HashSet<NodeIndex>> {
    if params.exclude_patterns.is_empty() {
        return Ok(HashSet::new());
    }

    // Compile glob patterns.
    let patterns: Vec<glob::Pattern> = params
        .exclude_patterns
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();

    let mut excluded = HashSet::new();
    for idx in graph.graph.node_indices() {
        if let GraphNode::File(ref fi) = graph.graph[idx] {
            let rel = fi
                .path
                .strip_prefix(&params.project_root)
                .unwrap_or(&fi.path);
            let rel_str = rel.to_string_lossy();
            if patterns.iter().any(|p| p.matches(&rel_str)) {
                excluded.insert(idx);
            }
        }
    }

    Ok(excluded)
}

/// Apply BFS from a named symbol outward to `params.depth` hops.
///
/// Returns the set of file NodeIndices that are within the BFS neighborhood.
fn apply_symbol_bfs_filter(
    graph: &CodeGraph,
    params: &ExportParams,
    sym_name: &str,
    candidate_files: &HashSet<NodeIndex>,
) -> HashSet<NodeIndex> {
    // Find matching symbol nodes.
    let start_symbols: Vec<NodeIndex> = graph
        .symbol_index
        .get(sym_name)
        .cloned()
        .unwrap_or_default();

    if start_symbols.is_empty() {
        return candidate_files.clone();
    }

    // BFS outward from symbol nodes, collecting file nodes along the way.
    let mut visited_symbols: HashSet<NodeIndex> = HashSet::new();
    let mut current_frontier: Vec<NodeIndex> = start_symbols;
    let mut neighborhood_files: HashSet<NodeIndex> = HashSet::new();

    for _ in 0..=params.depth {
        let mut next_frontier: Vec<NodeIndex> = Vec::new();
        for sym_idx in &current_frontier {
            if visited_symbols.contains(sym_idx) {
                continue;
            }
            visited_symbols.insert(*sym_idx);

            // Add the file that contains this symbol.
            for edge in graph.graph.edges_directed(*sym_idx, petgraph::Direction::Incoming) {
                if let crate::graph::edge::EdgeKind::Contains = edge.weight() {
                    if candidate_files.contains(&edge.source()) {
                        neighborhood_files.insert(edge.source());
                    }
                }
            }

            // Traverse outgoing dependency edges.
            for edge in graph.graph.edges(*sym_idx) {
                if matches!(
                    edge.weight(),
                    crate::graph::edge::EdgeKind::ResolvedImport { .. }
                        | crate::graph::edge::EdgeKind::Calls
                        | crate::graph::edge::EdgeKind::Extends
                        | crate::graph::edge::EdgeKind::Implements
                        | crate::graph::edge::EdgeKind::RustImport { .. }
                ) {
                    let neighbor = edge.target();
                    if !visited_symbols.contains(&neighbor) {
                        next_frontier.push(neighbor);
                    }
                }
            }
        }
        current_frontier = next_frontier;
    }

    // If we found no neighborhood files, fall back to all candidates.
    if neighborhood_files.is_empty() {
        candidate_files.clone()
    } else {
        neighborhood_files
    }
}

/// Count the number of nodes and edges at the chosen granularity level.
fn count_nodes_edges(
    graph: &CodeGraph,
    params: &ExportParams,
    visible_nodes: &HashSet<NodeIndex>,
) -> (usize, usize) {
    match params.granularity {
        Granularity::Symbol => {
            let node_count = visible_nodes
                .iter()
                .filter(|idx| matches!(graph.graph[**idx], GraphNode::Symbol(_)))
                .count();

            let edge_count = graph
                .graph
                .edge_references()
                .filter(|e| {
                    let src = e.source();
                    let tgt = e.target();
                    src != tgt
                        && visible_nodes.contains(&src)
                        && visible_nodes.contains(&tgt)
                        && matches!(graph.graph[src], GraphNode::Symbol(_))
                        && matches!(graph.graph[tgt], GraphNode::Symbol(_))
                        && is_dependency_edge_for_count(e.weight())
                })
                .count();

            (node_count, edge_count)
        }

        Granularity::File => {
            let node_count = visible_nodes
                .iter()
                .filter(|idx| matches!(graph.graph[**idx], GraphNode::File(_)))
                .count();

            // Count unique file->file edge pairs.
            let mut file_edge_pairs: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();
            for edge in graph.graph.edge_references() {
                let src = edge.source();
                let tgt = edge.target();
                if src == tgt {
                    continue;
                }
                if !visible_nodes.contains(&src) || !visible_nodes.contains(&tgt) {
                    continue;
                }
                if !matches!(graph.graph[src], GraphNode::File(_)) {
                    continue;
                }
                if !matches!(graph.graph[tgt], GraphNode::File(_)) {
                    continue;
                }
                if !is_dependency_edge_for_count(edge.weight()) {
                    continue;
                }
                file_edge_pairs.insert((src, tgt));
            }

            (node_count, file_edge_pairs.len())
        }

        Granularity::Package => {
            // Build package map and count unique packages and inter-package edges.
            let package_map = dot::build_package_map(graph, params, visible_nodes);

            let node_count = package_map.values().collect::<HashSet<_>>().len();

            let mut inter_pkg_pairs: HashSet<(String, String)> = HashSet::new();
            for edge in graph.graph.edge_references() {
                let src = edge.source();
                let tgt = edge.target();
                if src == tgt {
                    continue;
                }
                if !visible_nodes.contains(&src) || !visible_nodes.contains(&tgt) {
                    continue;
                }
                if !matches!(graph.graph[src], GraphNode::File(_)) {
                    continue;
                }
                if !matches!(graph.graph[tgt], GraphNode::File(_)) {
                    continue;
                }
                if !is_dependency_edge_for_count(edge.weight()) {
                    continue;
                }
                let src_pkg = match package_map.get(&src) {
                    Some(p) => p.clone(),
                    None => continue,
                };
                let tgt_pkg = match package_map.get(&tgt) {
                    Some(p) => p.clone(),
                    None => continue,
                };
                if src_pkg != tgt_pkg {
                    inter_pkg_pairs.insert((src_pkg, tgt_pkg));
                }
            }

            (node_count, inter_pkg_pairs.len())
        }
    }
}

/// Check whether an EdgeKind counts as a dependency edge for node/edge counting.
fn is_dependency_edge_for_count(kind: &crate::graph::edge::EdgeKind) -> bool {
    matches!(
        kind,
        crate::graph::edge::EdgeKind::ResolvedImport { .. }
            | crate::graph::edge::EdgeKind::Calls
            | crate::graph::edge::EdgeKind::Extends
            | crate::graph::edge::EdgeKind::Implements
            | crate::graph::edge::EdgeKind::BarrelReExportAll
            | crate::graph::edge::EdgeKind::ReExport { .. }
            | crate::graph::edge::EdgeKind::RustImport { .. }
    )
}

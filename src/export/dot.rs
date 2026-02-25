use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::export::model::{ExportParams, Granularity};
use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::{GraphNode, SymbolKind};

/// Sanitize a string for use as a DOT node ID or subgraph name.
///
/// Replaces non-alphanumeric characters with `_`. Prepends `n` if the result
/// starts with a digit (DOT IDs must not start with a digit).
pub fn sanitize_dot_id(s: &str) -> String {
    let mut result: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, 'n');
    }
    if result.is_empty() {
        result = "node".to_string();
    }
    result
}

/// Get the DOT fillcolor for a symbol kind.
fn symbol_fillcolor(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function | SymbolKind::ImplMethod => "#AED6F1",
        SymbolKind::Struct | SymbolKind::Class => "#A9DFBF",
        SymbolKind::Trait | SymbolKind::Interface => "#F9E79F",
        SymbolKind::Enum => "#F1948A",
        SymbolKind::TypeAlias => "#D7BDE2",
        SymbolKind::Const | SymbolKind::Static | SymbolKind::Variable => "#FAD7A0",
        SymbolKind::Macro => "#FDFEFE",
        _ => "#EAECEE",
    }
}

/// Get a short display label for a SymbolKind.
fn kind_label(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Enum => "enum",
        SymbolKind::Variable => "var",
        SymbolKind::Component => "component",
        SymbolKind::Method => "method",
        SymbolKind::Property => "property",
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "trait",
        SymbolKind::ImplMethod => "impl fn",
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
        SymbolKind::Macro => "macro",
    }
}

/// Check whether an EdgeKind is a dependency-semantic edge suitable for export.
///
/// Skips structural edges: Contains, ChildOf, Imports, Exports.
fn is_dependency_edge(kind: &EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::ResolvedImport { .. }
            | EdgeKind::Calls
            | EdgeKind::Extends
            | EdgeKind::Implements
            | EdgeKind::BarrelReExportAll
            | EdgeKind::ReExport { .. }
            | EdgeKind::RustImport { .. }
    )
}

/// DOT edge style attributes for a given EdgeKind.
fn edge_style(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::ResolvedImport { .. } => "style=solid",
        EdgeKind::ReExport { .. } | EdgeKind::BarrelReExportAll => "style=dashed",
        EdgeKind::Calls => "style=solid color=blue",
        EdgeKind::Extends => "style=solid arrowhead=onormal",
        EdgeKind::Implements => "style=dashed arrowhead=onormal",
        EdgeKind::RustImport { .. } => "style=dotted",
        _ => "style=solid",
    }
}

/// Render the code graph as DOT format.
///
/// Supports symbol, file, and package granularity levels.
/// Uses manual string generation for all levels (consistent approach, supports cluster subgraphs).
pub fn render_dot(
    graph: &CodeGraph,
    params: &ExportParams,
    module_path_map: &HashMap<PathBuf, String>,
    visible_nodes: &HashSet<NodeIndex>,
) -> String {
    let mut out = String::new();
    writeln!(out, "digraph code_graph {{").unwrap();
    writeln!(out, "    rankdir=TB;").unwrap();
    writeln!(out, "    node [style=filled fontname=monospace];").unwrap();

    match params.granularity {
        Granularity::Symbol => render_dot_symbol(graph, module_path_map, visible_nodes, &mut out),
        Granularity::File => render_dot_file(graph, params, visible_nodes, &mut out),
        Granularity::Package => render_dot_package(graph, params, visible_nodes, &mut out),
    }

    writeln!(out, "}}").unwrap();
    out
}

/// Symbol-granularity DOT: one node per Symbol node in the graph.
fn render_dot_symbol(
    graph: &CodeGraph,
    module_path_map: &HashMap<PathBuf, String>,
    visible_nodes: &HashSet<NodeIndex>,
    out: &mut String,
) {
    // Emit symbol nodes.
    for idx in graph.graph.node_indices() {
        if !visible_nodes.contains(&idx) {
            continue;
        }
        if let GraphNode::Symbol(ref s) = graph.graph[idx] {
            // Try to find the parent file's module path for Rust files.
            let module_annotation = {
                let mut annotation = String::new();
                for edge in graph
                    .graph
                    .edges_directed(idx, petgraph::Direction::Incoming)
                {
                    if let EdgeKind::Contains = edge.weight()
                        && let GraphNode::File(ref fi) = graph.graph[edge.source()]
                        && let Some(mod_path) = module_path_map.get(&fi.path)
                    {
                        annotation = format!("\\n{}", mod_path);
                    }
                }
                annotation
            };

            let label = format!("{} ({}){}", s.name, kind_label(&s.kind), module_annotation);
            let color = symbol_fillcolor(&s.kind);
            let node_id = format!("n{}", idx.index());
            writeln!(
                out,
                "    {} [label=\"{}\" fillcolor=\"{}\"];",
                node_id, label, color
            )
            .unwrap();
        }
    }

    // Emit dependency edges between visible symbol nodes.
    for edge in graph.graph.edge_references() {
        let src = edge.source();
        let tgt = edge.target();
        // Skip self-edges.
        if src == tgt {
            continue;
        }
        if !visible_nodes.contains(&src) || !visible_nodes.contains(&tgt) {
            continue;
        }
        // Only symbol -> symbol dependency edges.
        let is_sym_src = matches!(graph.graph[src], GraphNode::Symbol(_));
        let is_sym_tgt = matches!(graph.graph[tgt], GraphNode::Symbol(_));
        if !is_sym_src || !is_sym_tgt {
            continue;
        }
        if !is_dependency_edge(edge.weight()) {
            continue;
        }
        let style = edge_style(edge.weight());
        writeln!(out, "    n{} -> n{} [{}];", src.index(), tgt.index(), style).unwrap();
    }
}

/// File-granularity DOT: one node per File node, aggregated inter-file edges.
fn render_dot_file(
    graph: &CodeGraph,
    params: &ExportParams,
    visible_nodes: &HashSet<NodeIndex>,
    out: &mut String,
) {
    // Emit file nodes.
    for idx in graph.graph.node_indices() {
        if !visible_nodes.contains(&idx) {
            continue;
        }
        if let GraphNode::File(ref fi) = graph.graph[idx] {
            let rel_path = fi
                .path
                .strip_prefix(&params.project_root)
                .unwrap_or(&fi.path);
            let label = rel_path.display().to_string();
            let node_id = format!("n{}", idx.index());
            writeln!(
                out,
                "    {} [label=\"{}\" fillcolor=\"#AED6F1\"];",
                node_id, label
            )
            .unwrap();
        }
    }

    // Aggregate inter-file dependency edges.
    let mut edge_counts: HashMap<(NodeIndex, NodeIndex), usize> = HashMap::new();
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
        if !is_dependency_edge(edge.weight()) {
            continue;
        }
        *edge_counts.entry((src, tgt)).or_insert(0) += 1;
    }

    for ((src, tgt), count) in &edge_counts {
        let label = if *count == 1 {
            "1 import".to_string()
        } else {
            format!("{} imports", count)
        };
        writeln!(
            out,
            "    n{} -> n{} [label=\"{}\"];",
            src.index(),
            tgt.index(),
            label
        )
        .unwrap();
    }
}

/// Package-granularity DOT: subgraph cluster_* blocks per package, inter-package edges only.
fn render_dot_package(
    graph: &CodeGraph,
    params: &ExportParams,
    visible_nodes: &HashSet<NodeIndex>,
    out: &mut String,
) {
    // Determine package membership for visible file nodes.
    let package_map = build_package_map(graph, params, visible_nodes);

    // Group file nodes by package.
    let mut packages: HashMap<String, Vec<NodeIndex>> = HashMap::new();
    for (node_idx, pkg_name) in &package_map {
        packages
            .entry(pkg_name.clone())
            .or_default()
            .push(*node_idx);
    }

    // Emit subgraph cluster blocks.
    for (pkg_name, file_nodes) in &packages {
        let cluster_id = sanitize_dot_id(pkg_name);
        writeln!(out, "    subgraph cluster_{} {{", cluster_id).unwrap();
        writeln!(out, "        label=\"{}\";", pkg_name).unwrap();
        writeln!(out, "        color=lightgrey;").unwrap();
        writeln!(out, "        style=filled;").unwrap();
        for &node_idx in file_nodes {
            if let GraphNode::File(ref fi) = graph.graph[node_idx] {
                let rel_path = fi
                    .path
                    .strip_prefix(&params.project_root)
                    .unwrap_or(&fi.path);
                let label = rel_path.display().to_string();
                writeln!(
                    out,
                    "        n{} [label=\"{}\" fillcolor=\"#AED6F1\"];",
                    node_idx.index(),
                    label
                )
                .unwrap();
            }
        }
        writeln!(out, "    }}").unwrap();
    }

    // Emit inter-package edges only (aggregate by package pair).
    let mut inter_pkg_edges: HashMap<(String, String), usize> = HashMap::new();
    // Also track representative node indices for the edge endpoints.
    let mut pkg_rep_node: HashMap<String, NodeIndex> = HashMap::new();
    for (node_idx, pkg_name) in &package_map {
        pkg_rep_node.entry(pkg_name.clone()).or_insert(*node_idx);
    }

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
        if !is_dependency_edge(edge.weight()) {
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
        if src_pkg == tgt_pkg {
            continue; // intra-package edge: skip
        }
        *inter_pkg_edges.entry((src_pkg, tgt_pkg)).or_insert(0) += 1;
    }

    for ((src_pkg, tgt_pkg), count) in &inter_pkg_edges {
        let src_node = match pkg_rep_node.get(src_pkg) {
            Some(n) => n,
            None => continue,
        };
        let tgt_node = match pkg_rep_node.get(tgt_pkg) {
            Some(n) => n,
            None => continue,
        };
        let label = if *count == 1 {
            "1 import".to_string()
        } else {
            format!("{} imports", count)
        };
        writeln!(
            out,
            "    n{} -> n{} [label=\"{}\"];",
            src_node.index(),
            tgt_node.index(),
            label
        )
        .unwrap();
    }
}

/// Build a map from file NodeIndex to package name for all visible file nodes.
///
/// For Rust projects: uses FileInfo.crate_name if available.
/// For non-Rust or missing crate_name: groups by top-level directory under src/.
/// Files not under src/ go into a "root" package.
pub fn build_package_map(
    graph: &CodeGraph,
    params: &ExportParams,
    visible_nodes: &HashSet<NodeIndex>,
) -> HashMap<NodeIndex, String> {
    let mut map: HashMap<NodeIndex, String> = HashMap::new();

    for idx in graph.graph.node_indices() {
        if !visible_nodes.contains(&idx) {
            continue;
        }
        if let GraphNode::File(ref fi) = graph.graph[idx] {
            let pkg_name = if let Some(ref crate_name) = fi.crate_name {
                // Rust file with known crate name.
                crate_name.clone()
            } else {
                // Group by top-level directory relative to project root.
                let rel = fi
                    .path
                    .strip_prefix(&params.project_root)
                    .unwrap_or(&fi.path);

                // Try to get the first path component under src/.
                let mut components = rel.components();
                let first = components
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned());
                let second = components
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned());

                match (first.as_deref(), second.as_deref()) {
                    (Some("src"), Some(dir)) => dir.trim_end_matches(".rs").to_string(),
                    (Some(dir), _) if dir != "src" => dir.to_string(),
                    _ => "root".to_string(),
                }
            };
            map.insert(idx, pkg_name);
        }
    }

    map
}

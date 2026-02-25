use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write;
use std::path::PathBuf;

use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::export::dot::build_package_map;
use crate::export::model::{ExportParams, Granularity};
use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::{GraphNode, SymbolKind};

/// Check whether an EdgeKind is a dependency-semantic edge suitable for export.
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

/// Get a short display label for a SymbolKind.
fn kind_label(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function | SymbolKind::ImplMethod => "fn",
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
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
        SymbolKind::Macro => "macro",
    }
}

/// Escape a string for safe use in Mermaid node labels (quotes inside labels break the syntax).
fn escape_mermaid_label(s: &str) -> String {
    s.replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('{', "&#123;")
        .replace('}', "&#125;")
}

/// Render the code graph as Mermaid flowchart format.
///
/// Supports symbol, file, and package granularity levels.
pub fn render_mermaid(
    graph: &CodeGraph,
    params: &ExportParams,
    module_path_map: &HashMap<PathBuf, String>,
    visible_nodes: &HashSet<NodeIndex>,
) -> String {
    let mut out = String::new();
    writeln!(out, "flowchart TB").unwrap();

    match params.granularity {
        Granularity::Symbol => {
            render_mermaid_symbol(graph, module_path_map, visible_nodes, &mut out)
        }
        Granularity::File => render_mermaid_file(graph, params, visible_nodes, &mut out),
        Granularity::Package => render_mermaid_package(graph, params, visible_nodes, &mut out),
    }

    out
}

/// Symbol-granularity Mermaid: one node per Symbol, shaped by kind.
fn render_mermaid_symbol(
    graph: &CodeGraph,
    module_path_map: &HashMap<PathBuf, String>,
    visible_nodes: &HashSet<NodeIndex>,
    out: &mut String,
) {
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
                        annotation = format!(" ({})", mod_path);
                    }
                }
                annotation
            };

            let label = escape_mermaid_label(&format!(
                "{} ({}){}",
                s.name,
                kind_label(&s.kind),
                module_annotation
            ));
            let node_id = format!("n{}", idx.index());

            // Node shape by kind.
            let node_def = match s.kind {
                SymbolKind::Function | SymbolKind::ImplMethod | SymbolKind::Method => {
                    // Rectangle (default)
                    format!("    {}[\"{}\"]", node_id, label)
                }
                SymbolKind::Struct | SymbolKind::Class | SymbolKind::Component => {
                    // Stadium/rounded
                    format!("    {}([\"{}\" ])", node_id, label)
                }
                SymbolKind::Enum => {
                    // Rhombus/diamond
                    format!("    {}{{\"{}\" }}", node_id, label)
                }
                SymbolKind::Trait | SymbolKind::Interface => {
                    // Rounded (parentheses)
                    format!("    {}([\"{}\" ])", node_id, label)
                }
                _ => {
                    // Default rectangle
                    format!("    {}[\"{}\"]", node_id, label)
                }
            };
            writeln!(out, "{}", node_def).unwrap();
        }
    }

    // Emit dependency edges between visible symbol nodes.
    for edge in graph.graph.edge_references() {
        let src = edge.source();
        let tgt = edge.target();
        if src == tgt {
            continue;
        }
        if !visible_nodes.contains(&src) || !visible_nodes.contains(&tgt) {
            continue;
        }
        if !matches!(graph.graph[src], GraphNode::Symbol(_)) {
            continue;
        }
        if !matches!(graph.graph[tgt], GraphNode::Symbol(_)) {
            continue;
        }
        if !is_dependency_edge(edge.weight()) {
            continue;
        }

        let arrow = match edge.weight() {
            EdgeKind::ReExport { .. } | EdgeKind::BarrelReExportAll | EdgeKind::Implements => {
                "-.->".to_string()
            }
            _ => "-->".to_string(),
        };

        writeln!(out, "    n{} {} n{}", src.index(), arrow, tgt.index()).unwrap();
    }
}

/// File-granularity Mermaid: one node per file, aggregated edges with counts.
fn render_mermaid_file(
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
            let label = escape_mermaid_label(&rel_path.display().to_string());
            writeln!(out, "    n{}[\"{}\"]", idx.index(), label).unwrap();
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
            "    n{} -->|\"{}\"|n{}",
            src.index(),
            label,
            tgt.index()
        )
        .unwrap();
    }
}

/// Package-granularity Mermaid: subgraph blocks per package, inter-package edges only.
fn render_mermaid_package(
    graph: &CodeGraph,
    params: &ExportParams,
    visible_nodes: &HashSet<NodeIndex>,
    out: &mut String,
) {
    let package_map = build_package_map(graph, params, visible_nodes);

    // Group file nodes by package.
    let mut packages: HashMap<String, Vec<NodeIndex>> = HashMap::new();
    for (node_idx, pkg_name) in &package_map {
        packages
            .entry(pkg_name.clone())
            .or_default()
            .push(*node_idx);
    }

    // Emit subgraph blocks.
    for (pkg_name, file_nodes) in &packages {
        // Mermaid subgraph IDs cannot contain spaces or special chars.
        let subgraph_id = pkg_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        writeln!(
            out,
            "    subgraph {}[\"{}\"]",
            subgraph_id,
            escape_mermaid_label(pkg_name)
        )
        .unwrap();
        for &node_idx in file_nodes {
            if let GraphNode::File(ref fi) = graph.graph[node_idx] {
                let rel_path = fi
                    .path
                    .strip_prefix(&params.project_root)
                    .unwrap_or(&fi.path);
                let label = escape_mermaid_label(&rel_path.display().to_string());
                writeln!(out, "        n{}[\"{}\"]", node_idx.index(), label).unwrap();
            }
        }
        writeln!(out, "    end").unwrap();
    }

    // Inter-package edges only, aggregated by package pair.
    let mut inter_pkg_edges: HashMap<(String, String), usize> = HashMap::new();
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
            continue;
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
            "    n{} -->|\"{}\"|n{}",
            src_node.index(),
            label,
            tgt_node.index()
        )
        .unwrap();
    }
}

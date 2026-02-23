use std::path::{Path, PathBuf};

use anyhow::Result;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use regex::RegexBuilder;

use crate::graph::{CodeGraph, edge::EdgeKind, node::{GraphNode, SymbolKind}};

/// A single matching symbol definition returned by `find_symbol`.
#[derive(Debug, Clone)]
pub struct FindResult {
    pub symbol_name: String,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub is_exported: bool,
    pub is_default: bool,
}

/// Convert a `SymbolKind` to its lowercase string representation used in output and filtering.
pub fn kind_to_str(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Enum => "enum",
        SymbolKind::Variable => "variable",
        SymbolKind::Component => "component",
        SymbolKind::Method => "method",
        SymbolKind::Property => "property",
    }
}

/// Find the parent file node of a symbol via a `Contains` edge.
///
/// `Contains` edges go FILE -> SYMBOL (outgoing from file, incoming to symbol).
/// We must filter specifically to `EdgeKind::Contains` because other edges (e.g. `Calls`)
/// also arrive at symbol nodes with a File as source.
fn find_containing_file(graph: &CodeGraph, sym_idx: petgraph::stable_graph::NodeIndex) -> Option<crate::graph::node::FileInfo> {
    graph
        .graph
        .edges_directed(sym_idx, Direction::Incoming)
        .find_map(|edge_ref| {
            if matches!(edge_ref.weight(), EdgeKind::Contains) {
                let source = edge_ref.source();
                if let GraphNode::File(ref fi) = graph.graph[source] {
                    return Some(fi.clone());
                }
            }
            None
        })
}

/// Find the containing file of a child symbol (one that has a ChildOf edge to its parent symbol).
///
/// ChildOf edges go CHILD -> PARENT (outgoing from child). So we traverse Outgoing to get
/// the parent symbol, then use `find_containing_file` on the parent.
fn find_containing_file_of_child(graph: &CodeGraph, child_idx: petgraph::stable_graph::NodeIndex) -> Option<crate::graph::node::FileInfo> {
    graph
        .graph
        .edges_directed(child_idx, Direction::Outgoing)
        .find_map(|edge_ref| {
            if matches!(edge_ref.weight(), EdgeKind::ChildOf) {
                let parent_sym_idx = edge_ref.target();
                find_containing_file(graph, parent_sym_idx)
            } else {
                None
            }
        })
}

/// Find symbols in `graph` matching the given regex `pattern`.
///
/// - `case_insensitive`: enable case-insensitive regex matching
/// - `kind_filter`: if non-empty, only include symbols whose kind string is in this list
/// - `file_filter`: if Some, only include symbols whose file path starts with this prefix
///   (matched as a relative path against `project_root`)
/// - `project_root`: used for relativizing file paths when applying `file_filter`
///
/// Returns results sorted by file path then line number.
pub fn find_symbol(
    graph: &CodeGraph,
    pattern: &str,
    case_insensitive: bool,
    kind_filter: &[String],
    file_filter: Option<&Path>,
    project_root: &Path,
) -> Result<Vec<FindResult>> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", pattern, e))?;

    let mut results: Vec<FindResult> = Vec::new();

    // Iterate symbol_index keys — O(symbols). Regex compiled ONCE above.
    for (name, node_indices) in &graph.symbol_index {
        if !re.is_match(name) {
            continue;
        }

        for &sym_idx in node_indices {
            let sym_info = match &graph.graph[sym_idx] {
                GraphNode::Symbol(info) => info.clone(),
                _ => continue,
            };

            // Kind filter (if any).
            if !kind_filter.is_empty() {
                let kind_str = kind_to_str(&sym_info.kind);
                if !kind_filter.iter().any(|k| k.as_str() == kind_str) {
                    continue;
                }
            }

            // Find parent file via Contains edge (not just any incoming file neighbor).
            // Falls back to ChildOf -> Contains for child symbols.
            let file_info = find_containing_file(graph, sym_idx)
                .or_else(|| find_containing_file_of_child(graph, sym_idx));

            let file_info = match file_info {
                Some(fi) => fi,
                None => continue, // Cannot locate file — skip.
            };

            // File filter: match relative path prefix.
            if let Some(filter) = file_filter {
                let rel_path = file_info
                    .path
                    .strip_prefix(project_root)
                    .unwrap_or(&file_info.path);
                if !rel_path.starts_with(filter) {
                    continue;
                }
            }

            results.push(FindResult {
                symbol_name: sym_info.name.clone(),
                kind: sym_info.kind.clone(),
                file_path: file_info.path.clone(),
                line: sym_info.line,
                col: sym_info.col,
                is_exported: sym_info.is_exported,
                is_default: sym_info.is_default,
            });
        }
    }

    // Sort by file path then line number for deterministic output.
    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    Ok(results)
}

/// Compile `pattern` as a regex and collect all matching symbol names with their node indices.
///
/// Returns a vec of `(name, indices)` pairs — one entry per unique symbol name that matches.
/// The caller decides whether an empty result is an error.
///
/// `case_insensitive`: enable case-insensitive matching.
pub fn match_symbols(
    graph: &CodeGraph,
    pattern: &str,
    case_insensitive: bool,
) -> Result<Vec<(String, Vec<NodeIndex>)>> {
    let re = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", pattern, e))?;

    let matches: Vec<(String, Vec<NodeIndex>)> = graph
        .symbol_index
        .iter()
        .filter(|(name, _)| re.is_match(name))
        .map(|(name, indices)| (name.clone(), indices.clone()))
        .collect();

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::{CodeGraph, node::{SymbolInfo, SymbolKind}};

    fn make_graph_with_symbols() -> (CodeGraph, PathBuf) {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let f1 = graph.add_file(root.join("src/user.ts"), "typescript");
        graph.add_symbol(
            f1,
            SymbolInfo {
                name: "UserService".into(),
                kind: SymbolKind::Class,
                line: 10,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );

        let f2 = graph.add_file(root.join("src/auth.ts"), "typescript");
        graph.add_symbol(
            f2,
            SymbolInfo {
                name: "AuthService".into(),
                kind: SymbolKind::Class,
                line: 5,
                col: 0,
                is_exported: true,
                is_default: false,
            },
        );
        graph.add_symbol(
            f2,
            SymbolInfo {
                name: "greetUser".into(),
                kind: SymbolKind::Function,
                line: 20,
                col: 0,
                is_exported: false,
                is_default: false,
            },
        );

        (graph, root)
    }

    #[test]
    fn test_exact_name_match() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "UserService", false, &[], None, &root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "UserService");
        assert_eq!(results[0].kind, SymbolKind::Class);
        assert_eq!(results[0].line, 10);
    }

    #[test]
    fn test_regex_pattern_matches_multiple() {
        let (graph, root) = make_graph_with_symbols();
        // ".*Service" should match both UserService and AuthService
        let results = find_symbol(&graph, ".*Service", false, &[], None, &root).unwrap();
        assert_eq!(results.len(), 2, "should match UserService and AuthService");
    }

    #[test]
    fn test_case_insensitive_flag() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "userservice", true, &[], None, &root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "UserService");
    }

    #[test]
    fn test_kind_filter() {
        let (graph, root) = make_graph_with_symbols();
        let kind_filter = vec!["function".to_string()];
        let results = find_symbol(&graph, ".*", false, &kind_filter, None, &root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "greetUser");
        assert_eq!(results[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_no_match_returns_empty() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "NonExistent", false, &[], None, &root).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_invalid_regex_returns_error() {
        let (graph, root) = make_graph_with_symbols();
        let err = find_symbol(&graph, "[unclosed", false, &[], None, &root);
        assert!(err.is_err(), "invalid regex should return an error");
    }

    #[test]
    fn test_calls_edge_does_not_affect_parent_file_lookup() {
        // Regression test: Calls edges (File -> Symbol) must not be confused with Contains edges.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let f1 = graph.add_file(root.join("src/greet.ts"), "typescript");
        let greet_sym = graph.add_symbol(
            f1,
            SymbolInfo {
                name: "greet".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 16,
                is_exported: true,
                is_default: false,
            },
        );

        // Simulate a Calls edge from main.ts (file) to greet (symbol), as the resolver does.
        let f2 = graph.add_file(root.join("src/main.ts"), "typescript");
        graph.add_calls_edge(f2, greet_sym);

        let results = find_symbol(&graph, "greet", false, &[], None, &root).unwrap();
        assert_eq!(results.len(), 1, "should find exactly one definition");
        assert_eq!(
            results[0].file_path,
            root.join("src/greet.ts"),
            "greet should be in greet.ts, not main.ts"
        );
    }
}

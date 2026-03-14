use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;
use regex::RegexBuilder;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{DecoratorInfo, GraphNode, SymbolKind, SymbolVisibility},
};

/// Indicates how a search result was matched. Used in BM25/hybrid search (plan 20-01).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    Exact,
    Trigram,
    Bm25,
}

#[cfg(test)]
impl std::fmt::Display for MatchMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchMethod::Exact => write!(f, "[exact]"),
            MatchMethod::Trigram => write!(f, "[trigram]"),
            MatchMethod::Bm25 => write!(f, "[BM25]"),
        }
    }
}

/// A single matching symbol definition returned by `find_symbol`.
#[derive(Debug, Clone)]
pub struct FindResult {
    pub symbol_name: String,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub line: usize,
    pub line_end: usize,
    pub col: usize,
    pub is_exported: bool,
    pub is_default: bool,
    pub visibility: SymbolVisibility,
    #[allow(dead_code)]
    pub decorators: Vec<DecoratorInfo>,
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
        // Rust-specific kinds (Phase 8)
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "trait",
        SymbolKind::ImplMethod => "impl_method",
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
        SymbolKind::Macro => "macro",
    }
}

/// Find the parent file node of a symbol via a `Contains` edge.
///
/// `Contains` edges go FILE -> SYMBOL (outgoing from file, incoming to symbol).
/// We must filter specifically to `EdgeKind::Contains` because other edges (e.g. `Calls`)
/// also arrive at symbol nodes with a File as source.
fn find_containing_file(
    graph: &CodeGraph,
    sym_idx: petgraph::stable_graph::NodeIndex,
) -> Option<crate::graph::node::FileInfo> {
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
fn find_containing_file_of_child(
    graph: &CodeGraph,
    child_idx: petgraph::stable_graph::NodeIndex,
) -> Option<crate::graph::node::FileInfo> {
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
/// - `language_filter`: if Some, only include symbols from files with this language string
///   (e.g. "rust", "typescript", "javascript")
///
/// Returns results sorted by file path then line number.
pub fn find_symbol(
    graph: &CodeGraph,
    pattern: &str,
    case_insensitive: bool,
    kind_filter: &[String],
    file_filter: Option<&Path>,
    project_root: &Path,
    language_filter: Option<&str>,
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

            // Language filter: skip symbols from files whose language doesn't match.
            if let Some(lang) = language_filter
                && file_info.language.as_str() != lang
            {
                continue;
            }

            results.push(FindResult {
                symbol_name: sym_info.name.clone(),
                kind: sym_info.kind.clone(),
                file_path: file_info.path.clone(),
                line: sym_info.line,
                line_end: sym_info.line_end,
                col: sym_info.col,
                is_exported: sym_info.is_exported,
                is_default: sym_info.is_default,
                visibility: sym_info.visibility.clone(),
                decorators: sym_info.decorators.clone(),
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

// ---------------------------------------------------------------------------
// Trigram helpers (moved from server.rs so find.rs can reuse them)
// ---------------------------------------------------------------------------

/// Compute character-level trigrams from a string (lowercased).
/// Returns an empty set for strings shorter than 3 characters. Used in plan 20-01.
pub(crate) fn trigrams(s: &str) -> HashSet<[char; 3]> {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    if chars.len() < 3 {
        return HashSet::new();
    }
    chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// Jaccard similarity between two trigram sets: |A ∩ B| / |A ∪ B|.
/// Returns 0.0 if both sets are empty (no useful comparison possible). Used in plan 20-01.
pub(crate) fn jaccard_similarity(a: &HashSet<[char; 3]>, b: &HashSet<[char; 3]>) -> f32 {
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

// ---------------------------------------------------------------------------
// Tiered search functions
// ---------------------------------------------------------------------------

/// Find symbols using trigram similarity. Returns `FindResult` items for all
/// symbols whose Jaccard similarity with `query` is >= 0.3.
/// Results are sorted by score descending and limited to `limit`. Used in plan 20-01.
pub fn find_symbol_trigram(graph: &CodeGraph, query: &str, limit: usize) -> Vec<FindResult> {
    let query_trigrams = trigrams(query);
    if query_trigrams.is_empty() {
        return Vec::new();
    }

    const THRESHOLD: f32 = 0.3;

    let mut scored: Vec<(FindResult, f32)> = Vec::new();

    for (name, node_indices) in &graph.symbol_index {
        let name_trigrams = trigrams(name);
        let score = jaccard_similarity(&query_trigrams, &name_trigrams);
        if score < THRESHOLD {
            continue;
        }

        for &sym_idx in node_indices {
            let sym_info = match &graph.graph[sym_idx] {
                crate::graph::node::GraphNode::Symbol(info) => info.clone(),
                _ => continue,
            };

            let file_info = find_containing_file(graph, sym_idx)
                .or_else(|| find_containing_file_of_child(graph, sym_idx));

            if let Some(fi) = file_info {
                scored.push((
                    FindResult {
                        symbol_name: sym_info.name.clone(),
                        kind: sym_info.kind.clone(),
                        file_path: fi.path.clone(),
                        line: sym_info.line,
                        line_end: sym_info.line_end,
                        col: sym_info.col,
                        is_exported: sym_info.is_exported,
                        is_default: sym_info.is_default,
                        visibility: sym_info.visibility.clone(),
                        decorators: sym_info.decorators.clone(),
                    },
                    score,
                ));
            }
        }
    }

    // Sort descending by score
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.file_path.cmp(&b.0.file_path))
            .then(a.0.line.cmp(&b.0.line))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(r, _)| r).collect()
}

/// Search for symbols using the BM25 full-text index.
/// Returns an empty vec if the BM25 index is not built yet (`bm25_index` is None). Used in plan 20-01.
pub fn bm25_search(graph: &CodeGraph, query: &str, limit: usize) -> Vec<FindResult> {
    let engine = match &graph.bm25_index {
        Some(e) => e,
        None => return Vec::new(),
    };

    let search_results = engine.search(query, limit);
    let mut results = Vec::new();

    for sr in search_results {
        let node_idx = petgraph::stable_graph::NodeIndex::new(sr.document.id as usize);
        if let Some(GraphNode::Symbol(sym)) = graph.graph.node_weight(node_idx) {
            let file_info = find_containing_file(graph, node_idx)
                .or_else(|| find_containing_file_of_child(graph, node_idx));

            if let Some(fi) = file_info {
                results.push(FindResult {
                    symbol_name: sym.name.clone(),
                    kind: sym.kind.clone(),
                    file_path: fi.path.clone(),
                    line: sym.line,
                    line_end: sym.line_end,
                    col: sym.col,
                    is_exported: sym.is_exported,
                    is_default: sym.is_default,
                    visibility: sym.visibility.clone(),
                    decorators: sym.decorators.clone(),
                });
            }
        }
    }

    results
}

/// Merge two ranked result lists using Reciprocal Rank Fusion (k=60).
/// Returns a unified list sorted by combined RRF score, highest first. Used in plan 20-01.
pub fn reciprocal_rank_fusion(list_a: &[FindResult], list_b: &[FindResult]) -> Vec<FindResult> {
    let k = 60.0_f32;
    let mut scores: HashMap<String, (f32, FindResult)> = HashMap::new();

    for (rank, result) in list_a.iter().enumerate() {
        let key = format!("{}:{}", result.symbol_name, result.line);
        let score = 1.0 / (k + (rank + 1) as f32);
        scores
            .entry(key)
            .and_modify(|(s, _)| *s += score)
            .or_insert((score, result.clone()));
    }
    for (rank, result) in list_b.iter().enumerate() {
        let key = format!("{}:{}", result.symbol_name, result.line);
        let score = 1.0 / (k + (rank + 1) as f32);
        scores
            .entry(key)
            .and_modify(|(s, _)| *s += score)
            .or_insert((score, result.clone()));
    }

    let mut merged: Vec<(f32, FindResult)> = scores.into_values().collect();
    merged.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.file_path.cmp(&b.1.file_path))
            .then(a.1.line.cmp(&b.1.line))
    });
    merged.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::{
        CodeGraph,
        node::{SymbolInfo, SymbolKind},
    };

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
                is_exported: true,
                ..Default::default()
            },
        );

        let f2 = graph.add_file(root.join("src/auth.ts"), "typescript");
        graph.add_symbol(
            f2,
            SymbolInfo {
                name: "AuthService".into(),
                kind: SymbolKind::Class,
                line: 5,
                is_exported: true,
                ..Default::default()
            },
        );
        graph.add_symbol(
            f2,
            SymbolInfo {
                name: "greetUser".into(),
                kind: SymbolKind::Function,
                line: 20,
                ..Default::default()
            },
        );

        (graph, root)
    }

    #[test]
    fn test_exact_name_match() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "UserService", false, &[], None, &root, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "UserService");
        assert_eq!(results[0].kind, SymbolKind::Class);
        assert_eq!(results[0].line, 10);
    }

    #[test]
    fn test_regex_pattern_matches_multiple() {
        let (graph, root) = make_graph_with_symbols();
        // ".*Service" should match both UserService and AuthService
        let results = find_symbol(&graph, ".*Service", false, &[], None, &root, None).unwrap();
        assert_eq!(results.len(), 2, "should match UserService and AuthService");
    }

    #[test]
    fn test_case_insensitive_flag() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "userservice", true, &[], None, &root, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "UserService");
    }

    #[test]
    fn test_kind_filter() {
        let (graph, root) = make_graph_with_symbols();
        let kind_filter = vec!["function".to_string()];
        let results = find_symbol(&graph, ".*", false, &kind_filter, None, &root, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "greetUser");
        assert_eq!(results[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_no_match_returns_empty() {
        let (graph, root) = make_graph_with_symbols();
        let results = find_symbol(&graph, "NonExistent", false, &[], None, &root, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_invalid_regex_returns_error() {
        let (graph, root) = make_graph_with_symbols();
        let err = find_symbol(&graph, "[unclosed", false, &[], None, &root, None);
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
                ..Default::default()
            },
        );

        // Simulate a Calls edge from main.ts (file) to greet (symbol), as the resolver does.
        let f2 = graph.add_file(root.join("src/main.ts"), "typescript");
        graph.add_calls_edge(f2, greet_sym);

        let results = find_symbol(&graph, "greet", false, &[], None, &root, None).unwrap();
        assert_eq!(results.len(), 1, "should find exactly one definition");
        assert_eq!(
            results[0].file_path,
            root.join("src/greet.ts"),
            "greet should be in greet.ts, not main.ts"
        );
    }

    // -----------------------------------------------------------------------
    // MatchMethod tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_match_method_display() {
        assert_eq!(MatchMethod::Exact.to_string(), "[exact]");
        assert_eq!(MatchMethod::Trigram.to_string(), "[trigram]");
        assert_eq!(MatchMethod::Bm25.to_string(), "[BM25]");
    }

    // -----------------------------------------------------------------------
    // Trigram search tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_symbol_trigram_returns_fuzzy_matches() {
        // "authHandler" should be found when querying "authHandlr" (typo)
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let f = graph.add_file(root.join("src/auth.ts"), "typescript");
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "authHandler".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        let results = find_symbol_trigram(&graph, "authHandlr", 10);
        assert!(
            !results.is_empty(),
            "trigram search should find authHandler for typo authHandlr"
        );
        assert_eq!(results[0].symbol_name, "authHandler");
    }

    // -----------------------------------------------------------------------
    // BM25 search tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bm25_search_returns_results() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let f = graph.add_file(root.join("src/auth.ts"), "typescript");
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "authHandler".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        // Rebuild BM25 index first
        graph.rebuild_bm25_index();

        let results = bm25_search(&graph, "auth handler", 10);
        assert!(
            !results.is_empty(),
            "BM25 search for 'auth handler' should find authHandler"
        );
        assert_eq!(results[0].symbol_name, "authHandler");
    }

    #[test]
    fn test_bm25_search_no_index_returns_empty() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let f = graph.add_file(root.join("src/auth.ts"), "typescript");
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "authHandler".into(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        // Do NOT call rebuild_bm25_index — bm25_index stays None

        let results = bm25_search(&graph, "auth", 10);
        assert!(
            results.is_empty(),
            "bm25_search with no index should return empty vec"
        );
    }

    // -----------------------------------------------------------------------
    // Reciprocal Rank Fusion tests
    // -----------------------------------------------------------------------

    fn make_find_result(name: &str, line: usize) -> FindResult {
        FindResult {
            symbol_name: name.to_string(),
            kind: SymbolKind::Function,
            file_path: PathBuf::from("/proj/src/a.ts"),
            line,
            line_end: line,
            col: 0,
            is_exported: false,
            is_default: false,
            visibility: crate::graph::node::SymbolVisibility::Private,
            decorators: vec![],
        }
    }

    #[test]
    fn test_reciprocal_rank_fusion_empty_lists() {
        let result = reciprocal_rank_fusion(&[], &[]);
        assert!(
            result.is_empty(),
            "merging empty lists should produce empty result"
        );
    }

    #[test]
    fn test_reciprocal_rank_fusion_merges_lists() {
        // list_a: alpha at rank 0, beta at rank 1
        // list_b: beta at rank 0, gamma at rank 1
        // Expected: beta scores highest (appears in both)
        let list_a = vec![make_find_result("alpha", 1), make_find_result("beta", 2)];
        let list_b = vec![make_find_result("beta", 2), make_find_result("gamma", 3)];

        let merged = reciprocal_rank_fusion(&list_a, &list_b);
        assert_eq!(merged.len(), 3, "merged list should have 3 unique results");
        // beta appears in both lists with rank 1 (list_a) and rank 0 (list_b),
        // so its RRF score = 1/(60+2) + 1/(60+1) > any single-list entry
        assert_eq!(
            merged[0].symbol_name, "beta",
            "beta should rank first since it appears in both lists"
        );
    }
}

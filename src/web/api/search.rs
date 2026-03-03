use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::query::find::{
    FindResult, bm25_search, find_symbol, find_symbol_trigram, kind_to_str, reciprocal_rank_fusion,
};

use super::super::server::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

/// Serialisable representation of a search result.
#[derive(Serialize)]
pub struct SearchResult {
    pub symbol: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub line_end: usize,
    pub is_exported: bool,
}

impl From<FindResult> for SearchResult {
    fn from(r: FindResult) -> Self {
        SearchResult {
            symbol: r.symbol_name,
            kind: kind_to_str(&r.kind).to_string(),
            file: r.file_path.to_string_lossy().to_string(),
            line: r.line,
            line_end: r.line_end,
            is_exported: r.is_exported,
        }
    }
}

/// GET /api/search?q=CodeGraph&limit=20
///
/// Returns matching symbol definitions as a JSON array.
///
/// Uses a tiered pipeline mirroring the MCP find_symbol tool:
/// - Tier 1: exact/regex find_symbol (case-insensitive) — returned immediately on hit
/// - Tier 2+3: trigram + BM25 fuzzy search when Tier 1 misses
/// - If both trigram and BM25 hit: merge via reciprocal rank fusion (RRF)
/// - If only one hits: return that tier's results
pub async fn handler(
    Query(params): Query<SearchQuery>,
    State(state): State<AppState>,
) -> Result<Json<Vec<SearchResult>>, (StatusCode, String)> {
    let q = &params.q;
    let limit = params.limit;
    let graph = state.graph.read().await;

    // Tier 1: exact/regex match (case-insensitive, no kind/file/language filter)
    let tier1 = find_symbol(
        &graph,
        q,
        true, // case-insensitive
        &[],  // no kind filter
        None, // no file filter
        &state.project_root,
        None, // no language filter
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    if !tier1.is_empty() {
        // Tier 1 hit — return immediately
        let limited: Vec<SearchResult> = tier1
            .into_iter()
            .take(limit)
            .map(SearchResult::from)
            .collect();
        return Ok(Json(limited));
    }

    // Tier 1 miss — fall through to trigram + BM25
    let trigram = find_symbol_trigram(&graph, q, limit);
    let bm25 = bm25_search(&graph, q, limit);

    let results: Vec<FindResult> = match (trigram.is_empty(), bm25.is_empty()) {
        (false, false) => reciprocal_rank_fusion(&trigram, &bm25),
        (false, true) => trigram,
        (true, false) => bm25,
        (true, true) => vec![],
    };

    let limited: Vec<SearchResult> = results
        .into_iter()
        .take(limit)
        .map(SearchResult::from)
        .collect();

    Ok(Json(limited))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CodeGraph;
    use crate::graph::node::{SymbolInfo, SymbolKind};
    use crate::query::find::bm25_search;
    use std::path::PathBuf;

    #[test]
    fn test_search_api_returns_matches() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "MyService".to_string(),
                kind: SymbolKind::Struct,
                line: 10,
                ..Default::default()
            },
        );
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "OtherThing".to_string(),
                kind: SymbolKind::Function,
                line: 20,
                ..Default::default()
            },
        );

        let results = find_symbol(&graph, "MyService", true, &[], None, &root, None)
            .expect("search should succeed");

        assert_eq!(results.len(), 1, "should find exactly one match");
        assert_eq!(results[0].symbol_name, "MyService");
    }

    #[test]
    fn test_search_api_case_insensitive() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "CodeGraph".to_string(),
                kind: SymbolKind::Struct,
                line: 5,
                ..Default::default()
            },
        );

        let results = find_symbol(&graph, "codegraph", true, &[], None, &root, None)
            .expect("case-insensitive search should succeed");

        assert_eq!(results.len(), 1, "case-insensitive match expected");
    }

    /// Test that BM25 search finds "authHandler" via multi-word query "auth handler"
    /// which would miss in Tier 1 (exact/regex find_symbol) but succeed in Tier 3 (BM25).
    #[test]
    fn test_search_bm25_fuzzy_fallback() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(root.join("src/auth.rs"), "rust");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "authHandler".to_string(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        // Rebuild BM25 index so authHandler is indexed
        graph.rebuild_bm25_index();

        // Tier 1 miss: "auth handler" (with space) does not match "authHandler" exactly
        let tier1 = find_symbol(&graph, "auth handler", true, &[], None, &root, None)
            .expect("find_symbol should not error");
        assert!(
            tier1.is_empty(),
            "Tier 1 should miss multi-word 'auth handler' query for symbol 'authHandler'"
        );

        // Tier 3: BM25 should find "authHandler" via fuzzy token matching
        let bm25_results = bm25_search(&graph, "auth handler", 10);
        assert!(
            !bm25_results.is_empty(),
            "BM25 should find 'authHandler' for query 'auth handler'"
        );
        assert_eq!(
            bm25_results[0].symbol_name, "authHandler",
            "first BM25 result should be 'authHandler'"
        );
    }
}

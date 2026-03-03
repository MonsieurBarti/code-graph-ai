use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::graph::node::GraphNode;
use crate::query::stats::project_stats;

use super::super::server::AppState;

/// Per-language file/symbol counts for the frontend.
#[derive(Serialize)]
pub struct LanguageStats {
    pub language: String,
    pub files: usize,
    pub symbols: usize,
}

/// Serialisable subset of ProjectStats for the web API response.
///
/// Field names match the frontend `StatsResponse` TypeScript interface.
#[derive(Serialize)]
pub struct StatsResponse {
    pub project_root: String,
    pub total_files: usize,
    pub total_symbols: usize,
    pub languages: Vec<LanguageStats>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub functions: usize,
    pub classes: usize,
    pub interfaces: usize,
    pub type_aliases: usize,
    pub enums: usize,
    pub variables: usize,
    pub components: usize,
    pub methods: usize,
    pub import_edges: usize,
    pub external_packages: usize,
    pub unresolved_imports: usize,
    // Language-specific counts
    pub rust_fns: usize,
    pub rust_structs: usize,
    pub rust_enums: usize,
    pub rust_traits: usize,
    pub python_file_count: usize,
    pub python_symbol_count: usize,
    pub go_file_count: usize,
    pub go_symbol_count: usize,
    pub non_parsed_files: usize,
}

/// GET /api/stats
///
/// Returns project statistics as JSON.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<StatsResponse>, (StatusCode, String)> {
    let graph = state.graph.read().await;
    let stats = project_stats(&graph);

    // Build per-language breakdown from file nodes.
    let mut lang_files: HashMap<String, usize> = HashMap::new();
    let mut lang_symbols: HashMap<String, usize> = HashMap::new();

    for idx in graph.graph.node_indices() {
        if let GraphNode::File(ref fi) = graph.graph[idx] {
            *lang_files.entry(fi.language.clone()).or_default() += 1;
            // Count symbols contained in this file.
            let sym_count = graph
                .graph
                .edges(idx)
                .filter(|e| matches!(e.weight(), crate::graph::edge::EdgeKind::Contains))
                .count();
            *lang_symbols.entry(fi.language.clone()).or_default() += sym_count;
        }
    }

    let mut languages: Vec<LanguageStats> = lang_files
        .into_iter()
        .map(|(language, files)| LanguageStats {
            symbols: *lang_symbols.get(&language).unwrap_or(&0),
            language,
            files,
        })
        .collect();
    // Sort by file count descending for the frontend.
    languages.sort_by(|a, b| b.files.cmp(&a.files));

    let project_root = state.project_root.to_string_lossy().to_string();

    Ok(Json(StatsResponse {
        project_root,
        total_files: stats.file_count,
        total_symbols: stats.symbol_count,
        languages,
        file_count: stats.file_count,
        symbol_count: stats.symbol_count,
        functions: stats.functions,
        classes: stats.classes,
        interfaces: stats.interfaces,
        type_aliases: stats.type_aliases,
        enums: stats.enums,
        variables: stats.variables,
        components: stats.components,
        methods: stats.methods,
        import_edges: stats.import_edges,
        external_packages: stats.external_packages,
        unresolved_imports: stats.unresolved_imports,
        rust_fns: stats.rust_fns,
        rust_structs: stats.rust_structs,
        rust_enums: stats.rust_enums,
        rust_traits: stats.rust_traits,
        python_file_count: stats.python_file_count,
        python_symbol_count: stats.python_symbol_count,
        go_file_count: stats.go_file_count,
        go_symbol_count: stats.go_symbol_count,
        non_parsed_files: stats.non_parsed_files,
    }))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CodeGraph;
    use crate::graph::node::{SymbolInfo, SymbolKind};
    use std::path::PathBuf;

    #[test]
    fn test_stats_api_returns_correct_counts() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "MyStruct".to_string(),
                kind: SymbolKind::Struct,
                line: 10,
                ..Default::default()
            },
        );
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "my_fn".to_string(),
                kind: SymbolKind::Function,
                line: 20,
                ..Default::default()
            },
        );

        let stats = project_stats(&graph);
        assert_eq!(stats.file_count, 1, "one file expected");
        assert_eq!(stats.symbol_count, 2, "two symbols expected");
    }
}

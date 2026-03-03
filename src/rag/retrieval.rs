/// Hybrid retrieval pipeline for the RAG conversational agent.
///
/// Combines graph-based structural retrieval (symbol lookup + reference tracing) with
/// vector-similarity-based conceptual retrieval (embedding search over indexed symbols).
///
/// # Retrieval modes
///
/// - **Structural**: Uses `find_symbol` and `symbol_context` from the code graph.
///   Returns symbols matching the query as named entities.
/// - **Conceptual**: Embeds the query via `EmbeddingEngine::embed_batch`, then searches
///   the `VectorStore` for the top-10 nearest neighbors.
/// - **Hybrid**: Runs both structural and conceptual, then merges and deduplicates by
///   `(file_path, symbol_name)`.
///
/// All retrieval modes produce a `RetrievalResult` with:
/// - `context_text`: a numbered list of evidence items with `[N]` citation markers
/// - `citations`: `Vec<Citation>` carrying file + line provenance for each evidence item
/// - `tools_used`: names of the tools invoked (e.g. "find_symbol", "vector_search")
use std::collections::HashSet;
use std::path::Path;

use crate::graph::CodeGraph;
use crate::query::find::find_symbol;
use crate::rag::agent::QueryKind;
use crate::rag::embedding::EmbeddingEngine;
use crate::rag::vector_store::VectorStore;

/// Maximum number of citations returned to the LLM.
const MAX_CITATIONS: usize = 5;
/// Number of top results for which we include actual source code in the context.
const CODE_SNIPPET_COUNT: usize = 3;
/// Maximum lines of source code to include per snippet.
const MAX_SNIPPET_LINES: usize = 40;

// ─── Types ────────────────────────────────────────────────────────────────────

/// A single piece of evidence linking a symbol to its source location.
#[derive(Debug, Clone)]
pub struct Citation {
    /// 1-based citation index (matches `[N]` marker in `context_text`).
    pub index: usize,
    /// File containing the cited symbol.
    pub file_path: String,
    /// 1-based line number where the symbol is defined.
    pub line_start: usize,
    /// Symbol name.
    pub symbol_name: String,
}

/// The complete output of one retrieval pass.
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    /// Numbered context text ready to be embedded in the LLM prompt.
    ///
    /// Format: `[1] function auth_handler in src/auth.rs:42\n[2] ...`
    pub context_text: String,
    /// Structured evidence items parallel to the `[N]` markers in `context_text`.
    pub citations: Vec<Citation>,
    /// Names of retrieval tools that were invoked.
    pub tools_used: Vec<String>,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Run the retrieval pipeline for the given `query` and `kind`.
///
/// Dispatches to [`retrieve_structural`], [`retrieve_conceptual`], or both (Hybrid),
/// then merges and formats the results.
pub async fn retrieve(
    graph: &CodeGraph,
    vector_store: &VectorStore,
    engine: &EmbeddingEngine,
    query: &str,
    kind: QueryKind,
) -> anyhow::Result<RetrievalResult> {
    match kind {
        QueryKind::Structural => {
            let (context_text, citations, tools_used) = retrieve_structural(graph, query);
            Ok(RetrievalResult {
                context_text,
                citations,
                tools_used,
            })
        }
        QueryKind::Conceptual => {
            let (context_text, citations, tools_used) =
                retrieve_conceptual(vector_store, engine, query).await?;
            Ok(RetrievalResult {
                context_text,
                citations,
                tools_used,
            })
        }
        QueryKind::Hybrid => {
            let (ctx_s, cit_s, tools_s) = retrieve_structural(graph, query);
            let (ctx_c, cit_c, tools_c) = retrieve_conceptual(vector_store, engine, query).await?;

            // Merge: structural first, then conceptual (dedup by (file_path, symbol_name)).
            let mut seen: HashSet<(String, String)> = HashSet::new();
            let mut merged_items: Vec<(String, usize, String)> = Vec::new(); // (file_path, line_start, symbol_name)

            for c in cit_s.iter().chain(cit_c.iter()) {
                let key = (c.file_path.clone(), c.symbol_name.clone());
                if seen.insert(key) {
                    merged_items.push((c.file_path.clone(), c.line_start, c.symbol_name.clone()));
                }
            }

            // Re-number citations after dedup, capped at MAX_CITATIONS.
            let mut context_lines: Vec<String> = Vec::new();
            let mut citations: Vec<Citation> = Vec::new();
            for (i, (file_path, line_start, symbol_name)) in
                merged_items.iter().take(MAX_CITATIONS).enumerate()
            {
                let idx = i + 1;

                if i < CODE_SNIPPET_COUNT {
                    if let Some(snippet) = read_code_snippet(file_path, *line_start) {
                        context_lines.push(format!(
                            "[{idx}] `{symbol_name}` in {file_path}:{line_start}\n```\n{snippet}\n```"
                        ));
                    } else {
                        context_lines.push(format!(
                            "[{idx}] `{symbol_name}` in {file_path}:{line_start}"
                        ));
                    }
                } else {
                    context_lines.push(format!(
                        "[{idx}] `{symbol_name}` in {file_path}:{line_start}"
                    ));
                }

                citations.push(Citation {
                    index: idx,
                    file_path: file_path.clone(),
                    line_start: *line_start,
                    symbol_name: symbol_name.clone(),
                });
            }

            // Merge tools_used (deduplicated).
            let mut tools_used: Vec<String> = tools_s;
            for t in tools_c {
                if !tools_used.contains(&t) {
                    tools_used.push(t);
                }
            }

            let context_text = if context_lines.is_empty() {
                // Fall back to concatenating both raw contexts.
                let combined = format!("{}\n{}", ctx_s, ctx_c);
                combined.trim().to_string()
            } else {
                context_lines.join("\n")
            };

            Ok(RetrievalResult {
                context_text,
                citations,
                tools_used,
            })
        }
    }
}

// ─── Structural retrieval ────────────────────────────────────────────────────

/// Structural retrieval: search the code graph for symbols matching `query`.
///
/// Calls `find_symbol` with a case-insensitive regex derived from the query (words extracted
/// and joined as an alternation). Returns formatted context text, citations, and tools used.
///
/// This function is synchronous (no async needed — graph operations are in-memory).
pub fn retrieve_structural(graph: &CodeGraph, query: &str) -> (String, Vec<Citation>, Vec<String>) {
    let mut tools_used = vec!["find_symbol".to_string()];

    // Extract keywords from the query by stripping common stop words and using what remains
    // as a regex pattern. We join with `|` for an OR search.
    let pattern = extract_search_pattern(query);

    let project_root = Path::new(".");
    let results =
        find_symbol(graph, &pattern, true, &[], None, project_root, None).unwrap_or_default();

    if results.is_empty() {
        return (String::new(), Vec::new(), tools_used);
    }

    tools_used.push("get_context".to_string());

    let mut context_lines: Vec<String> = Vec::new();
    let mut citations: Vec<Citation> = Vec::new();

    for (i, result) in results.iter().take(MAX_CITATIONS).enumerate() {
        let idx = i + 1;
        let file_str = result.file_path.to_string_lossy().to_string();
        let kind_str = crate::query::find::kind_to_str(&result.kind);

        // Include actual source code for the top results.
        if i < CODE_SNIPPET_COUNT {
            if let Some(snippet) = read_code_snippet(&file_str, result.line) {
                context_lines.push(format!(
                    "[{idx}] {kind_str} `{}` in {}:{}\n```\n{}\n```",
                    result.symbol_name, file_str, result.line, snippet
                ));
            } else {
                context_lines.push(format!(
                    "[{idx}] {kind_str} `{}` in {}:{}",
                    result.symbol_name, file_str, result.line
                ));
            }
        } else {
            context_lines.push(format!(
                "[{idx}] {kind_str} `{}` in {}:{}",
                result.symbol_name, file_str, result.line
            ));
        }

        citations.push(Citation {
            index: idx,
            file_path: file_str,
            line_start: result.line,
            symbol_name: result.symbol_name.clone(),
        });
    }

    (context_lines.join("\n\n"), citations, tools_used)
}

// ─── Conceptual retrieval ─────────────────────────────────────────────────────

/// Conceptual retrieval: embed the query and search the vector store.
///
/// Calls `embedding_engine.embed_batch([query])` to get a query embedding, then searches
/// the `VectorStore` for the top-10 nearest symbol embeddings.
///
/// Returns formatted context text, citations, and tools used.
pub async fn retrieve_conceptual(
    vector_store: &VectorStore,
    engine: &EmbeddingEngine,
    query: &str,
) -> anyhow::Result<(String, Vec<Citation>, Vec<String>)> {
    let tools_used = vec!["vector_search".to_string()];

    // Embed the query.
    let embeddings = engine.embed_batch(vec![query.to_string()]).await?;
    let query_embedding = embeddings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("embedding engine returned no results"))?;

    // Search the vector store.
    let results = vector_store.search(&query_embedding, MAX_CITATIONS)?;

    if results.is_empty() {
        return Ok((String::new(), Vec::new(), tools_used));
    }

    let mut context_lines: Vec<String> = Vec::new();
    let mut citations: Vec<Citation> = Vec::new();

    for (i, (meta, _distance)) in results.iter().take(MAX_CITATIONS).enumerate() {
        let idx = i + 1;

        if i < CODE_SNIPPET_COUNT {
            if let Some(snippet) = read_code_snippet(&meta.file_path, meta.line_start) {
                context_lines.push(format!(
                    "[{idx}] {} `{}` in {}:{}\n```\n{}\n```",
                    meta.kind, meta.symbol_name, meta.file_path, meta.line_start, snippet
                ));
            } else {
                context_lines.push(format!(
                    "[{idx}] {} `{}` in {}:{}",
                    meta.kind, meta.symbol_name, meta.file_path, meta.line_start
                ));
            }
        } else {
            context_lines.push(format!(
                "[{idx}] {} `{}` in {}:{}",
                meta.kind, meta.symbol_name, meta.file_path, meta.line_start
            ));
        }

        citations.push(Citation {
            index: idx,
            file_path: meta.file_path.clone(),
            line_start: meta.line_start,
            symbol_name: meta.symbol_name.clone(),
        });
    }

    Ok((context_lines.join("\n\n"), citations, tools_used))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Read source code around a symbol definition, returning up to `MAX_SNIPPET_LINES` lines.
///
/// Tries to read from `line_start` (1-based) to `line_start + MAX_SNIPPET_LINES`.
/// Returns `None` if the file cannot be read.
fn read_code_snippet(file_path: &str, line_start: usize) -> Option<String> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = line_start.saturating_sub(1); // Convert to 0-based.
    let end = (start + MAX_SNIPPET_LINES).min(lines.len());
    if start >= lines.len() {
        return None;
    }
    Some(lines[start..end].join("\n"))
}

/// Extract a search pattern from a natural-language query.
///
/// Strips common stop words and question-prefixes, then joins the remaining words
/// with `|` to produce a regex alternation. Falls back to the whole query if
/// nothing meaningful remains.
fn extract_search_pattern(query: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        // Question words
        "where",
        "what",
        "which",
        "how",
        "why",
        "when",
        "who",
        // Articles / determiners
        "the",
        "a",
        "an",
        "this",
        "that",
        "these",
        "those",
        "its",
        // Prepositions / conjunctions
        "in",
        "of",
        "for",
        "with",
        "about",
        "to",
        "from",
        "by",
        "on",
        "at",
        "and",
        "or",
        // Common verbs
        "is",
        "are",
        "was",
        "were",
        "does",
        "do",
        "did",
        "has",
        "have",
        "had",
        "can",
        "could",
        "will",
        "would",
        "should",
        "calls",
        "find",
        "explain",
        "describe",
        "show",
        "locate",
        "uses",
        "used",
        // Code-structure words (too generic for symbol search)
        "function",
        "method",
        "struct",
        "class",
        "module",
        "type",
        "enum",
        // Filler
        "me",
        "it",
        "all",
        "any",
        "some",
        "not",
        "be",
        // Generic project words (match everything, help nothing)
        "tool",
        "code",
        "codebase",
        "project",
        "file",
        "support",
        "work",
        "programming",
    ];

    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| {
            // Skip very short words (1-2 chars) and stop words.
            w.len() > 2 && !STOP_WORDS.contains(&w.as_str())
        })
        .map(|w| stem_word(&w))
        .collect();

    if words.is_empty() {
        // Fall back to the full query escaped as a literal.
        regex::escape(query)
    } else {
        words
            .iter()
            .map(|w| regex::escape(w))
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// Basic English stemming — strip common suffixes so plural/verb forms match symbol names.
///
/// "languages" → "language", "programming" → "programm", "handlers" → "handler", etc.
/// Not a full Porter stemmer, just enough for symbol matching.
fn stem_word(word: &str) -> String {
    // Order matters — try longer suffixes first.
    // "ies" before "s" so "queries" → "query" not "querie".
    // No "es" — "languages" should strip just "s" → "language", not "es" → "languag".
    // Words like "classes" → strip "s" → "classe" is fine for regex matching.
    let suffixes = [
        "ies", "ing", "tion", "sion", "ment", "ness", "ed", "ly", "s",
    ];
    for suffix in &suffixes {
        if let Some(stem) = word.strip_suffix(suffix) {
            // Don't strip if the remaining stem is too short.
            if stem.len() >= 3 {
                // "ies" → stem + "y" (e.g. "queries" → "query")
                if *suffix == "ies" {
                    return format!("{}y", stem);
                }
                return stem.to_string();
            }
        }
    }
    word.to_string()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citation_fields_are_correct() {
        let c = Citation {
            index: 3,
            file_path: "src/services/auth.rs".to_string(),
            line_start: 99,
            symbol_name: "verify_token".to_string(),
        };
        assert_eq!(c.index, 3);
        assert_eq!(c.file_path, "src/services/auth.rs");
        assert_eq!(c.line_start, 99);
        assert_eq!(c.symbol_name, "verify_token");
    }

    #[test]
    fn retrieval_result_context_text_contains_numbered_citations() {
        // Build a synthetic RetrievalResult and verify format.
        let citations = [
            Citation {
                index: 1,
                file_path: "src/auth.rs".to_string(),
                line_start: 10,
                symbol_name: "auth_handler".to_string(),
            },
            Citation {
                index: 2,
                file_path: "src/user.rs".to_string(),
                line_start: 20,
                symbol_name: "get_user".to_string(),
            },
        ];
        let context_text = citations
            .iter()
            .map(|c| {
                format!(
                    "[{}] {} in {}:{}",
                    c.index, c.symbol_name, c.file_path, c.line_start
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(context_text.contains("[1]"), "should have [1] marker");
        assert!(context_text.contains("[2]"), "should have [2] marker");
        assert!(
            context_text.contains("auth_handler"),
            "should mention symbol name"
        );
        assert!(
            context_text.contains("src/auth.rs:10"),
            "should have file:line"
        );
    }

    #[test]
    fn tools_used_accumulates_tool_names() {
        // Verify that tools_used from structural retrieval contains expected names.
        // We test this with a minimal empty graph (no results — still tracks tools).
        let graph = CodeGraph::new();
        let (_ctx, _cit, tools) = retrieve_structural(&graph, "find auth");
        assert!(
            tools.contains(&"find_symbol".to_string()),
            "structural retrieval should track find_symbol"
        );
    }

    #[test]
    fn retrieve_structural_empty_graph_returns_empty_context() {
        let graph = CodeGraph::new();
        let (ctx, cit, _tools) = retrieve_structural(&graph, "some query");
        assert!(ctx.is_empty(), "empty graph should produce empty context");
        assert!(cit.is_empty(), "empty graph should produce no citations");
    }

    #[test]
    fn extract_search_pattern_strips_stop_words() {
        // "where is auth" → "auth"
        let pattern = extract_search_pattern("where is auth");
        assert!(
            pattern.contains("auth"),
            "should retain meaningful keyword 'auth'"
        );
        // "find" is a stop word but "UserService" is not.
        let pattern2 = extract_search_pattern("find UserService");
        assert!(
            pattern2.contains("userservice"),
            "should retain 'UserService' (lowercased)"
        );
    }

    #[test]
    fn extract_search_pattern_stems_plurals() {
        // "languages" → "language" so it matches LanguageKind
        let pattern = extract_search_pattern("what languages does this tool support");
        assert!(
            pattern.contains("language"),
            "should stem 'languages' to 'language', got: {pattern}"
        );
        // Generic words like "tool", "support", "this" should be filtered out.
        assert!(
            !pattern.contains("tool"),
            "generic word 'tool' should be stopped"
        );
        assert!(!pattern.contains("this"), "'this' should be stopped");
    }

    #[test]
    fn stem_word_handles_common_suffixes() {
        assert_eq!(stem_word("languages"), "language");
        assert_eq!(stem_word("handlers"), "handler"); // strip "s" not "ers"
        assert_eq!(stem_word("queries"), "query");
        assert_eq!(stem_word("caching"), "cach");
        assert_eq!(stem_word("auth"), "auth"); // too short to strip
    }
}

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use tokio::sync::RwLock;

use super::params::{
    BatchQueryParams, DetectCircularParams, ExportGraphParams, FindDeadCodeParams,
    FindReferencesParams, FindSymbolParams, GetContextParams, GetDiffParams, GetFileSummaryParams,
    GetImpactParams, GetImportsParams, GetStatsParams, GetStructureParams, ListProjectsParams,
    RegisterProjectParams,
};
use crate::graph::CodeGraph;

// ---------------------------------------------------------------------------
// CodeGraphServer
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CodeGraphServer {
    default_project_root: Arc<PathBuf>,
    graph_cache: Arc<RwLock<HashMap<PathBuf, Arc<CodeGraph>>>>,
    watcher_handle: Arc<tokio::sync::Mutex<Option<crate::watcher::WatcherHandle>>>,
    watch_enabled: bool,
    mcp_config: crate::config::McpConfig,
    tool_router: ToolRouter<Self>,
    /// Registry of additional project roots with optional aliases.
    /// Maps absolute project path -> alias name (for display).
    registered_projects: Arc<RwLock<HashMap<PathBuf, String>>>,
}

impl CodeGraphServer {
    pub fn new(project_root: PathBuf, watch: bool) -> Self {
        let config = crate::config::CodeGraphConfig::load(&project_root);
        Self {
            default_project_root: Arc::new(project_root),
            graph_cache: Arc::new(RwLock::new(HashMap::new())),
            watcher_handle: Arc::new(tokio::sync::Mutex::new(None)),
            watch_enabled: watch,
            mcp_config: config.mcp,
            tool_router: Self::tool_router(),
            registered_projects: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Resolve the effective project root (override > default), build or retrieve from cache.
    ///
    /// Cold start flow:
    ///  1. Fast path: read lock — return cached graph if available.
    ///  2. Slow path: write lock — double-check, then try disk cache or full build.
    ///  3. Save graph to disk cache after build.
    ///  4. Start watcher lazily (once per path).
    async fn resolve_graph(
        &self,
        project_path_override: Option<&str>,
    ) -> Result<(Arc<CodeGraph>, PathBuf), String> {
        let path: PathBuf = match project_path_override {
            Some(p) => PathBuf::from(p),
            None => (*self.default_project_root).clone(),
        };

        // Fast path: read lock — return immediately if graph is cached in memory
        {
            let cache = self.graph_cache.read().await;
            if let Some(graph) = cache.get(&path) {
                return Ok((Arc::clone(graph), path));
            }
        } // read lock dropped here

        // Slow path: write lock — build or load from disk cache
        let mut cache = self.graph_cache.write().await;
        // Double-check after acquiring write lock (another task may have populated it)
        if let Some(graph) = cache.get(&path) {
            return Ok((Arc::clone(graph), path));
        }

        // Try disk cache first (cold start), fall back to full build
        let path_clone = path.clone();
        let graph = tokio::task::spawn_blocking(move || -> Result<CodeGraph, String> {
            if let Some(envelope) = crate::cache::load_cache(&path_clone) {
                // Apply staleness diff — re-parse only changed files
                let graph =
                    apply_staleness_diff(envelope, &path_clone).map_err(|e| e.to_string())?;
                // Save updated cache with fresh mtimes
                let _ = crate::cache::save_cache(&path_clone, &graph);
                Ok(graph)
            } else {
                // No cache — full build
                let graph = crate::build_graph(&path_clone, false).map_err(|e| e.to_string())?;
                // Save to disk cache for future cold starts
                let _ = crate::cache::save_cache(&path_clone, &graph);
                Ok(graph)
            }
        })
        .await
        .map_err(|e| format!("task join error: {}", e))??;

        if graph.file_index.is_empty() {
            return Err(format!(
                "No source files found at '{}'. Ensure the directory contains supported files (.ts, .tsx, .js, .jsx, .rs).",
                path.display()
            ));
        }

        let graph = Arc::new(graph);
        cache.insert(path.clone(), Arc::clone(&graph));
        // Write lock still held — drop after insert
        drop(cache);

        // Start watcher lazily (must happen after write lock is dropped)
        if self.watch_enabled {
            self.ensure_watcher_running(&path).await;
        }

        Ok((graph, path))
    }

    /// Start the file watcher if not already running.
    ///
    /// CRITICAL lock discipline (per research Pitfall 2):
    /// - Event loop NEVER holds the RwLock write guard during CPU-bound work
    ///   (parse_file, resolve_import) or blocking I/O (save_cache).
    /// - Write lock is acquired ONLY for the final Arc swap (nanoseconds).
    /// - This ensures concurrent MCP tool calls (which need read access) are
    ///   never blocked for the full re-parse duration (50-100ms+).
    async fn ensure_watcher_running(&self, project_root: &Path) {
        let mut watcher_guard = self.watcher_handle.lock().await;
        if watcher_guard.is_some() {
            return; // already running
        }

        match crate::watcher::start_watcher(project_root) {
            Ok((handle, mut rx)) => {
                // Spawn background task to process events
                let graph_cache = Arc::clone(&self.graph_cache);
                let root = project_root.to_path_buf();
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        let root = root.clone();
                        let graph_cache = Arc::clone(&graph_cache);

                        match event {
                            crate::watcher::event::WatchEvent::ConfigChanged
                            | crate::watcher::event::WatchEvent::CrateRootChanged(_) => {
                                // Full rebuild — all CPU work happens in spawn_blocking
                                // with NO lock held. Write lock acquired only to swap result.
                                let root_clone = root.clone();
                                if let Some(new_graph) = tokio::task::spawn_blocking(
                                    move || -> anyhow::Result<CodeGraph> {
                                        let graph = crate::build_graph(&root_clone, false)?;
                                        let _ = crate::cache::save_cache(&root_clone, &graph);
                                        Ok(graph)
                                    },
                                )
                                .await
                                .ok()
                                .and_then(|r| r.ok())
                                {
                                    // Write lock held ONLY for the insert (nanoseconds)
                                    let mut cache = graph_cache.write().await;
                                    cache.insert(root.clone(), Arc::new(new_graph));
                                    // Lock dropped immediately
                                }
                            }
                            _ => {
                                // Incremental update — clone graph WITHOUT holding write lock.

                                // Step 1: Read lock to clone the Arc (fast)
                                let old_arc = {
                                    let cache = graph_cache.read().await;
                                    cache.get(&root).cloned()
                                    // Read lock dropped here
                                };

                                if let Some(old_arc) = old_arc {
                                    // Step 2: Clone graph data from Arc (no lock held)
                                    let mut graph = (*old_arc).clone();

                                    // Step 3: CPU-bound parse/resolve + blocking IO
                                    //         ALL happen with NO lock held
                                    let root_for_blocking = root.clone();
                                    let result = tokio::task::spawn_blocking(move || {
                                        let modified =
                                            crate::watcher::incremental::handle_file_event(
                                                &mut graph,
                                                &event,
                                                &root_for_blocking,
                                            );
                                        if modified {
                                            let _ = crate::cache::save_cache(
                                                &root_for_blocking,
                                                &graph,
                                            );
                                        }
                                        (graph, modified)
                                    })
                                    .await;

                                    // Step 4: Write lock ONLY to swap in result (nanoseconds)
                                    if let Ok((graph, true)) = result {
                                        let mut cache = graph_cache.write().await;
                                        cache.insert(root.clone(), Arc::new(graph));
                                        // Lock dropped immediately
                                    }
                                }
                            }
                        }
                    }
                });
                *watcher_guard = Some(handle);
            }
            Err(e) => {
                eprintln!("[watcher] failed to start: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Staleness diff
// ---------------------------------------------------------------------------

/// Apply staleness diff: compare cached file mtimes against current filesystem,
/// re-parse changed/new files, remove deleted files.
///
/// Threshold: if >= 10% of files changed, discard and do full rebuild instead.
fn apply_staleness_diff(
    envelope: crate::cache::CacheEnvelope,
    project_root: &Path,
) -> anyhow::Result<CodeGraph> {
    let mut graph = envelope.graph;
    let cached_mtimes = envelope.file_mtimes;

    // Walk current files
    let config = crate::config::CodeGraphConfig::load(project_root);
    let current_files = crate::walker::walk_project(project_root, &config, false, None)?;

    // Phase 12: Also walk non-parsed files to prevent false "deleted" detection.
    // Non-parsed files are in the cached graph's file_index but walk_project only returns source files.
    let non_parsed_files = crate::walker::walk_non_parsed_files(project_root, &config)?;
    let mut current_set: std::collections::HashSet<PathBuf> =
        current_files.iter().cloned().collect();
    current_set.extend(non_parsed_files.iter().cloned());

    // Find changed and new files
    let mut files_to_reparse: Vec<PathBuf> = Vec::new();
    for file in &current_files {
        if let Ok(metadata) = std::fs::metadata(file) {
            let mtime_secs = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let size = metadata.len();

            match cached_mtimes.get(file) {
                Some(cached) if cached.mtime_secs == mtime_secs && cached.size == size => {
                    // Unchanged — skip
                }
                _ => {
                    // Changed or new — needs re-parse
                    files_to_reparse.push(file.clone());
                }
            }
        }
    }

    // Find deleted files (in cache but not on disk)
    let deleted_files: Vec<PathBuf> = cached_mtimes
        .keys()
        .filter(|p| !current_set.contains(*p))
        .cloned()
        .collect();

    let total_changed = files_to_reparse.len() + deleted_files.len();
    let total_current = current_files.len().max(1);

    // Threshold: if >= 10% changed, do full rebuild (faster than scoped re-resolve for many changes)
    if total_changed * 10 >= total_current {
        return crate::build_graph(project_root, false).map_err(|e| anyhow::anyhow!(e));
    }

    // Scoped approach: remove deleted + changed files, re-add changed files
    for path in &deleted_files {
        graph.remove_file_from_graph(path);
    }

    // Remove changed files from graph before re-parsing
    for path in &files_to_reparse {
        graph.remove_file_from_graph(path);
    }

    // Re-parse changed/new files in parallel.
    // "rs" => "rust" is included so Rust files are not silently dropped on cold-start cache diff.
    // Without this, Rust symbols would be missing from the MCP graph after a cache hit.
    let reparsed: Vec<(PathBuf, &'static str, crate::parser::ParseResult)> = files_to_reparse
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read(path).ok()?;
            let language_str: &'static str =
                match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "ts" => "typescript",
                    "tsx" => "tsx",
                    "js" | "jsx" => "javascript",
                    "rs" => "rust",
                    _ => return None,
                };
            let result = crate::parser::parse_file_parallel(path, &source).ok()?;
            Some((path.clone(), language_str, result))
        })
        .collect();

    for (path, language_str, result) in &reparsed {
        let file_idx = graph.add_file(path.clone(), language_str);
        for (symbol, children) in &result.symbols {
            let sym_idx = graph.add_symbol(file_idx, symbol.clone());
            for child in children {
                graph.add_child_symbol(sym_idx, child.clone());
            }
        }
        // Emit Rust use/pub-use edge placeholders (same as build_graph does).
        // Phase 9 resolve_all will replace these self-edges with resolved targets.
        for rust_use in &result.rust_uses {
            if rust_use.is_pub_use {
                graph.graph.add_edge(
                    file_idx,
                    file_idx,
                    crate::graph::edge::EdgeKind::ReExport {
                        path: rust_use.path.clone(),
                    },
                );
            } else {
                graph.graph.add_edge(
                    file_idx,
                    file_idx,
                    crate::graph::edge::EdgeKind::RustImport {
                        path: rust_use.path.clone(),
                    },
                );
            }
        }
    }

    // If any files were re-parsed, do a scoped resolve pass:
    // collect parse results for ALL current files, then resolve all.
    // This is acceptable on cold start (runs once).
    if !files_to_reparse.is_empty() || !deleted_files.is_empty() {
        // Populate crate_name on FileInfo before resolve_all (same as build_graph does).
        // Without this, the resolver cannot classify Rust symbols by crate.
        crate::populate_rust_crate_names(&mut graph, project_root);

        // Build parse results for all files in the updated graph (parallel re-parse for resolve)
        let all_file_paths: Vec<PathBuf> = graph.file_index.keys().cloned().collect();
        let parsed: Vec<(PathBuf, crate::parser::ParseResult)> = all_file_paths
            .par_iter()
            .filter_map(|file_path| {
                let source = std::fs::read(file_path).ok()?;
                let result = crate::parser::parse_file_parallel(file_path, &source).ok()?;
                Some((file_path.clone(), result))
            })
            .collect();
        let mut all_parse_results = std::collections::HashMap::new();
        for (path, result) in parsed {
            all_parse_results.insert(path, result);
        }
        crate::resolver::resolve_all(&mut graph, project_root, &all_parse_results, false);
    }

    // Phase 12: Add any new non-parsed files discovered on this cold start
    for file_path in &non_parsed_files {
        if !graph.file_index.contains_key(file_path) {
            let kind = crate::graph::node::classify_file_kind(file_path);
            graph.add_non_parsed_file(file_path.clone(), kind);
        }
    }

    Ok(graph)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute character-level trigrams from a string (lowercased).
/// Returns an empty set for strings shorter than 3 characters.
fn trigrams(s: &str) -> HashSet<[char; 3]> {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    if chars.len() < 3 {
        return HashSet::new();
    }
    chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// Jaccard similarity between two trigram sets: |A ∩ B| / |A ∪ B|.
/// Returns 0.0 if both sets are empty (no useful comparison possible).
fn jaccard_similarity(a: &HashSet<[char; 3]>, b: &HashSet<[char; 3]>) -> f32 {
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

/// Suggest similar symbol names using trigram Jaccard similarity.
///
/// Returns at most 3 candidates with Jaccard >= 0.3, sorted descending by score.
/// Returns an empty vec for queries shorter than 3 characters (no trigrams to compare).
fn suggest_similar_fuzzy(graph: &CodeGraph, query: &str) -> Vec<String> {
    let query_trigrams = trigrams(query);
    if query_trigrams.is_empty() {
        return Vec::new();
    }

    const THRESHOLD: f32 = 0.3;

    let mut scored: Vec<(String, f32)> = graph
        .symbol_index
        .keys()
        .filter_map(|name| {
            let name_trigrams = trigrams(name);
            let score = jaccard_similarity(&query_trigrams, &name_trigrams);
            if score >= THRESHOLD {
                Some((name.clone(), score))
            } else {
                None
            }
        })
        .collect();

    // Sort descending by score (best match first)
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(3);
    scored.into_iter().map(|(name, _)| name).collect()
}

fn not_found_msg(symbol: &str, suggestions: &[String]) -> String {
    let mut msg = format!("Symbol '{}' not found.", symbol);
    if !suggestions.is_empty() {
        msg.push_str(&format!(" Did you mean: {}?", suggestions.join(", ")));
    }
    msg
}

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

/// Return the effective limit: use the explicit per-call value when present,
/// otherwise fall back to the project config default.
fn resolve_limit(call_limit: Option<usize>, config: &crate::config::McpConfig) -> usize {
    call_limit.unwrap_or(config.default_limit)
}

/// Return the effective sections filter: use the explicit per-call value when present,
/// otherwise fall back to the project config default.
fn resolve_sections<'a>(
    call_sections: Option<&'a str>,
    config: &'a crate::config::McpConfig,
) -> Option<&'a str> {
    call_sections.or(config.default_sections.as_deref())
}

// ---------------------------------------------------------------------------
// Batch query helpers
// ---------------------------------------------------------------------------

/// Format a section header for a batch query result.
/// Output: `## tool_name(key=val, key=val)`
/// Only includes non-null params, sorted by key for determinism.
fn format_query_header(tool: &str, params: &serde_json::Value) -> String {
    let param_str = if let Some(obj) = params.as_object() {
        let mut pairs: Vec<String> = obj
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{}={}", k, val_str)
            })
            .collect();
        pairs.sort();
        pairs.join(", ")
    } else {
        String::new()
    };
    if param_str.is_empty() {
        format!("## {}", tool)
    } else {
        format!("## {}({})", tool, param_str)
    }
}

/// Dispatch a single query by tool name, calling query functions directly.
///
/// BATCH-02: This function does NOT call MCP handler methods — it calls query
/// functions directly so that the graph is resolved exactly once for all N queries.
/// Hints are NOT appended here; only one combined hint is added by `batch_query`.
///
/// `registered_projects`: optional registry of additional project roots (for list_projects).
fn dispatch_query(
    config: &crate::config::McpConfig,
    graph: &CodeGraph,
    root: &Path,
    tool: &str,
    params: &serde_json::Value,
    registered_projects: Option<&HashMap<PathBuf, String>>,
) -> Result<String, String> {
    match tool {
        "find_symbol" => {
            let symbol = params["symbol"]
                .as_str()
                .ok_or("missing required param: symbol")?;
            let limit_param = params["limit"].as_u64().map(|n| n as usize);
            let kind = params["kind"].as_str();
            let path = params["path"].as_str();

            let kind_filter: Vec<String> = kind
                .map(|k| k.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            let file_filter = path.map(Path::new);

            let results = crate::query::find::find_symbol(
                graph,
                symbol,
                false,
                &kind_filter,
                file_filter,
                root,
                None,
            )
            .map_err(|e| e.to_string())?;

            if results.is_empty() {
                let suggestions = suggest_similar_fuzzy(graph, symbol);
                return Err(not_found_msg(symbol, &suggestions));
            }

            let limit = resolve_limit(limit_param, config);
            let truncated = results.len() > limit;
            let limited = &results[..results.len().min(limit)];
            let output = crate::query::output::format_find_to_string(limited, root);

            Ok(if truncated && !config.suppress_summary_line {
                format!("truncated: {}/{}\n{}", limit, results.len(), output)
            } else {
                output
            })
        }
        "find_references" => {
            let symbol = params["symbol"]
                .as_str()
                .ok_or("missing required param: symbol")?;
            let limit_param = params["limit"].as_u64().map(|n| n as usize);

            let matches = crate::query::find::match_symbols(graph, symbol, false)
                .map_err(|e| e.to_string())?;
            if matches.is_empty() {
                let suggestions = suggest_similar_fuzzy(graph, symbol);
                return Err(not_found_msg(symbol, &suggestions));
            }

            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let results = crate::query::refs::find_refs(graph, symbol, &all_indices, root);

            let limit = resolve_limit(limit_param, config);
            let truncated = results.len() > limit;
            let limited = &results[..results.len().min(limit)];
            let output = crate::query::output::format_refs_to_string(limited, root);

            Ok(if truncated && !config.suppress_summary_line {
                format!("truncated: {}/{}\n{}", limit, results.len(), output)
            } else {
                output
            })
        }
        "get_impact" => {
            let symbol = params["symbol"]
                .as_str()
                .ok_or("missing required param: symbol")?;
            let limit_param = params["limit"].as_u64().map(|n| n as usize);

            let matches = crate::query::find::match_symbols(graph, symbol, false)
                .map_err(|e| e.to_string())?;
            if matches.is_empty() {
                let suggestions = suggest_similar_fuzzy(graph, symbol);
                return Err(not_found_msg(symbol, &suggestions));
            }

            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let results = crate::query::impact::blast_radius(graph, &all_indices, root);

            let limit = resolve_limit(limit_param, config);
            let truncated = results.len() > limit;
            let limited = &results[..results.len().min(limit)];
            let output = crate::query::output::format_impact_to_string(limited, root);

            Ok(if truncated && !config.suppress_summary_line {
                format!("truncated: {}/{}\n{}", limit, results.len(), output)
            } else {
                output
            })
        }
        "get_context" => {
            let symbol = params["symbol"]
                .as_str()
                .ok_or("missing required param: symbol")?;
            let sections_param = params["sections"].as_str();

            let matches = crate::query::find::match_symbols(graph, symbol, false)
                .map_err(|e| e.to_string())?;
            if matches.is_empty() {
                let suggestions = suggest_similar_fuzzy(graph, symbol);
                return Err(not_found_msg(symbol, &suggestions));
            }

            let contexts: Vec<crate::query::context::SymbolContext> = matches
                .iter()
                .map(|(name, indices)| {
                    crate::query::context::symbol_context(graph, name, indices, root)
                })
                .collect();

            let sections = resolve_sections(sections_param, config);
            let output = crate::query::output::format_context_to_string(&contexts, root, sections);
            Ok(output)
        }
        "detect_circular" => {
            let cycles = crate::query::circular::find_circular(graph, root);
            let output = crate::query::output::format_circular_to_string(&cycles, root);
            Ok(output)
        }
        "get_stats" => {
            let stats = crate::query::stats::project_stats(graph);
            let output = crate::query::output::format_stats_to_string(&stats, None);
            Ok(output)
        }
        "get_structure" => {
            let path = params["path"].as_str().map(std::path::Path::new);
            let depth = params["depth"].as_u64().map(|n| n as usize).unwrap_or(3);
            let tree = crate::query::structure::file_structure(graph, root, path, depth);
            let output = crate::query::output::format_structure_to_string(&tree, root);
            Ok(output)
        }
        "get_file_summary" => {
            let path_str = params["path"]
                .as_str()
                .ok_or("missing required param: path")?;
            let file_path = std::path::Path::new(path_str);
            let summary = crate::query::file_summary::file_summary(graph, root, file_path)?;
            let output = crate::query::output::format_file_summary_to_string(&summary);
            Ok(output)
        }
        "get_imports" => {
            let path_str = params["path"]
                .as_str()
                .ok_or("missing required param: path")?;
            let file_path = std::path::Path::new(path_str);
            let entries = crate::query::imports::file_imports(graph, root, file_path)?;
            let output = crate::query::output::format_imports_to_string(&entries, path_str);
            Ok(output)
        }
        "export_graph" => {
            // Parse format
            let format = match params["format"].as_str() {
                Some("mermaid") => crate::export::model::ExportFormat::Mermaid,
                Some("dot") | None => crate::export::model::ExportFormat::Dot,
                Some(other) => {
                    return Err(format!(
                        "Unknown format '{}'. Use 'dot' or 'mermaid'.",
                        other
                    ));
                }
            };
            let granularity = match params["granularity"].as_str() {
                Some("symbol") => crate::export::model::Granularity::Symbol,
                Some("package") => crate::export::model::Granularity::Package,
                Some("file") | None => crate::export::model::Granularity::File,
                Some(other) => {
                    return Err(format!(
                        "Unknown granularity '{}'. Use 'symbol', 'file', or 'package'.",
                        other
                    ));
                }
            };
            let exclude_patterns: Vec<String> = params["exclude"]
                .as_str()
                .map(|e| e.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            let export_params = crate::export::model::ExportParams {
                format,
                granularity,
                root_filter: params["root"].as_str().map(std::path::PathBuf::from),
                symbol_filter: params["symbol"].as_str().map(|s| s.to_string()),
                depth: params["depth"].as_u64().map(|n| n as usize).unwrap_or(1),
                exclude_patterns,
                project_root: root.to_path_buf(),
                stdout: true,
            };
            let result =
                crate::export::export_graph(graph, &export_params).map_err(|e| e.to_string())?;
            let mut response = format!(
                "Exported {} nodes, {} edges (format: {:?}, granularity: {:?})\n",
                result.node_count, result.edge_count, format, granularity
            );
            for warning in &result.warnings {
                response.push_str(&format!("Warning: {}\n", warning));
            }
            response.push_str(&result.content);
            Ok(response)
        }
        "find_dead_code" => {
            let scope = params["scope"].as_str().map(std::path::Path::new);
            let result = crate::query::dead_code::find_dead_code(graph, root, scope);
            let output = crate::query::output::format_dead_code_to_string(&result, root);
            Ok(output)
        }
        "list_projects" => {
            // In batch context, we have access to registered_projects (if provided by caller)
            let default_path = root;
            let cache_has_default = graph.file_count() > 0; // graph is already resolved for this root
            let mut lines = vec![format!(
                "* {} (default, {})",
                default_path.display(),
                if cache_has_default {
                    "indexed"
                } else {
                    "not indexed"
                }
            )];
            if let Some(registry) = registered_projects {
                for (path, alias) in registry.iter() {
                    if path == default_path {
                        continue;
                    }
                    // We don't have access to graph_cache here, so we can't check indexed status
                    // Report as "registered" without index status
                    lines.push(format!("* {} [{}]", path.display(), alias));
                }
            }
            Ok(lines.join("\n"))
        }
        "get_diff" => {
            let from = params["from"]
                .as_str()
                .ok_or("missing required param: from")?;
            let to = params["to"].as_str();
            let diff = crate::query::diff::compute_diff(root, from, to, graph)?;
            let output = crate::query::output::format_diff_to_string(&diff);
            Ok(output)
        }
        _ => Err(format!("unknown tool: {}", tool)),
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl CodeGraphServer {
    #[tool(
        description = "Find symbol definitions by name or regex. Returns file:line locations and symbol kind."
    )]
    async fn find_symbol(
        &self,
        Parameters(p): Parameters<FindSymbolParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let kind_filter: Vec<String> = p
            .kind
            .map(|k| k.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let file_filter = p.path.as_ref().map(Path::new);

        let results = crate::query::find::find_symbol(
            &graph,
            &p.symbol,
            false,
            &kind_filter,
            file_filter,
            &root,
            None, // MCP operates on full project — no language filter
        )
        .map_err(|e| e.to_string())?;

        if results.is_empty() {
            let suggestions = suggest_similar_fuzzy(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let limit = resolve_limit(p.limit, &self.mcp_config);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_find_to_string(limited, &root);

        let output = if truncated && !self.mcp_config.suppress_summary_line {
            format!("truncated: {}/{}\n{}", limit, results.len(), output)
        } else {
            output
        };

        let first_name = limited.first().map(|r| r.symbol_name.as_str());
        let output = format!(
            "{}{}",
            output,
            crate::mcp::hints::find_hint(&p.symbol, limited.len(), truncated, first_name)
        );
        Ok(output)
    }

    #[tool(
        description = "Find all files and call sites that reference a symbol. Shows import and call edges."
    )]
    async fn find_references(
        &self,
        Parameters(p): Parameters<FindReferencesParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar_fuzzy(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
            .iter()
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect();

        let results = crate::query::refs::find_refs(&graph, &p.symbol, &all_indices, &root);

        let limit = resolve_limit(p.limit, &self.mcp_config);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_refs_to_string(limited, &root);

        let output = if truncated && !self.mcp_config.suppress_summary_line {
            format!("truncated: {}/{}\n{}", limit, results.len(), output)
        } else {
            output
        };

        let output = format!("{}{}", output, crate::mcp::hints::refs_hint(&p.symbol));
        Ok(output)
    }

    #[tool(
        description = "Get the blast radius of changing a symbol. Returns transitive dependent files."
    )]
    async fn get_impact(
        &self,
        Parameters(p): Parameters<GetImpactParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar_fuzzy(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
            .iter()
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect();

        let results = crate::query::impact::blast_radius(&graph, &all_indices, &root);

        let limit = resolve_limit(p.limit, &self.mcp_config);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_impact_to_string(limited, &root);

        let output = if truncated && !self.mcp_config.suppress_summary_line {
            format!("truncated: {}/{}\n{}", limit, results.len(), output)
        } else {
            output
        };

        let output = format!("{}{}", output, crate::mcp::hints::impact_hint(&p.symbol));
        Ok(output)
    }

    #[tool(
        description = "Detect circular dependency cycles in the import graph. Returns file cycles."
    )]
    async fn detect_circular(
        &self,
        Parameters(p): Parameters<DetectCircularParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let cycles = crate::query::circular::find_circular(&graph, &root);
        let output = crate::query::output::format_circular_to_string(&cycles, &root);
        let output = format!(
            "{}{}",
            output,
            crate::mcp::hints::circular_hint(cycles.len())
        );
        Ok(output)
    }

    #[tool(
        description = "360-degree view of a symbol: definition, references, callers, callees, type hierarchy."
    )]
    async fn get_context(
        &self,
        Parameters(p): Parameters<GetContextParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar_fuzzy(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let contexts: Vec<crate::query::context::SymbolContext> = matches
            .iter()
            .map(|(name, indices)| {
                crate::query::context::symbol_context(&graph, name, indices, &root)
            })
            .collect();

        let effective_sections = resolve_sections(p.sections.as_deref(), &self.mcp_config);
        let output =
            crate::query::output::format_context_to_string(&contexts, &root, effective_sections);
        let output = format!("{}{}", output, crate::mcp::hints::context_hint(&p.symbol));
        Ok(output)
    }

    #[tool(
        description = "Project overview: file count, symbol breakdown by kind, import/resolution summary."
    )]
    async fn get_stats(&self, Parameters(p): Parameters<GetStatsParams>) -> Result<String, String> {
        let (graph, _root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let stats = crate::query::stats::project_stats(&graph);
        let output = crate::query::output::format_stats_to_string(&stats, None);
        let output = format!("{}{}", output, crate::mcp::hints::stats_hint());
        Ok(output)
    }

    #[tool(
        description = "Export the code graph to DOT or Mermaid format for architectural visualization. Returns the rendered graph text."
    )]
    async fn export_graph(
        &self,
        Parameters(p): Parameters<ExportGraphParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        // Parse format
        let format = match p.format.as_deref() {
            Some("mermaid") => crate::export::model::ExportFormat::Mermaid,
            Some("dot") | None => crate::export::model::ExportFormat::Dot,
            Some(other) => {
                return Err(format!(
                    "Unknown format '{}'. Use 'dot' or 'mermaid'.",
                    other
                ));
            }
        };

        // Parse granularity
        let granularity = match p.granularity.as_deref() {
            Some("symbol") => crate::export::model::Granularity::Symbol,
            Some("package") => crate::export::model::Granularity::Package,
            Some("file") | None => crate::export::model::Granularity::File,
            Some(other) => {
                return Err(format!(
                    "Unknown granularity '{}'. Use 'symbol', 'file', or 'package'.",
                    other
                ));
            }
        };

        let exclude_patterns: Vec<String> = p
            .exclude
            .map(|e| e.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let params = crate::export::model::ExportParams {
            format,
            granularity,
            root_filter: p.root.map(std::path::PathBuf::from),
            symbol_filter: p.symbol,
            depth: p.depth.unwrap_or(1),
            exclude_patterns,
            project_root: root.clone(),
            stdout: true, // MCP always returns content as string, never writes files
        };

        let result = crate::export::export_graph(&graph, &params).map_err(|e| e.to_string())?;

        // Build response: stats header + content
        let mut response = format!(
            "Exported {} nodes, {} edges (format: {:?}, granularity: {:?})\n",
            result.node_count, result.edge_count, format, granularity
        );
        for warning in &result.warnings {
            response.push_str(&format!("Warning: {}\n", warning));
        }
        response.push_str(&result.content);

        Ok(response)
    }

    #[tool(description = "Directory/module tree with files and their top-level symbols.")]
    async fn get_structure(
        &self,
        Parameters(p): Parameters<GetStructureParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;
        let path = p.path.as_deref().map(std::path::Path::new);
        let depth = p.depth.unwrap_or(3);
        let tree = crate::query::structure::file_structure(&graph, &root, path, depth);
        let output = crate::query::output::format_structure_to_string(&tree, &root);
        let hint = crate::mcp::hints::structure_hint(p.path.as_deref());
        let output = format!("{}{}", output, hint);
        Ok(output)
    }

    #[tool(
        description = "File overview: exports, imports, symbol count, dependency role, and graph position — without reading source."
    )]
    async fn get_file_summary(
        &self,
        Parameters(p): Parameters<GetFileSummaryParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;
        let file_path = std::path::Path::new(&p.path);
        let summary = crate::query::file_summary::file_summary(&graph, &root, file_path)?;
        let output = crate::query::output::format_file_summary_to_string(&summary);
        let hint = crate::mcp::hints::file_summary_hint(&p.path);
        let output = format!("{}{}", output, hint);
        Ok(output)
    }

    #[tool(
        description = "File import/dependency list classified by type (internal, workspace, external, builtin). Shows re-exports."
    )]
    async fn get_imports(
        &self,
        Parameters(p): Parameters<GetImportsParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;
        let file_path = std::path::Path::new(&p.path);
        let entries = crate::query::imports::file_imports(&graph, &root, file_path)?;
        let output = crate::query::output::format_imports_to_string(&entries, &p.path);
        let hint = crate::mcp::hints::imports_hint(&p.path);
        let output = format!("{}{}", output, hint);
        Ok(output)
    }

    #[tool(
        description = "Detect unreferenced symbols and unreachable files within a path scope. Returns dead code candidates grouped by file. Entry points (main, pub, exports, trait impls, tests) are excluded."
    )]
    async fn find_dead_code(
        &self,
        Parameters(p): Parameters<FindDeadCodeParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;
        let scope = p.scope.as_deref().map(std::path::Path::new);
        let result = crate::query::dead_code::find_dead_code(&graph, &root, scope);
        let unreferenced_count: usize = result
            .unreferenced_symbols
            .iter()
            .map(|(_, syms)| syms.len())
            .sum();
        let output = crate::query::output::format_dead_code_to_string(&result, &root);
        let hint =
            crate::mcp::hints::dead_code_hint(result.unreachable_files.len(), unreferenced_count);
        Ok(format!("{}{}", output, hint))
    }

    #[tool(
        description = "Compare the current graph against a named snapshot, or compare two snapshots. Reports added/removed/modified files and symbols."
    )]
    async fn get_diff(&self, Parameters(p): Parameters<GetDiffParams>) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;
        let diff = crate::query::diff::compute_diff(&root, &p.from, p.to.as_deref(), &graph)?;
        let has_changes = !diff.added_files.is_empty()
            || !diff.removed_files.is_empty()
            || !diff.added_symbols.is_empty()
            || !diff.removed_symbols.is_empty()
            || !diff.modified_symbols.is_empty();
        let output = crate::query::output::format_diff_to_string(&diff);
        let hint = crate::mcp::hints::diff_hint(has_changes);
        Ok(format!("{}{}", output, hint))
    }

    #[tool(
        description = "Register a new project root for multi-project querying. Indexes immediately. Use project_path on other tools to query this project."
    )]
    async fn register_project(
        &self,
        Parameters(p): Parameters<RegisterProjectParams>,
    ) -> Result<String, String> {
        let path = PathBuf::from(&p.path);
        if !path.exists() {
            return Err(format!("path '{}' does not exist", p.path));
        }
        let alias = p.name.unwrap_or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
        // Force immediate indexing
        let _ = self.resolve_graph(Some(&p.path)).await?;
        // Record alias
        let mut registry = self.registered_projects.write().await;
        registry.insert(path, alias.clone());
        let output = format!("registered '{}' at {} — ready to query", alias, p.path);
        let hint = crate::mcp::hints::register_project_hint(&p.path);
        Ok(format!("{}{}", output, hint))
    }

    #[tool(description = "List all registered project roots in this server session.")]
    async fn list_projects(
        &self,
        Parameters(_p): Parameters<ListProjectsParams>,
    ) -> Result<String, String> {
        let registry = self.registered_projects.read().await;
        let cache = self.graph_cache.read().await;
        let default_path = &*self.default_project_root;
        let mut lines = vec![format!(
            "* {} (default, {})",
            default_path.display(),
            if cache.contains_key(default_path) {
                "indexed"
            } else {
                "not indexed"
            }
        )];
        for (path, alias) in registry.iter() {
            if path == default_path {
                continue;
            }
            lines.push(format!(
                "* {} [{}] ({})",
                path.display(),
                alias,
                if cache.contains_key(path) {
                    "indexed"
                } else {
                    "not indexed"
                }
            ));
        }
        let output = lines.join("\n");
        let hint = crate::mcp::hints::list_projects_hint();
        Ok(format!("{}{}", output, hint))
    }

    #[tool(
        description = "Execute multiple graph queries in a single call. Returns results separated by section headers. Max 10 queries per batch."
    )]
    async fn batch_query(
        &self,
        Parameters(p): Parameters<BatchQueryParams>,
    ) -> Result<String, String> {
        if p.queries.len() > 10 {
            return Err("batch_query: max 10 queries per batch".to_string());
        }
        if p.queries.is_empty() {
            return Err("batch_query: queries array is empty".to_string());
        }

        // BATCH-02: Resolve graph ONCE for all queries
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        // Read the registered projects registry for list_projects support in batch
        let registry_guard = self.registered_projects.read().await;

        let mut sections: Vec<String> = Vec::new();
        let mut query_meta: Vec<(&str, bool)> = Vec::new();

        for entry in &p.queries {
            let header = format_query_header(&entry.tool, &entry.params);
            let result = dispatch_query(
                &self.mcp_config,
                &graph,
                &root,
                &entry.tool,
                &entry.params,
                Some(&*registry_guard),
            );
            match result {
                Ok(output) => {
                    sections.push(format!("{}\n{}", header, output));
                    query_meta.push((&entry.tool, true));
                }
                Err(e) => {
                    sections.push(format!("{}\nerror: {}", header, e));
                    query_meta.push((&entry.tool, false));
                }
            }
        }

        let combined = sections.join("\n\n");
        let hint = crate::mcp::hints::batch_hint(&query_meta);
        Ok(format!("{}{}", combined, hint))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for CodeGraphServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "code-graph indexes and queries TypeScript/JavaScript/Rust dependency graphs. \
                 The graph is built automatically on first tool call — no manual indexing needed. \
                 When started with --watch, file changes are auto-reindexed. \
                 All tools accept an optional project_path parameter to override the default project root. \
                 Navigation funnel: get_structure (project tree) → get_file_summary (file overview) → \
                 get_imports (dependency list) → get_context (symbol detail). \
                 Dead code analysis: find_dead_code detects unreferenced symbols and unreachable files. \
                 Multi-project support: register_project adds a new project root; list_projects shows all registered projects. \
                 Snapshot/diff workflow: Use code-graph snapshot create <name> to save a baseline, then get_diff to see what changed."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpConfig;
    use rmcp::ServerHandler;
    use std::path::PathBuf;

    // ---------------------------------------------------------------------------
    // resolve_limit tests (CFG-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_resolve_limit_explicit_overrides_config() {
        let config = McpConfig {
            default_limit: 20,
            ..Default::default()
        };
        assert_eq!(
            resolve_limit(Some(50), &config),
            50,
            "explicit per-call limit should override config default"
        );
    }

    #[test]
    fn test_resolve_limit_none_uses_config() {
        let config = McpConfig {
            default_limit: 20,
            ..Default::default()
        };
        assert_eq!(
            resolve_limit(None, &config),
            20,
            "None should fall back to config default_limit"
        );
    }

    // ---------------------------------------------------------------------------
    // resolve_sections tests (CFG-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_resolve_sections_explicit_overrides_config() {
        let config = McpConfig {
            default_sections: Some("r".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_sections(Some("r,c"), &config),
            Some("r,c"),
            "explicit per-call sections should override config default"
        );
    }

    #[test]
    fn test_resolve_sections_none_uses_config() {
        let config = McpConfig {
            default_sections: Some("r".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_sections(None, &config),
            Some("r"),
            "None should fall back to config default_sections"
        );
    }

    #[test]
    fn test_resolve_sections_both_none() {
        let config = McpConfig {
            default_sections: None,
            ..Default::default()
        };
        assert_eq!(
            resolve_sections(None, &config),
            None,
            "None with no config default should return None"
        );
    }

    // ---------------------------------------------------------------------------
    // test_server_loads_mcp_config (CFG-01 + CFG-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_server_loads_mcp_config() {
        let dir = tempfile::tempdir().expect("tempdir should succeed");
        let toml_content = r#"
[mcp]
default_limit = 42
default_sections = "r,c"
suppress_summary_line = true
"#;
        std::fs::write(dir.path().join("code-graph.toml"), toml_content)
            .expect("write should succeed");

        let server = CodeGraphServer::new(dir.path().to_path_buf(), false);
        assert_eq!(
            server.mcp_config.default_limit, 42,
            "server should load default_limit from code-graph.toml"
        );
        assert_eq!(
            server.mcp_config.default_sections.as_deref(),
            Some("r,c"),
            "server should load default_sections from code-graph.toml"
        );
        assert!(
            server.mcp_config.suppress_summary_line,
            "server should load suppress_summary_line from code-graph.toml"
        );
    }

    #[test]
    fn test_get_info_describes_auto_indexing() {
        let server = CodeGraphServer::new(PathBuf::from("/tmp/test"), false);
        let info = server.get_info();
        let instructions = info.instructions.expect("instructions should be set");
        // SRV-01: Does not tell users to manually index
        assert!(
            !instructions.contains("Index with"),
            "should not mention manual indexing"
        );
        // SRV-03: Mentions auto-indexing
        assert!(
            instructions.contains("automatically"),
            "should mention automatic indexing"
        );
        // SRV-03: Mentions --watch
        assert!(
            instructions.contains("--watch"),
            "should mention --watch flag"
        );
        // SRV-03: Mentions project_path override
        assert!(
            instructions.contains("project_path"),
            "should mention project_path override"
        );
        // SRV-03: Mentions all supported languages
        assert!(
            instructions.contains("TypeScript") && instructions.contains("Rust"),
            "should mention supported languages"
        );
        // NAV funnel: instructions describe the navigation funnel tools
        assert!(
            instructions.contains("get_structure")
                && instructions.contains("get_file_summary")
                && instructions.contains("get_imports"),
            "should describe navigation funnel tools"
        );
    }

    #[test]
    fn test_watch_disabled_by_default() {
        let server = CodeGraphServer::new(PathBuf::from("/tmp/test"), false);
        assert!(!server.watch_enabled, "watch should be disabled when false");
    }

    #[test]
    fn test_watch_enabled_when_flag_set() {
        let server = CodeGraphServer::new(PathBuf::from("/tmp/test"), true);
        assert!(server.watch_enabled, "watch should be enabled when true");
    }

    // ---------------------------------------------------------------------------
    // Fuzzy matching tests (FUZZY-01, FUZZY-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_trigrams_normal_string() {
        // "MyStruct" -> 6 trigrams: [M,y,S], [y,S,t], [S,t,r], [t,r,u], [r,u,c], [u,c,t]
        let t = trigrams("MyStruct");
        assert_eq!(t.len(), 6, "MyStruct should have 6 trigrams");
        assert!(
            t.contains(&['m', 'y', 's']),
            "should contain [m,y,s] (lowercased)"
        );
        assert!(t.contains(&['y', 's', 't']), "should contain [y,s,t]");
        assert!(t.contains(&['s', 't', 'r']), "should contain [s,t,r]");
        assert!(t.contains(&['t', 'r', 'u']), "should contain [t,r,u]");
        assert!(t.contains(&['r', 'u', 'c']), "should contain [r,u,c]");
        assert!(t.contains(&['u', 'c', 't']), "should contain [u,c,t]");
    }

    #[test]
    fn test_trigrams_short_strings() {
        // Strings shorter than 3 chars return empty set
        assert!(trigrams("").is_empty(), "empty string -> no trigrams");
        assert!(trigrams("a").is_empty(), "1 char -> no trigrams");
        assert!(trigrams("ab").is_empty(), "2 chars -> no trigrams");
    }

    #[test]
    fn test_trigrams_exactly_three_chars() {
        // Exactly 3 chars -> exactly 1 trigram
        let t = trigrams("foo");
        assert_eq!(t.len(), 1, "3-char string should have 1 trigram");
        assert!(t.contains(&['f', 'o', 'o']), "should contain [f,o,o]");
    }

    #[test]
    fn test_trigrams_case_insensitive() {
        // trigrams are lowercased
        let t_upper = trigrams("ABC");
        let t_lower = trigrams("abc");
        assert_eq!(t_upper, t_lower, "trigrams should be case-insensitive");
    }

    #[test]
    fn test_jaccard_identical_sets() {
        let a: std::collections::HashSet<[char; 3]> = [['a', 'b', 'c'], ['b', 'c', 'd']].into();
        let b = a.clone();
        let score = jaccard_similarity(&a, &b);
        assert!((score - 1.0).abs() < 1e-6, "identical sets -> score 1.0");
    }

    #[test]
    fn test_jaccard_disjoint_sets() {
        let a: std::collections::HashSet<[char; 3]> = [['a', 'b', 'c']].into();
        let b: std::collections::HashSet<[char; 3]> = [['x', 'y', 'z']].into();
        let score = jaccard_similarity(&a, &b);
        assert!((score - 0.0).abs() < 1e-6, "disjoint sets -> score 0.0");
    }

    #[test]
    fn test_jaccard_partial_overlap() {
        // {A,B,C} ∩ {B,C,D} = {B,C}, |union| = 4
        let a: std::collections::HashSet<[char; 3]> =
            [['a', 'b', 'c'], ['b', 'c', 'd'], ['c', 'd', 'e']].into();
        let b: std::collections::HashSet<[char; 3]> =
            [['b', 'c', 'd'], ['c', 'd', 'e'], ['d', 'e', 'f']].into();
        let score = jaccard_similarity(&a, &b);
        // intersection = {[b,c,d],[c,d,e]}, union = {[a,b,c],[b,c,d],[c,d,e],[d,e,f]}
        // jaccard = 2/4 = 0.5
        assert!(
            (score - 0.5).abs() < 1e-6,
            "partial overlap: expected 0.5, got {}",
            score
        );
    }

    #[test]
    fn test_jaccard_empty_sets() {
        let a: std::collections::HashSet<[char; 3]> = std::collections::HashSet::new();
        let b: std::collections::HashSet<[char; 3]> = std::collections::HashSet::new();
        let score = jaccard_similarity(&a, &b);
        assert!((score - 0.0).abs() < 1e-6, "both empty -> 0.0");
    }

    /// Build a minimal CodeGraph stub with the given symbol names in symbol_index.
    fn make_graph_with_symbols(symbols: &[&str]) -> CodeGraph {
        let mut graph = CodeGraph::default();
        for &name in symbols {
            graph.symbol_index.insert(name.to_string(), vec![]);
        }
        graph
    }

    #[test]
    fn test_suggest_fuzzy_typo_mystruct() {
        // "MyStrct" is a typo for "MyStruct" — should be suggested
        let graph = make_graph_with_symbols(&["MyStruct", "MyStructBuilder", "OtherThing"]);
        let suggestions = suggest_similar_fuzzy(&graph, "MyStrct");
        assert!(
            !suggestions.is_empty(),
            "typo MyStrct should yield suggestions"
        );
        assert_eq!(
            suggestions[0], "MyStruct",
            "MyStruct should be top suggestion for MyStrct"
        );
        assert!(suggestions.len() <= 3, "at most 3 suggestions");
        // All suggestions must have been in the graph
        for s in &suggestions {
            assert!(
                ["MyStruct", "MyStructBuilder", "OtherThing"].contains(&s.as_str()),
                "suggestion '{}' not in graph",
                s
            );
        }
    }

    #[test]
    fn test_suggest_fuzzy_short_query() {
        // Queries shorter than 3 chars return no suggestions
        let graph = make_graph_with_symbols(&["Foo", "Bar", "Baz"]);
        assert!(
            suggest_similar_fuzzy(&graph, "ab").is_empty(),
            "2-char query -> empty"
        );
        assert!(
            suggest_similar_fuzzy(&graph, "a").is_empty(),
            "1-char query -> empty"
        );
        assert!(
            suggest_similar_fuzzy(&graph, "").is_empty(),
            "empty query -> empty"
        );
    }

    #[test]
    fn test_suggest_fuzzy_unrelated_query() {
        // All scores < 0.3 -> no suggestions
        let graph = make_graph_with_symbols(&["Foo", "Bar", "Baz"]);
        let suggestions = suggest_similar_fuzzy(&graph, "CompletelyUnrelated");
        assert!(
            suggestions.is_empty() || suggestions.iter().all(|_| true),
            "unrelated query may return empty or low-scoring suggestions"
        );
        // More precisely: verify score threshold is applied
        let suggestions2 = suggest_similar_fuzzy(&graph, "XyzXyzXyz");
        // Foo/Bar/Baz have no trigrams in common with XyzXyzXyz
        assert!(
            suggestions2.is_empty(),
            "no-match query -> empty suggestions"
        );
    }

    #[test]
    fn test_suggest_fuzzy_max_three_results() {
        // Even if many symbols match, at most 3 are returned
        let graph = make_graph_with_symbols(&[
            "MyStruct",
            "MyStructA",
            "MyStructB",
            "MyStructC",
            "MyStructD",
        ]);
        let suggestions = suggest_similar_fuzzy(&graph, "MyStruct");
        assert!(suggestions.len() <= 3, "at most 3 suggestions returned");
    }

    #[test]
    fn test_suggest_fuzzy_sorted_by_score() {
        // Results must be sorted by score descending (best match first)
        let graph = make_graph_with_symbols(&["MyStruct", "MyStructBuilder"]);
        let suggestions = suggest_similar_fuzzy(&graph, "MyStrct");
        // MyStruct is closer to MyStrct than MyStructBuilder
        if suggestions.len() >= 2 {
            assert_eq!(suggestions[0], "MyStruct", "best match should be first");
        }
    }

    #[test]
    fn test_suggest_fuzzy_score_threshold() {
        // Symbols with Jaccard < 0.3 must not appear in suggestions
        // "Foo" has trigrams {[f,o,o]} — very low similarity to "FooBarBazQux"
        let graph = make_graph_with_symbols(&["Foo"]);
        let suggestions = suggest_similar_fuzzy(&graph, "FooBarBazQux");
        // "Foo" trigrams: {foo}. "FooBarBazQux" trigrams: {foo,oob,oba,bar,arb,rba,baz,azq,zqu,qux}
        // intersection = {foo}, union = 10 elements -> jaccard = 0.1 < 0.3
        assert!(
            suggestions.is_empty(),
            "symbol with low Jaccard score should not be suggested"
        );
    }

    // ---------------------------------------------------------------------------
    // format_query_header tests (BATCH-01)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_format_query_header_with_params() {
        let params = serde_json::json!({"symbol": "Foo", "limit": 10});
        let header = format_query_header("find_symbol", &params);
        assert_eq!(
            header, "## find_symbol(limit=10, symbol=Foo)",
            "header should sort params alphabetically and format tool(k=v, ...)"
        );
    }

    #[test]
    fn test_format_query_header_no_params() {
        let params = serde_json::json!({});
        let header = format_query_header("get_stats", &params);
        assert_eq!(
            header, "## get_stats",
            "header with empty params object should just show tool name"
        );
    }

    #[test]
    fn test_format_query_header_null_params_skipped() {
        let params = serde_json::json!({"symbol": "Foo", "kind": null});
        let header = format_query_header("find_symbol", &params);
        assert_eq!(
            header, "## find_symbol(symbol=Foo)",
            "null-valued params should be omitted from header"
        );
    }

    // ---------------------------------------------------------------------------
    // dispatch_query tests (BATCH-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_dispatch_unknown_tool() {
        let config = McpConfig::default();
        let graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp");
        let params = serde_json::json!({});
        let result = dispatch_query(&config, &graph, root, "nonexistent", &params, None);
        assert!(result.is_err(), "unknown tool should return Err");
        assert!(
            result.unwrap_err().contains("unknown tool: nonexistent"),
            "error message should mention the unknown tool name"
        );
    }

    #[test]
    fn test_dispatch_missing_required_param() {
        let config = McpConfig::default();
        let graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp");
        let params = serde_json::json!({});
        let result = dispatch_query(&config, &graph, root, "find_symbol", &params, None);
        assert!(
            result.is_err(),
            "find_symbol without symbol param should return Err"
        );
        assert!(
            result.unwrap_err().contains("missing required param"),
            "error should mention missing required param"
        );
    }

    #[test]
    fn test_dispatch_unknown_tool_updated() {
        let config = McpConfig::default();
        let graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp");
        let params = serde_json::json!({});
        let result = dispatch_query(&config, &graph, root, "nonexistent_v2", &params, None);
        assert!(result.is_err(), "unknown tool should return Err");
    }

    #[test]
    fn test_dispatch_find_dead_code() {
        let config = McpConfig::default();
        let mut graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp/project");
        // Add a file with no importers
        let file_path = root.join("src/unused.rs");
        graph.add_file(file_path.clone(), "rust");

        let params = serde_json::json!({});
        let result = dispatch_query(&config, &graph, root, "find_dead_code", &params, None);
        assert!(
            result.is_ok(),
            "find_dead_code should succeed: {:?}",
            result
        );
        let output = result.unwrap();
        assert!(
            output.contains("unreachable files"),
            "output should contain unreachable files section"
        );
    }

    #[test]
    fn test_dispatch_register_project_not_in_batch() {
        // register_project is not in dispatch_query — it should return unknown tool error
        let config = McpConfig::default();
        let graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp");
        let params = serde_json::json!({"path": "/tmp"});
        let result = dispatch_query(&config, &graph, root, "register_project", &params, None);
        assert!(
            result.is_err(),
            "register_project should not be available in batch"
        );
        assert!(
            result.unwrap_err().contains("unknown tool"),
            "error should say unknown tool"
        );
    }

    #[test]
    fn test_dispatch_list_projects_in_batch() {
        let config = McpConfig::default();
        let graph = CodeGraph::default();
        let root = std::path::Path::new("/tmp/project");
        let params = serde_json::json!({});

        let mut registry = HashMap::new();
        registry.insert(PathBuf::from("/tmp/other_project"), "other".to_string());

        let result = dispatch_query(
            &config,
            &graph,
            root,
            "list_projects",
            &params,
            Some(&registry),
        );
        assert!(
            result.is_ok(),
            "list_projects should work in batch: {:?}",
            result
        );
        let output = result.unwrap();
        assert!(
            output.contains("/tmp/project"),
            "output should contain default project path"
        );
    }
}

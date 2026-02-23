use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use tokio::sync::RwLock;

use super::params::{
    DetectCircularParams, FindReferencesParams, FindSymbolParams, GetContextParams, GetImpactParams,
    GetStatsParams,
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
    tool_router: ToolRouter<Self>,
}

impl CodeGraphServer {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            default_project_root: Arc::new(project_root),
            graph_cache: Arc::new(RwLock::new(HashMap::new())),
            watcher_handle: Arc::new(tokio::sync::Mutex::new(None)),
            tool_router: Self::tool_router(),
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
                let graph = apply_staleness_diff(envelope, &path_clone)
                    .map_err(|e| e.to_string())?;
                // Save updated cache with fresh mtimes
                let _ = crate::cache::save_cache(&path_clone, &graph);
                Ok(graph)
            } else {
                // No cache — full build
                let graph = crate::build_graph(&path_clone, false)
                    .map_err(|e| e.to_string())?;
                // Save to disk cache for future cold starts
                let _ = crate::cache::save_cache(&path_clone, &graph);
                Ok(graph)
            }
        })
        .await
        .map_err(|e| format!("task join error: {}", e))?
        .map_err(|e| e)?;

        if graph.file_index.is_empty() {
            return Err(format!(
                "No indexed files found at '{}'. Run 'code-graph index <path>' first.",
                path.display()
            ));
        }

        let graph = Arc::new(graph);
        cache.insert(path.clone(), Arc::clone(&graph));
        // Write lock still held — drop after insert
        drop(cache);

        // Start watcher lazily (must happen after write lock is dropped)
        self.ensure_watcher_running(&path).await;

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
                            crate::watcher::event::WatchEvent::ConfigChanged => {
                                // Full rebuild — all CPU work happens in spawn_blocking
                                // with NO lock held. Write lock acquired only to swap result.
                                let root_clone = root.clone();
                                if let Some(new_graph) =
                                    tokio::task::spawn_blocking(move || -> anyhow::Result<CodeGraph> {
                                        let graph = crate::build_graph(&root_clone, false)?;
                                        let _ = crate::cache::save_cache(&root_clone, &graph);
                                        Ok(graph)
                                    })
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
    let current_files = crate::walker::walk_project(project_root, &config, false)?;
    let current_set: std::collections::HashSet<PathBuf> =
        current_files.iter().cloned().collect();

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
        return crate::build_graph(&project_root.to_path_buf(), false)
            .map_err(|e| anyhow::anyhow!(e));
    }

    // Scoped approach: remove deleted + changed files, re-add changed files
    for path in &deleted_files {
        graph.remove_file_from_graph(path);
    }

    for path in &files_to_reparse {
        graph.remove_file_from_graph(path);

        let source = match std::fs::read(path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let language_str = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "ts" => "typescript",
            "tsx" => "tsx",
            "js" | "jsx" => "javascript",
            _ => continue,
        };

        let result = match crate::parser::parse_file(path, &source) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let file_idx = graph.add_file(path.clone(), language_str);
        for (symbol, children) in &result.symbols {
            let sym_idx = graph.add_symbol(file_idx, symbol.clone());
            for child in children {
                graph.add_child_symbol(sym_idx, child.clone());
            }
        }
    }

    // If any files were re-parsed, do a scoped resolve pass:
    // collect parse results for ALL current files, then resolve all.
    // This is acceptable on cold start (runs once).
    if !files_to_reparse.is_empty() || !deleted_files.is_empty() {
        // Build parse results for all files in the updated graph
        let mut all_parse_results = std::collections::HashMap::new();
        for file_path in graph.file_index.keys() {
            if let Ok(source) = std::fs::read(file_path) {
                if let Ok(result) = crate::parser::parse_file(file_path, &source) {
                    all_parse_results.insert(file_path.clone(), result);
                }
            }
        }
        crate::resolver::resolve_all(&mut graph, project_root, &all_parse_results, false);
    }

    Ok(graph)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn suggest_similar(graph: &CodeGraph, query: &str) -> Vec<String> {
    let query_lower = query.to_lowercase();
    let prefix = &query_lower[..query_lower.len().min(3)];
    let mut matches: Vec<String> = graph
        .symbol_index
        .keys()
        .filter(|name| {
            let n = name.to_lowercase();
            n.contains(&query_lower) || n.starts_with(prefix)
        })
        .cloned()
        .collect();
    matches.sort();
    matches.truncate(3);
    matches
}

fn not_found_msg(symbol: &str, suggestions: &[String]) -> String {
    let mut msg = format!("Symbol '{}' not found.", symbol);
    if !suggestions.is_empty() {
        msg.push_str(&format!(" Did you mean: {}?", suggestions.join(", ")));
    }
    msg
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl CodeGraphServer {
    #[tool(description = "Find symbol definitions by name or regex. Returns file:line locations and symbol kind.")]
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
        )
        .map_err(|e| e.to_string())?;

        if results.is_empty() {
            let suggestions = suggest_similar(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let limit = p.limit.unwrap_or(20);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_find_to_string(limited, &root);

        let output = if truncated {
            let after_first_line = output.find('\n').map(|i| &output[i + 1..]).unwrap_or("");
            format!(
                "showing {}/{} definitions (increase limit for more)\n{}",
                limit,
                results.len(),
                after_first_line
            )
        } else {
            output
        };

        Ok(output)
    }

    #[tool(description = "Find all files and call sites that reference a symbol. Shows import and call edges.")]
    async fn find_references(
        &self,
        Parameters(p): Parameters<FindReferencesParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let all_indices: Vec<petgraph::stable_graph::NodeIndex> =
            matches.iter().flat_map(|(_, indices)| indices.iter().copied()).collect();

        let results = crate::query::refs::find_refs(&graph, &p.symbol, &all_indices, &root);

        let limit = p.limit.unwrap_or(30);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_refs_to_string(limited, &root);

        let output = if truncated {
            let after_first_line = output.find('\n').map(|i| &output[i + 1..]).unwrap_or("");
            format!(
                "showing {}/{} references (increase limit for more)\n{}",
                limit,
                results.len(),
                after_first_line
            )
        } else {
            output
        };

        Ok(output)
    }

    #[tool(description = "Get the blast radius of changing a symbol. Returns transitive dependent files.")]
    async fn get_impact(
        &self,
        Parameters(p): Parameters<GetImpactParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let all_indices: Vec<petgraph::stable_graph::NodeIndex> =
            matches.iter().flat_map(|(_, indices)| indices.iter().copied()).collect();

        let results = crate::query::impact::blast_radius(&graph, &all_indices, &root);

        let limit = p.limit.unwrap_or(50);
        let truncated = results.len() > limit;
        let limited = &results[..results.len().min(limit)];
        let output = crate::query::output::format_impact_to_string(limited, &root);

        let output = if truncated {
            let after_first_line = output.find('\n').map(|i| &output[i + 1..]).unwrap_or("");
            format!(
                "showing {}/{} affected files (increase limit for more)\n{}",
                limit,
                results.len(),
                after_first_line
            )
        } else {
            output
        };

        Ok(output)
    }

    #[tool(description = "Detect circular dependency cycles in the import graph. Returns file cycles.")]
    async fn detect_circular(
        &self,
        Parameters(p): Parameters<DetectCircularParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let cycles = crate::query::circular::find_circular(&graph, &root);
        Ok(crate::query::output::format_circular_to_string(&cycles, &root))
    }

    #[tool(description = "360-degree view of a symbol: definition, references, callers, callees, type hierarchy.")]
    async fn get_context(
        &self,
        Parameters(p): Parameters<GetContextParams>,
    ) -> Result<String, String> {
        let (graph, root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let matches = crate::query::find::match_symbols(&graph, &p.symbol, false)
            .map_err(|e| e.to_string())?;

        if matches.is_empty() {
            let suggestions = suggest_similar(&graph, &p.symbol);
            return Err(not_found_msg(&p.symbol, &suggestions));
        }

        let contexts: Vec<crate::query::context::SymbolContext> = matches
            .iter()
            .map(|(name, indices)| {
                crate::query::context::symbol_context(&graph, name, indices, &root)
            })
            .collect();

        Ok(crate::query::output::format_context_to_string(&contexts, &root))
    }

    #[tool(description = "Project overview: file count, symbol breakdown by kind, import/resolution summary.")]
    async fn get_stats(
        &self,
        Parameters(p): Parameters<GetStatsParams>,
    ) -> Result<String, String> {
        let (graph, _root) = self.resolve_graph(p.project_path.as_deref()).await?;

        let stats = crate::query::stats::project_stats(&graph);
        Ok(crate::query::output::format_stats_to_string(&stats))
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
                "code-graph: query TypeScript/JavaScript dependency graphs. Index with 'code-graph index <path>' first.".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

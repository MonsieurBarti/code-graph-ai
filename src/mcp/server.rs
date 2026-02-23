use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use tokio::sync::Mutex;

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
    graph_cache: Arc<Mutex<HashMap<PathBuf, Arc<CodeGraph>>>>,
    tool_router: ToolRouter<Self>,
}

impl CodeGraphServer {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            default_project_root: Arc::new(project_root),
            graph_cache: Arc::new(Mutex::new(HashMap::new())),
            tool_router: Self::tool_router(),
        }
    }

    /// Resolve the effective project root (override > default), build or retrieve from cache.
    async fn resolve_graph(
        &self,
        project_path_override: Option<&str>,
    ) -> Result<(Arc<CodeGraph>, PathBuf), String> {
        let path: PathBuf = match project_path_override {
            Some(p) => PathBuf::from(p),
            None => (*self.default_project_root).clone(),
        };

        let mut cache = self.graph_cache.lock().await;
        if let Some(graph) = cache.get(&path) {
            return Ok((Arc::clone(graph), path));
        }

        // Build graph (CPU-bound, run in blocking thread pool)
        let path_clone = path.clone();
        let graph = tokio::task::spawn_blocking(move || {
            crate::build_graph(&path_clone, false)
        })
        .await
        .map_err(|e| format!("task join error: {}", e))?
        .map_err(|e| e.to_string())?;

        if graph.file_index.is_empty() {
            return Err(format!(
                "No indexed files found at '{}'. Run 'code-graph index <path>' first.",
                path.display()
            ));
        }

        let graph = Arc::new(graph);
        cache.insert(path.clone(), Arc::clone(&graph));
        Ok((graph, path))
    }
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

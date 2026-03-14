use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use super::envelope::CacheEnvelope;
use crate::graph::CodeGraph;

/// Apply staleness diff: compare cached file mtimes against current filesystem,
/// re-parse changed/new files, remove deleted files.
///
/// Threshold: if >= 10% of files changed, discard and do full rebuild instead.
pub fn apply_staleness_diff(
    envelope: CacheEnvelope,
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
    let mut current_set: HashSet<PathBuf> = current_files.iter().cloned().collect();
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
                    // Unchanged -- skip
                }
                _ => {
                    // Changed or new -- needs re-parse
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

    // Threshold: if >= 10% changed, do full rebuild (faster than scoped re-resolve for many changes).
    // NOTE: build_graph blocks the calling thread for the full duration of the rebuild.
    // Async callers should use spawn_blocking or equivalent.
    if total_changed * 10 >= total_current {
        return crate::build_graph(project_root, false);
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
    // Without this, Rust symbols would be missing from the graph after a cache hit.
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
                    "py" => "python",
                    "go" => "go",
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

    // If any files were re-parsed, do a scoped resolve pass.
    // Reuse already-reparsed results from the earlier parallel parse, and only re-parse
    // unchanged files that are still needed for resolution (avoids full re-read from disk).
    if !files_to_reparse.is_empty() || !deleted_files.is_empty() {
        // Populate crate_name on FileInfo before resolve_all (same as build_graph does).
        // Without this, the resolver cannot classify Rust symbols by crate.
        crate::populate_rust_crate_names(&mut graph, project_root);

        // Seed with already-reparsed results (avoids re-reading changed files from disk).
        let mut all_parse_results: HashMap<PathBuf, crate::parser::ParseResult> = HashMap::new();
        for (path, _language_str, result) in reparsed {
            all_parse_results.insert(path, result);
        }

        // Only re-parse unchanged files that were not already parsed above.
        let unchanged_paths: Vec<PathBuf> = graph
            .file_index
            .keys()
            .filter(|p| !all_parse_results.contains_key(*p))
            .cloned()
            .collect();
        let unchanged_parsed: Vec<(PathBuf, crate::parser::ParseResult)> = unchanged_paths
            .par_iter()
            .filter_map(|file_path| {
                let source = std::fs::read(file_path).ok()?;
                let result = crate::parser::parse_file_parallel(file_path, &source).ok()?;
                Some((file_path.clone(), result))
            })
            .collect();
        for (path, result) in unchanged_parsed {
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

    // Phase 25: Enrich decorator frameworks and add HasDecorator self-edges after partial re-parse.
    // Only run when files were actually changed or deleted to avoid unnecessary full-graph scans.
    if !files_to_reparse.is_empty() || !deleted_files.is_empty() {
        crate::query::decorators::enrich_decorator_frameworks(&mut graph);
        crate::query::decorators::add_has_decorator_edges(&mut graph);
    }

    Ok(graph)
}

/// Load a cached graph with staleness diff, or fall back to a full build.
///
/// This is the primary entry point for query subcommands:
/// 1. Try `load_cache()` -- if hit, apply staleness diff via `apply_staleness_diff()`.
/// 2. If cache miss or version mismatch, call `build_graph()` for a full rebuild.
/// 3. Save the resulting graph to cache.
///
/// The `verbose` flag is forwarded to `build_graph()` when a full rebuild is needed.
pub fn load_or_build(project_root: &Path, verbose: bool) -> anyhow::Result<CodeGraph> {
    let graph = match super::load_cache(project_root) {
        Some(envelope) => {
            if verbose {
                eprintln!("[cache] hit -- applying staleness diff...");
            }
            apply_staleness_diff(envelope, project_root)?
        }
        None => {
            if verbose {
                eprintln!("[cache] miss -- full rebuild...");
            }
            crate::build_graph(project_root, verbose)?
        }
    };

    // Save cache after building.
    if let Err(e) = super::save_cache(project_root, &graph) {
        if verbose {
            eprintln!("[cache] save failed: {}", e);
        }
    }

    Ok(graph)
}

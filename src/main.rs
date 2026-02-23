mod cache;
mod cli;
mod config;
mod graph;
mod mcp;
mod output;
mod parser;
mod query;
mod resolver;
mod walker;
mod watcher;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use rayon::prelude::*;

use cli::{Cli, Commands};
use config::CodeGraphConfig;
use graph::{CodeGraph, node::SymbolKind};
use output::{IndexStats, print_summary};
use parser::ParseResult;
use parser::imports::ImportKind;
use walker::walk_project;

/// Build the code graph for a project at `path` by walking, parsing, and resolving all files.
///
/// This is the shared pipeline used by all query subcommands. The Index command has its own
/// inline copy so it can also compute detailed stats without a second pass.
pub(crate) fn build_graph(path: &Path, verbose: bool) -> Result<CodeGraph> {
    let config = CodeGraphConfig::load(path);
    let files = walk_project(path, &config, verbose)?;

    // Phase 1: Parse all files in parallel (CPU-bound — rayon par_iter).
    let raw_results: Vec<(PathBuf, &'static str, ParseResult)> = files
        .par_iter()
        .filter_map(|file_path| {
            let source = std::fs::read(file_path).ok()?;
            let language_str: &'static str =
                match file_path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "ts" => "typescript",
                    "tsx" => "tsx",
                    "js" | "jsx" => "javascript",
                    _ => return None,
                };
            let result = parser::parse_file_parallel(file_path, &source).ok()?;
            Some((file_path.clone(), language_str, result))
        })
        .collect();

    // Phase 2: Insert into graph sequentially (petgraph is not Send).
    let mut graph = CodeGraph::new();
    let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

    for (file_path, language_str, result) in raw_results {
        let file_idx = graph.add_file(file_path.clone(), language_str);

        for (symbol, children) in &result.symbols {
            let sym_idx = graph.add_symbol(file_idx, symbol.clone());
            for child in children {
                graph.add_child_symbol(sym_idx, child.clone());
            }
        }

        if verbose {
            eprintln!(
                "  {} symbols, {} imports, {} exports from {}",
                result.symbols.len(),
                result.imports.len(),
                result.exports.len(),
                file_path.display()
            );
        }

        parse_results.insert(file_path, result);
    }

    resolver::resolve_all(&mut graph, path, &parse_results, verbose);

    Ok(graph)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index {
            path,
            verbose,
            json,
        } => {
            // 1. Load config (always succeeds — defaults when file is absent).
            let config = CodeGraphConfig::load(&path);

            // 2. Start timer.
            let start = std::time::Instant::now();

            // 3. Walk files.
            let files = walk_project(&path, &config, verbose)?;

            // 4. Create graph.
            let mut graph = CodeGraph::new();

            // Import/export counts (accumulated across all files).
            let mut total_imports: usize = 0;
            let mut total_exports: usize = 0;
            let mut esm_imports: usize = 0;
            let mut cjs_imports: usize = 0;
            let mut dynamic_imports: usize = 0;

            // Parse results map — retained for the resolution step.
            let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

            // 5. Parse all files in parallel (rayon par_iter).
            let raw_results: Vec<(PathBuf, &'static str, ParseResult)> = files
                .par_iter()
                .filter_map(|file_path| {
                    let source = std::fs::read(file_path).ok()?;
                    let language_str: &'static str =
                        match file_path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                            "ts" => "typescript",
                            "tsx" => "tsx",
                            "js" | "jsx" => "javascript",
                            _ => return None,
                        };
                    let result = parser::parse_file_parallel(file_path, &source).ok()?;
                    Some((file_path.clone(), language_str, result))
                })
                .collect();

            // skipped = files that couldn't be read or parsed (filter_map silently drops them)
            let skipped = files.len() - raw_results.len();

            // 6. Insert into graph sequentially + accumulate stats.
            for (file_path, language_str, result) in raw_results {
                // Add file node to graph.
                let file_idx = graph.add_file(file_path.clone(), language_str);

                // Add symbols (parent + child).
                for (symbol, children) in &result.symbols {
                    let sym_idx = graph.add_symbol(file_idx, symbol.clone());
                    for child in children {
                        graph.add_child_symbol(sym_idx, child.clone());
                    }
                }

                // Accumulate import/export counts.
                total_imports += result.imports.len();
                total_exports += result.exports.len();
                for imp in &result.imports {
                    match imp.kind {
                        ImportKind::Esm => esm_imports += 1,
                        ImportKind::Cjs => cjs_imports += 1,
                        ImportKind::DynamicImport => dynamic_imports += 1,
                    }
                }

                if verbose {
                    eprintln!(
                        "  {} symbols, {} imports, {} exports from {}",
                        result.symbols.len(),
                        result.imports.len(),
                        result.exports.len(),
                        file_path.display()
                    );
                }

                // Store parse result for the resolution pass.
                parse_results.insert(file_path, result);
            }

            // 7. Resolve imports, barrel chains, and symbol relationships.
            let resolve_stats = resolver::resolve_all(&mut graph, &path, &parse_results, verbose);

            if verbose {
                eprintln!(
                    "  Resolution: {} resolved, {} external, {} unresolved, {} builtins",
                    resolve_stats.resolved,
                    resolve_stats.external,
                    resolve_stats.unresolved,
                    resolve_stats.builtin,
                );
                eprintln!(
                    "  Relationships: {} edges added",
                    resolve_stats.relationships_added
                );
            }

            // 8. Compute stats from graph.
            let elapsed_secs = start.elapsed().as_secs_f64();
            let breakdown: HashMap<SymbolKind, usize> = graph.symbols_by_kind();

            let stats = IndexStats {
                file_count: graph.file_count(),
                functions: *breakdown.get(&SymbolKind::Function).unwrap_or(&0),
                classes: *breakdown.get(&SymbolKind::Class).unwrap_or(&0),
                interfaces: *breakdown.get(&SymbolKind::Interface).unwrap_or(&0),
                type_aliases: *breakdown.get(&SymbolKind::TypeAlias).unwrap_or(&0),
                enums: *breakdown.get(&SymbolKind::Enum).unwrap_or(&0),
                variables: *breakdown.get(&SymbolKind::Variable).unwrap_or(&0),
                components: *breakdown.get(&SymbolKind::Component).unwrap_or(&0),
                methods: *breakdown.get(&SymbolKind::Method).unwrap_or(&0),
                properties: *breakdown.get(&SymbolKind::Property).unwrap_or(&0),
                imports: total_imports,
                esm_imports,
                cjs_imports,
                dynamic_imports,
                exports: total_exports,
                skipped,
                elapsed_secs,
                resolved_imports: resolve_stats.resolved,
                unresolved_imports: resolve_stats.unresolved,
                external_packages: resolve_stats.external,
                builtin_modules: resolve_stats.builtin,
                relationship_edges: resolve_stats.relationships_added,
            };

            // 9. Print summary.
            print_summary(&stats, json);

            // 10. Save graph to disk cache for fast cold starts.
            if let Err(e) = cache::save_cache(&path, &graph)
                && verbose
            {
                eprintln!("  Cache save failed: {}", e);
            }
        }

        Commands::Find {
            path,
            symbol,
            case_insensitive,
            kind,
            file,
            format,
        } => {
            // Validate regex FIRST before the expensive index pipeline (Research Pitfall 4).
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let graph = build_graph(&path, false)?;
            let results = query::find::find_symbol(
                &graph,
                &symbol,
                case_insensitive,
                &kind,
                file.as_deref(),
                &path,
            )?;

            if results.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            query::output::format_find_results(&results, &format, &path);
        }

        Commands::Stats { path, format } => {
            let graph = build_graph(&path, false)?;
            let stats = query::stats::project_stats(&graph);
            query::output::format_stats(&stats, &format);
        }

        Commands::Refs {
            path,
            symbol,
            case_insensitive,
            kind: _,
            file: _,
            format,
        } => {
            // Validate regex FIRST before the expensive index pipeline.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            // Collect all matched NodeIndices.
            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let results = query::refs::find_refs(&graph, &symbol, &all_indices, &path);

            if results.is_empty() {
                eprintln!("no references to '{}' found", symbol);
            } else {
                query::output::format_refs_results(&results, &format, &path);
            }
        }

        Commands::Impact {
            path,
            symbol,
            case_insensitive,
            tree,
            format,
        } => {
            // Validate regex FIRST.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let results = query::impact::blast_radius(&graph, &all_indices, &path);
            query::output::format_impact_results(&results, &format, &path, tree);
        }

        Commands::Circular { path, format } => {
            let graph = build_graph(&path, false)?;
            let cycles = query::circular::find_circular(&graph, &path);

            if cycles.is_empty() {
                println!("no circular dependencies found");
            } else {
                query::output::format_circular_results(&cycles, &format, &path);
            }
        }

        Commands::Context {
            path,
            symbol,
            case_insensitive,
            format,
        } => {
            // Validate regex FIRST before the expensive index pipeline.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            // Build one SymbolContext per matched symbol name.
            let results: Vec<query::context::SymbolContext> = matches
                .iter()
                .map(|(name, indices)| query::context::symbol_context(&graph, name, indices, &path))
                .collect();

            query::output::format_context_results(&results, &format, &path, &symbol);
        }

        Commands::Mcp { path } => {
            let project_root = path.unwrap_or_else(|| {
                std::env::current_dir().expect("cannot determine current directory")
            });
            mcp::run(project_root).await?;
        }

        Commands::Watch { path } => {
            eprintln!("Indexing {}...", path.display());
            let mut graph = build_graph(&path, false)?;
            eprintln!(
                "Indexed {} files, {} symbols. Starting watcher...",
                graph.file_count(),
                graph.symbol_count()
            );

            // Save initial cache
            if let Err(e) = cache::save_cache(&path, &graph) {
                eprintln!("[cache] failed to save: {}", e);
            }

            // Start watcher
            let (handle, mut rx) = watcher::start_watcher(&path)
                .map_err(|e| anyhow::anyhow!("failed to start watcher: {}", e))?;

            // Keep handle alive — dropping it stops the watcher
            let _handle = handle;

            eprintln!("Watching for changes... (press Ctrl+C to stop)");

            // Process events — terminal status output goes to stderr (Phase 1 convention)
            while let Some(event) = rx.recv().await {
                match &event {
                    watcher::event::WatchEvent::Modified(p) => {
                        let start = std::time::Instant::now();
                        watcher::incremental::handle_file_event(&mut graph, &event, &path);
                        let elapsed = start.elapsed();
                        eprintln!(
                            "[watch] modified: {} ({:.1}ms, {} files, {} symbols)",
                            p.strip_prefix(&path).unwrap_or(p).display(),
                            elapsed.as_secs_f64() * 1000.0,
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                    watcher::event::WatchEvent::Deleted(p) => {
                        watcher::incremental::handle_file_event(&mut graph, &event, &path);
                        eprintln!(
                            "[watch] deleted: {} ({} files, {} symbols)",
                            p.strip_prefix(&path).unwrap_or(p).display(),
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                    watcher::event::WatchEvent::ConfigChanged => {
                        eprintln!("[watch] config changed — full re-index...");
                        let start = std::time::Instant::now();
                        graph = build_graph(&path, false)?;
                        let elapsed = start.elapsed();
                        eprintln!(
                            "[watch] re-indexed in {:.1}ms ({} files, {} symbols)",
                            elapsed.as_secs_f64() * 1000.0,
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                }
            }
        }
    }

    Ok(())
}

mod cli;
mod config;
mod graph;
mod output;
mod parser;
mod walker;

use std::collections::HashMap;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};
use config::CodeGraphConfig;
use graph::{CodeGraph, node::SymbolKind};
use output::{IndexStats, print_summary};
use walker::walk_project;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, verbose, json } => {
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
            let mut skipped: usize = 0;

            // 5. Parse each file (serial — parallel is Phase 6).
            for file_path in &files {
                // Read source bytes.
                let source = match std::fs::read(file_path) {
                    Ok(s) => s,
                    Err(e) => {
                        if verbose {
                            eprintln!("skip: {}: {}", file_path.display(), e);
                        }
                        skipped += 1;
                        continue;
                    }
                };

                // Determine language string for graph node.
                let language_str = match file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                {
                    "ts" => "typescript",
                    "tsx" => "tsx",
                    "js" | "jsx" => "javascript",
                    _ => "unknown",
                };

                // Parse: tree-sitter + symbol/import/export extraction.
                let result = match parser::parse_file(file_path, &source) {
                    Ok(r) => r,
                    Err(e) => {
                        if verbose {
                            eprintln!("skip: {}: {}", file_path.display(), e);
                        }
                        skipped += 1;
                        continue;
                    }
                };

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

                if verbose {
                    eprintln!(
                        "  {} symbols, {} imports, {} exports from {}",
                        result.symbols.len(),
                        result.imports.len(),
                        result.exports.len(),
                        file_path.display()
                    );
                }
            }

            // 6. Compute stats from graph.
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
                exports: total_exports,
                skipped,
                elapsed_secs,
            };

            // 7. Print summary.
            print_summary(&stats, json);
        }
    }

    Ok(())
}

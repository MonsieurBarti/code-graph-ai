use std::io::IsTerminal;
use std::path::Path;

use crate::cli::OutputFormat;
use crate::query::find::FindResult;
use crate::query::find::kind_to_str;
use crate::query::stats::ProjectStats;

/// Format and print find results to stdout according to the selected output format.
pub fn format_find_results(results: &[FindResult], format: &OutputFormat, project_root: &Path) {
    match format {
        OutputFormat::Compact => {
            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                println!(
                    "def {} {}:{} {}",
                    r.symbol_name,
                    rel.display(),
                    r.line,
                    kind_to_str(&r.kind)
                );
            }
            println!("{} definitions found", results.len());
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();

            // Column widths: auto-sized to data.
            let name_w = results
                .iter()
                .map(|r| r.symbol_name.len())
                .max()
                .unwrap_or(6)
                .max(6);
            let file_w = results
                .iter()
                .map(|r| {
                    r.file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path)
                        .to_string_lossy()
                        .len()
                })
                .max()
                .unwrap_or(4)
                .max(4);

            if use_color {
                println!(
                    "\x1b[1m{:<name_w$}  {:<file_w$}  {:>4}  {}\x1b[0m",
                    "SYMBOL",
                    "FILE",
                    "LINE",
                    "KIND",
                    name_w = name_w,
                    file_w = file_w,
                );
            } else {
                println!(
                    "{:<name_w$}  {:<file_w$}  {:>4}  {}",
                    "SYMBOL",
                    "FILE",
                    "LINE",
                    "KIND",
                    name_w = name_w,
                    file_w = file_w,
                );
            }
            println!("{}", "-".repeat(name_w + file_w + 14));

            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                println!(
                    "{:<name_w$}  {:<file_w$}  {:>4}  {}",
                    r.symbol_name,
                    rel.display(),
                    r.line,
                    kind_to_str(&r.kind),
                    name_w = name_w,
                    file_w = file_w,
                );
            }
        }

        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    serde_json::json!({
                        "name": r.symbol_name,
                        "kind": kind_to_str(&r.kind),
                        "file": rel.to_string_lossy(),
                        "line": r.line,
                        "col": r.col,
                        "exported": r.is_exported,
                        "default": r.is_default,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_results).unwrap_or_default()
            );
        }
    }
}

/// Format and print project stats to stdout according to the selected output format.
pub fn format_stats(stats: &ProjectStats, format: &OutputFormat) {
    match format {
        OutputFormat::Compact => {
            println!("files {}", stats.file_count);
            println!("symbols {}", stats.symbol_count);
            println!(
                "functions {} classes {} interfaces {} types {} enums {} variables {} components {} methods {} properties {}",
                stats.functions,
                stats.classes,
                stats.interfaces,
                stats.type_aliases,
                stats.enums,
                stats.variables,
                stats.components,
                stats.methods,
                stats.properties,
            );
            println!(
                "imports {} external {} unresolved {}",
                stats.import_edges, stats.external_packages, stats.unresolved_imports,
            );
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();
            let header = |s: &str| {
                if use_color {
                    format!("\x1b[1m{s}\x1b[0m")
                } else {
                    s.to_string()
                }
            };

            println!("{}", header("=== Project Overview ==="));
            println!("Files:    {}", stats.file_count);
            println!("Symbols:  {}", stats.symbol_count);
            println!();
            println!("{}", header("--- Symbol Breakdown ---"));
            println!("  Functions:   {}", stats.functions);
            println!("  Classes:     {}", stats.classes);
            println!("  Interfaces:  {}", stats.interfaces);
            println!("  Type Aliases:{}", stats.type_aliases);
            println!("  Enums:       {}", stats.enums);
            println!("  Variables:   {}", stats.variables);
            println!("  Components:  {}", stats.components);
            println!("  Methods:     {}", stats.methods);
            println!("  Properties:  {}", stats.properties);
            println!();
            println!("{}", header("--- Import Summary ---"));
            println!("  Resolved imports:  {}", stats.import_edges);
            println!("  External packages: {}", stats.external_packages);
            println!("  Unresolved:        {}", stats.unresolved_imports);
        }

        OutputFormat::Json => {
            let json = serde_json::json!({
                "file_count": stats.file_count,
                "symbol_count": stats.symbol_count,
                "functions": stats.functions,
                "classes": stats.classes,
                "interfaces": stats.interfaces,
                "type_aliases": stats.type_aliases,
                "enums": stats.enums,
                "variables": stats.variables,
                "components": stats.components,
                "methods": stats.methods,
                "properties": stats.properties,
                "import_edges": stats.import_edges,
                "external_packages": stats.external_packages,
                "unresolved_imports": stats.unresolved_imports,
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
        }
    }
}

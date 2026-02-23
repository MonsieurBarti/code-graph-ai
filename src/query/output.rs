use std::io::IsTerminal;
use std::path::Path;

use crate::cli::OutputFormat;
use crate::query::circular::CircularDep;
use crate::query::context::SymbolContext;
use crate::query::find::FindResult;
use crate::query::find::kind_to_str;
use crate::query::impact::ImpactResult;
use crate::query::refs::{RefKind, RefResult};
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
                    "\x1b[1m{:<name_w$}  {:<file_w$}  {:>4}  KIND\x1b[0m",
                    "SYMBOL",
                    "FILE",
                    "LINE",
                    name_w = name_w,
                    file_w = file_w,
                );
            } else {
                println!(
                    "{:<name_w$}  {:<file_w$}  {:>4}  KIND",
                    "SYMBOL",
                    "FILE",
                    "LINE",
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
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Refs output
// ---------------------------------------------------------------------------

/// Format and print reference results to stdout.
pub fn format_refs_results(results: &[RefResult], format: &OutputFormat, project_root: &Path) {
    match format {
        OutputFormat::Compact => {
            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                match r.ref_kind {
                    RefKind::Import => {
                        println!("ref {} import", rel.display());
                    }
                    RefKind::Call => {
                        let caller = r.symbol_name.as_deref().unwrap_or("?");
                        let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                        println!("ref {}:{} call {}", rel.display(), line, caller);
                    }
                }
            }
            println!("{} references found", results.len());
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();

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
            let caller_w = results
                .iter()
                .map(|r| r.symbol_name.as_deref().unwrap_or("").len())
                .max()
                .unwrap_or(6)
                .max(6);

            if use_color {
                println!(
                    "\x1b[1m{:<file_w$}  {:<6}  {:<caller_w$}  {:>6}\x1b[0m",
                    "FILE",
                    "TYPE",
                    "CALLER",
                    "LINE",
                    file_w = file_w,
                    caller_w = caller_w,
                );
            } else {
                println!(
                    "{:<file_w$}  {:<6}  {:<caller_w$}  {:>6}",
                    "FILE",
                    "TYPE",
                    "CALLER",
                    "LINE",
                    file_w = file_w,
                    caller_w = caller_w,
                );
            }
            println!("{}", "-".repeat(file_w + caller_w + 20));

            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                let kind_str = match r.ref_kind {
                    RefKind::Import => "import",
                    RefKind::Call => "call",
                };
                let caller = r.symbol_name.as_deref().unwrap_or("");
                let line_str = r.line.map_or_else(|| "-".to_string(), |l| l.to_string());
                println!(
                    "{:<file_w$}  {:<6}  {:<caller_w$}  {:>6}",
                    rel.display(),
                    kind_str,
                    caller,
                    line_str,
                    file_w = file_w,
                    caller_w = caller_w,
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
                    let kind_str = match r.ref_kind {
                        RefKind::Import => "import",
                        RefKind::Call => "call",
                    };
                    serde_json::json!({
                        "file": rel.to_string_lossy(),
                        "kind": kind_str,
                        "caller": r.symbol_name,
                        "line": r.line,
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

// ---------------------------------------------------------------------------
// Impact output
// ---------------------------------------------------------------------------

/// Format and print impact (blast radius) results to stdout.
///
/// `tree_mode`: when true, use 2-space indentation per depth level.
pub fn format_impact_results(
    results: &[ImpactResult],
    format: &OutputFormat,
    project_root: &Path,
    tree_mode: bool,
) {
    match format {
        OutputFormat::Compact => {
            if tree_mode {
                for r in results {
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    let indent = "  ".repeat(r.depth.saturating_sub(1));
                    println!("{}impact {}", indent, rel.display());
                }
            } else {
                for r in results {
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    println!("impact {}", rel.display());
                }
            }
            println!("{} files affected", results.len());
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();

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
                    "\x1b[1m{:>5}  {:<file_w$}\x1b[0m",
                    "DEPTH",
                    "FILE",
                    file_w = file_w,
                );
            } else {
                println!("{:>5}  {:<file_w$}", "DEPTH", "FILE", file_w = file_w,);
            }
            println!("{}", "-".repeat(file_w + 8));

            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                println!(
                    "{:>5}  {:<file_w$}",
                    r.depth,
                    rel.display(),
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
                        "file": rel.to_string_lossy(),
                        "depth": r.depth,
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

// ---------------------------------------------------------------------------
// Circular output
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Context output
// ---------------------------------------------------------------------------

/// Format and print symbol context results to stdout.
///
/// Compact format is token-optimized: prefixed lines with relative paths, no decoration.
/// Sections only appear when non-empty.
pub fn format_context_results(
    contexts: &[SymbolContext],
    format: &OutputFormat,
    project_root: &Path,
    _symbol_name: &str,
) {
    match format {
        OutputFormat::Compact => {
            for ctx in contexts {
                println!("symbol {}", ctx.symbol_name);

                for def in &ctx.definitions {
                    let rel = def
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&def.file_path);
                    println!(
                        "def {}:{} {}",
                        rel.display(),
                        def.line,
                        kind_to_str(&def.kind)
                    );
                }

                for r in &ctx.references {
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    match r.ref_kind {
                        RefKind::Import => {
                            println!("ref {} import", rel.display());
                        }
                        RefKind::Call => {
                            let caller = r.symbol_name.as_deref().unwrap_or("?");
                            let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                            println!("ref {}:{} call {}", rel.display(), line, caller);
                        }
                    }
                }

                for callee in &ctx.callees {
                    let rel = callee
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&callee.file_path);
                    println!(
                        "calls {} {}:{}",
                        callee.symbol_name,
                        rel.display(),
                        callee.line
                    );
                }

                for caller in &ctx.callers {
                    let rel = caller
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&caller.file_path);
                    println!(
                        "called-by {} {}:{}",
                        caller.symbol_name,
                        rel.display(),
                        caller.line
                    );
                }

                for ext in &ctx.extends {
                    let rel = ext
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&ext.file_path);
                    println!("extends {} {}:{}", ext.symbol_name, rel.display(), ext.line);
                }

                for imp in &ctx.implements {
                    let rel = imp
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&imp.file_path);
                    println!(
                        "implements {} {}:{}",
                        imp.symbol_name,
                        rel.display(),
                        imp.line
                    );
                }

                for ext_by in &ctx.extended_by {
                    let rel = ext_by
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&ext_by.file_path);
                    println!(
                        "extended-by {} {}:{}",
                        ext_by.symbol_name,
                        rel.display(),
                        ext_by.line
                    );
                }

                for impl_by in &ctx.implemented_by {
                    let rel = impl_by
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&impl_by.file_path);
                    println!(
                        "implemented-by {} {}:{}",
                        impl_by.symbol_name,
                        rel.display(),
                        impl_by.line
                    );
                }

                // Summary line.
                println!(
                    "{} refs, {} callers, {} callees",
                    ctx.references.len(),
                    ctx.callers.len(),
                    ctx.callees.len()
                );
            }
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();
            let bold = |s: &str| -> String {
                if use_color {
                    format!("\x1b[1m{s}\x1b[0m")
                } else {
                    s.to_string()
                }
            };

            for ctx in contexts {
                // Determine the primary kind from the first definition.
                let kind_label = ctx
                    .definitions
                    .first()
                    .map(|d| format!(" ({})", kind_to_str(&d.kind)))
                    .unwrap_or_default();

                println!(
                    "{}",
                    bold(&format!("=== {}{} ===", ctx.symbol_name, kind_label))
                );
                println!();

                // Definition section.
                println!("{}", bold("Definition:"));
                if ctx.definitions.is_empty() {
                    println!("  (none)");
                } else {
                    for def in &ctx.definitions {
                        let rel = def
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&def.file_path);
                        println!("  {}:{}", rel.display(), def.line);
                    }
                }
                println!();

                // References section.
                if !ctx.references.is_empty() {
                    println!(
                        "{}",
                        bold(&format!("References ({}):", ctx.references.len()))
                    );
                    for r in &ctx.references {
                        let rel = r
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&r.file_path);
                        match r.ref_kind {
                            RefKind::Import => {
                                println!("  {}  import", rel.display());
                            }
                            RefKind::Call => {
                                let caller = r.symbol_name.as_deref().unwrap_or("?");
                                let line =
                                    r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                                println!("  {}:{}  call  {}", rel.display(), line, caller);
                            }
                        }
                    }
                    println!();
                }

                // Calls section.
                if !ctx.callees.is_empty() {
                    println!("{}", bold(&format!("Calls ({}):", ctx.callees.len())));
                    for callee in &ctx.callees {
                        let rel = callee
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&callee.file_path);
                        println!(
                            "  {}  {}:{}",
                            callee.symbol_name,
                            rel.display(),
                            callee.line
                        );
                    }
                    println!();
                }

                // Called By section.
                if !ctx.callers.is_empty() {
                    println!("{}", bold(&format!("Called By ({}):", ctx.callers.len())));
                    for caller in &ctx.callers {
                        let rel = caller
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&caller.file_path);
                        println!(
                            "  {}  {}:{}",
                            caller.symbol_name,
                            rel.display(),
                            caller.line
                        );
                    }
                    println!();
                }

                // Extends section.
                if !ctx.extends.is_empty() {
                    println!("{}", bold(&format!("Extends ({}):", ctx.extends.len())));
                    for ext in &ctx.extends {
                        let rel = ext
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&ext.file_path);
                        println!("  {}  {}:{}", ext.symbol_name, rel.display(), ext.line);
                    }
                    println!();
                }

                // Implements section.
                if !ctx.implements.is_empty() {
                    println!(
                        "{}",
                        bold(&format!("Implements ({}):", ctx.implements.len()))
                    );
                    for imp in &ctx.implements {
                        let rel = imp
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&imp.file_path);
                        println!("  {}  {}:{}", imp.symbol_name, rel.display(), imp.line);
                    }
                    println!();
                }

                // Extended By section.
                if !ctx.extended_by.is_empty() {
                    println!(
                        "{}",
                        bold(&format!("Extended By ({}):", ctx.extended_by.len()))
                    );
                    for ext_by in &ctx.extended_by {
                        let rel = ext_by
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&ext_by.file_path);
                        println!(
                            "  {}  {}:{}",
                            ext_by.symbol_name,
                            rel.display(),
                            ext_by.line
                        );
                    }
                    println!();
                }

                // Implemented By section.
                if !ctx.implemented_by.is_empty() {
                    println!(
                        "{}",
                        bold(&format!("Implemented By ({}):", ctx.implemented_by.len()))
                    );
                    for impl_by in &ctx.implemented_by {
                        let rel = impl_by
                            .file_path
                            .strip_prefix(project_root)
                            .unwrap_or(&impl_by.file_path);
                        println!(
                            "  {}  {}:{}",
                            impl_by.symbol_name,
                            rel.display(),
                            impl_by.line
                        );
                    }
                    println!();
                }
            }
        }

        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = contexts
                .iter()
                .map(|ctx| {
                    let definitions: Vec<serde_json::Value> = ctx
                        .definitions
                        .iter()
                        .map(|d| {
                            let rel = d
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&d.file_path);
                            serde_json::json!({
                                "file": rel.to_string_lossy(),
                                "line": d.line,
                                "kind": kind_to_str(&d.kind),
                                "exported": d.is_exported,
                            })
                        })
                        .collect();

                    let references: Vec<serde_json::Value> = ctx
                        .references
                        .iter()
                        .map(|r| {
                            let rel = r
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&r.file_path);
                            let kind_str = match r.ref_kind {
                                RefKind::Import => "import",
                                RefKind::Call => "call",
                            };
                            serde_json::json!({
                                "file": rel.to_string_lossy(),
                                "kind": kind_str,
                                "caller": r.symbol_name,
                                "line": r.line,
                            })
                        })
                        .collect();

                    let callees: Vec<serde_json::Value> = ctx
                        .callees
                        .iter()
                        .map(|c| {
                            let rel = c
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&c.file_path);
                            serde_json::json!({
                                "name": c.symbol_name,
                                "kind": kind_to_str(&c.kind),
                                "file": rel.to_string_lossy(),
                                "line": c.line,
                            })
                        })
                        .collect();

                    let callers: Vec<serde_json::Value> = ctx
                        .callers
                        .iter()
                        .map(|c| {
                            let rel = c
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&c.file_path);
                            serde_json::json!({
                                "name": c.symbol_name,
                                "kind": kind_to_str(&c.kind),
                                "file": rel.to_string_lossy(),
                                "line": c.line,
                            })
                        })
                        .collect();

                    let extends: Vec<serde_json::Value> = ctx
                        .extends
                        .iter()
                        .map(|e| {
                            let rel = e
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&e.file_path);
                            serde_json::json!({
                                "name": e.symbol_name,
                                "kind": kind_to_str(&e.kind),
                                "file": rel.to_string_lossy(),
                                "line": e.line,
                            })
                        })
                        .collect();

                    let implements: Vec<serde_json::Value> = ctx
                        .implements
                        .iter()
                        .map(|i| {
                            let rel = i
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&i.file_path);
                            serde_json::json!({
                                "name": i.symbol_name,
                                "kind": kind_to_str(&i.kind),
                                "file": rel.to_string_lossy(),
                                "line": i.line,
                            })
                        })
                        .collect();

                    let extended_by: Vec<serde_json::Value> = ctx
                        .extended_by
                        .iter()
                        .map(|e| {
                            let rel = e
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&e.file_path);
                            serde_json::json!({
                                "name": e.symbol_name,
                                "kind": kind_to_str(&e.kind),
                                "file": rel.to_string_lossy(),
                                "line": e.line,
                            })
                        })
                        .collect();

                    let implemented_by: Vec<serde_json::Value> = ctx
                        .implemented_by
                        .iter()
                        .map(|i| {
                            let rel = i
                                .file_path
                                .strip_prefix(project_root)
                                .unwrap_or(&i.file_path);
                            serde_json::json!({
                                "name": i.symbol_name,
                                "kind": kind_to_str(&i.kind),
                                "file": rel.to_string_lossy(),
                                "line": i.line,
                            })
                        })
                        .collect();

                    serde_json::json!({
                        "symbol": ctx.symbol_name,
                        "definitions": definitions,
                        "references": references,
                        "callees": callees,
                        "callers": callers,
                        "extends": extends,
                        "implements": implements,
                        "extended_by": extended_by,
                        "implemented_by": implemented_by,
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

// ---------------------------------------------------------------------------
// MCP String-returning formatters (siblings of the println!-based CLI formatters)
// ---------------------------------------------------------------------------

/// Format find results to a String in compact format for MCP tool responses.
///
/// Summary header (total count) is written FIRST, before any result lines,
/// so Claude can count-check results at a glance (CONTEXT.md locked decision).
pub fn format_find_to_string(results: &[FindResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "{} definitions found", results.len()).unwrap();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        writeln!(
            buf,
            "def {} {}:{} {}",
            r.symbol_name,
            rel.display(),
            r.line,
            kind_to_str(&r.kind)
        )
        .unwrap();
    }
    buf
}

/// Format project stats to a String in compact format for MCP tool responses.
///
/// Summary header (file + symbol counts) is written FIRST.
pub fn format_stats_to_string(stats: &ProjectStats) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(
        buf,
        "{} files, {} symbols",
        stats.file_count, stats.symbol_count
    )
    .unwrap();
    writeln!(buf, "files {}", stats.file_count).unwrap();
    writeln!(buf, "symbols {}", stats.symbol_count).unwrap();
    writeln!(
        buf,
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
    ).unwrap();
    writeln!(
        buf,
        "imports {} external {} unresolved {}",
        stats.import_edges, stats.external_packages, stats.unresolved_imports,
    )
    .unwrap();
    buf
}

/// Format reference results to a String in compact format for MCP tool responses.
///
/// Summary header (total count) is written FIRST.
pub fn format_refs_to_string(results: &[RefResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "{} references found", results.len()).unwrap();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        match r.ref_kind {
            RefKind::Import => {
                writeln!(buf, "ref {} import", rel.display()).unwrap();
            }
            RefKind::Call => {
                let caller = r.symbol_name.as_deref().unwrap_or("?");
                let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                writeln!(buf, "ref {}:{} call {}", rel.display(), line, caller).unwrap();
            }
        }
    }
    buf
}

/// Format impact (blast radius) results to a String in compact flat format for MCP tool responses.
///
/// Summary header (total count) is written FIRST. Uses flat (non-tree) format — MCP responses
/// do not benefit from indentation and tree mode adds ambiguity when parsed by Claude.
pub fn format_impact_to_string(results: &[ImpactResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "{} affected files", results.len()).unwrap();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        writeln!(buf, "impact {}", rel.display()).unwrap();
    }
    buf
}

/// Format circular dependency results to a String in compact format for MCP tool responses.
///
/// Summary header (total count) is written FIRST.
pub fn format_circular_to_string(cycles: &[CircularDep], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "{} circular dependencies found", cycles.len()).unwrap();
    for cycle in cycles {
        let parts: Vec<String> = cycle
            .files
            .iter()
            .map(|p| {
                p.strip_prefix(project_root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        writeln!(buf, "cycle {}", parts.join(" -> ")).unwrap();
    }
    buf
}

/// Format symbol context results to a String with labeled sections for MCP tool responses.
///
/// Summary header (`{N} symbols`) is written FIRST (CONTEXT.md locked decision).
/// Each non-empty relationship group is preceded by a labeled section delimiter
/// (`--- callers ---`, `--- callees ---`, etc.) so Claude can parse sections easily
/// (CONTEXT.md locked decision on context/360-degree labeled sections).
pub fn format_context_to_string(contexts: &[SymbolContext], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(buf, "{} symbols", contexts.len()).unwrap();
    for ctx in contexts {
        writeln!(buf, "symbol {}", ctx.symbol_name).unwrap();

        if !ctx.definitions.is_empty() {
            writeln!(buf, "--- definitions ---").unwrap();
            for def in &ctx.definitions {
                let rel = def
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&def.file_path);
                writeln!(
                    buf,
                    "def {}:{} {}",
                    rel.display(),
                    def.line,
                    kind_to_str(&def.kind)
                )
                .unwrap();
            }
        }

        if !ctx.references.is_empty() {
            writeln!(buf, "--- references ---").unwrap();
            for r in &ctx.references {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                match r.ref_kind {
                    RefKind::Import => {
                        writeln!(buf, "ref {} import", rel.display()).unwrap();
                    }
                    RefKind::Call => {
                        let caller = r.symbol_name.as_deref().unwrap_or("?");
                        let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                        writeln!(buf, "ref {}:{} call {}", rel.display(), line, caller).unwrap();
                    }
                }
            }
        }

        if !ctx.callers.is_empty() {
            writeln!(buf, "--- callers ---").unwrap();
            for caller in &ctx.callers {
                let rel = caller
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&caller.file_path);
                writeln!(
                    buf,
                    "called-by {} {}:{}",
                    caller.symbol_name,
                    rel.display(),
                    caller.line
                )
                .unwrap();
            }
        }

        if !ctx.callees.is_empty() {
            writeln!(buf, "--- callees ---").unwrap();
            for callee in &ctx.callees {
                let rel = callee
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&callee.file_path);
                writeln!(
                    buf,
                    "calls {} {}:{}",
                    callee.symbol_name,
                    rel.display(),
                    callee.line
                )
                .unwrap();
            }
        }

        if !ctx.extends.is_empty() {
            writeln!(buf, "--- extends ---").unwrap();
            for ext in &ctx.extends {
                let rel = ext
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&ext.file_path);
                writeln!(
                    buf,
                    "extends {} {}:{}",
                    ext.symbol_name,
                    rel.display(),
                    ext.line
                )
                .unwrap();
            }
        }

        if !ctx.implements.is_empty() {
            writeln!(buf, "--- implements ---").unwrap();
            for imp in &ctx.implements {
                let rel = imp
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&imp.file_path);
                writeln!(
                    buf,
                    "implements {} {}:{}",
                    imp.symbol_name,
                    rel.display(),
                    imp.line
                )
                .unwrap();
            }
        }

        if !ctx.extended_by.is_empty() {
            writeln!(buf, "--- extended-by ---").unwrap();
            for ext_by in &ctx.extended_by {
                let rel = ext_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&ext_by.file_path);
                writeln!(
                    buf,
                    "extended-by {} {}:{}",
                    ext_by.symbol_name,
                    rel.display(),
                    ext_by.line
                )
                .unwrap();
            }
        }

        if !ctx.implemented_by.is_empty() {
            writeln!(buf, "--- implemented-by ---").unwrap();
            for impl_by in &ctx.implemented_by {
                let rel = impl_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&impl_by.file_path);
                writeln!(
                    buf,
                    "implemented-by {} {}:{}",
                    impl_by.symbol_name,
                    rel.display(),
                    impl_by.line
                )
                .unwrap();
            }
        }
    }
    buf
}

/// Format and print circular dependency results to stdout.
pub fn format_circular_results(cycles: &[CircularDep], format: &OutputFormat, project_root: &Path) {
    match format {
        OutputFormat::Compact => {
            for cycle in cycles {
                let parts: Vec<String> = cycle
                    .files
                    .iter()
                    .map(|p| {
                        p.strip_prefix(project_root)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .to_string()
                    })
                    .collect();
                println!("cycle {}", parts.join(" -> "));
            }
            println!("{} cycles found", cycles.len());
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

            for (i, cycle) in cycles.iter().enumerate() {
                println!("{}", header(&format!("=== Cycle {} ===", i + 1)));
                // Show all but the last entry (which is the repeated first file).
                let unique_files = &cycle.files[..cycle.files.len().saturating_sub(1)];
                for path in unique_files {
                    let rel = path.strip_prefix(project_root).unwrap_or(path);
                    println!("  {}", rel.display());
                }
                println!();
            }
            println!("{} cycles found", cycles.len());
        }

        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = cycles
                .iter()
                .map(|cycle| {
                    let files: Vec<String> = cycle
                        .files
                        .iter()
                        .map(|p| {
                            p.strip_prefix(project_root)
                                .unwrap_or(p)
                                .to_string_lossy()
                                .to_string()
                        })
                        .collect();
                    serde_json::json!({ "files": files })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_results).unwrap_or_default()
            );
        }
    }
}

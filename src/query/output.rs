use std::io::IsTerminal;
use std::path::Path;

use crate::cli::OutputFormat;
use crate::graph::node::SymbolVisibility;
use crate::query::circular::CircularDep;
use crate::query::context::SymbolContext;
use crate::query::find::FindResult;
use crate::query::find::kind_to_str;
use crate::query::impact::ImpactResult;
use crate::query::refs::{RefKind, RefResult};
use crate::query::stats::ProjectStats;

/// Determine the display language name of a file from its extension.
fn language_of_file(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" => "JavaScript",
        "rs" => "Rust",
        _ => "Unknown",
    }
}

/// Returns true if a slice of file paths spans multiple distinct languages.
fn is_mixed_language<F: Fn(&T) -> &Path, T>(items: &[T], get_path: F) -> bool {
    if items.is_empty() {
        return false;
    }
    let first_lang = language_of_file(get_path(&items[0]));
    items[1..]
        .iter()
        .any(|i| language_of_file(get_path(i)) != first_lang)
}

/// Sort key for language grouping: Rust < TypeScript < JavaScript < Unknown (alphabetical).
fn language_sort_key(lang: &str) -> u8 {
    match lang {
        "JavaScript" => 1,
        "Rust" => 2,
        "TypeScript" => 3,
        _ => 4,
    }
}

/// Map a `SymbolVisibility` to its display string for output.
fn visibility_str(vis: &SymbolVisibility) -> &'static str {
    match vis {
        SymbolVisibility::Pub => "pub",
        SymbolVisibility::PubCrate => "pub(crate)",
        SymbolVisibility::Private => "private",
    }
}

/// Returns true if any result has non-Private visibility.
/// Used to suppress visibility column noise for pure TS/JS projects.
fn any_non_private(results: &[FindResult]) -> bool {
    results
        .iter()
        .any(|r| r.visibility != SymbolVisibility::Private)
}

/// Format and print find results to stdout according to the selected output format.
///
/// In compact and table modes, if results span multiple languages, groups them under
/// `--- {Language} ---` section headers. JSON mode adds a "language" field per result.
pub fn format_find_results(results: &[FindResult], format: &OutputFormat, project_root: &Path) {
    let show_vis = any_non_private(results);
    let mixed = is_mixed_language(results, |r: &FindResult| r.file_path.as_path());

    // Sort results: by language first (for grouping), then file path, then line.
    let mut sorted = results.to_vec();
    if mixed {
        sorted.sort_by(|a, b| {
            let la = language_of_file(&a.file_path);
            let lb = language_of_file(&b.file_path);
            language_sort_key(la)
                .cmp(&language_sort_key(lb))
                .then(a.file_path.cmp(&b.file_path))
                .then(a.line.cmp(&b.line))
        });
    }

    match format {
        OutputFormat::Compact => {
            let mut last_lang: Option<&'static str> = None;
            for r in &sorted {
                if mixed {
                    let lang = language_of_file(&r.file_path);
                    if last_lang != Some(lang) {
                        println!("--- {} ---", lang);
                        last_lang = Some(lang);
                    }
                }
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                if show_vis {
                    println!(
                        "def {} {}:{} {} {}",
                        r.symbol_name,
                        rel.display(),
                        r.line,
                        kind_to_str(&r.kind),
                        visibility_str(&r.visibility),
                    );
                } else {
                    println!(
                        "def {} {}:{} {}",
                        r.symbol_name,
                        rel.display(),
                        r.line,
                        kind_to_str(&r.kind)
                    );
                }
            }
            println!("{} definitions found", results.len());
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();

            // Column widths: auto-sized to data.
            let name_w = sorted
                .iter()
                .map(|r| r.symbol_name.len())
                .max()
                .unwrap_or(6)
                .max(6);
            let file_w = sorted
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

            if show_vis {
                if use_color {
                    println!(
                        "\x1b[1m{:<name_w$}  {:<file_w$}  {:>4}  {:<10}  KIND\x1b[0m",
                        "SYMBOL",
                        "FILE",
                        "LINE",
                        "VIS",
                        name_w = name_w,
                        file_w = file_w,
                    );
                } else {
                    println!(
                        "{:<name_w$}  {:<file_w$}  {:>4}  {:<10}  KIND",
                        "SYMBOL",
                        "FILE",
                        "LINE",
                        "VIS",
                        name_w = name_w,
                        file_w = file_w,
                    );
                }
                println!("{}", "-".repeat(name_w + file_w + 26));
                let mut last_lang: Option<&'static str> = None;
                for r in &sorted {
                    if mixed {
                        let lang = language_of_file(&r.file_path);
                        if last_lang != Some(lang) {
                            println!("--- {} ---", lang);
                            last_lang = Some(lang);
                        }
                    }
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    println!(
                        "{:<name_w$}  {:<file_w$}  {:>4}  {:<10}  {}",
                        r.symbol_name,
                        rel.display(),
                        r.line,
                        visibility_str(&r.visibility),
                        kind_to_str(&r.kind),
                        name_w = name_w,
                        file_w = file_w,
                    );
                }
            } else {
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
                let mut last_lang: Option<&'static str> = None;
                for r in &sorted {
                    if mixed {
                        let lang = language_of_file(&r.file_path);
                        if last_lang != Some(lang) {
                            println!("--- {} ---", lang);
                            last_lang = Some(lang);
                        }
                    }
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
        }

        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = sorted
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
                        "language": language_of_file(&r.file_path),
                        "line": r.line,
                        "col": r.col,
                        "exported": r.is_exported,
                        "default": r.is_default,
                        "visibility": visibility_str(&r.visibility),
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

/// Determine if the stats have Rust symbols present.
fn stats_has_rust(stats: &ProjectStats) -> bool {
    stats.rust_fns
        + stats.rust_structs
        + stats.rust_enums
        + stats.rust_traits
        + stats.rust_impl_methods
        + stats.rust_type_aliases
        + stats.rust_consts
        + stats.rust_statics
        + stats.rust_macros
        + stats.rust_imports
        + stats.rust_reexports
        > 0
}

/// Determine if the stats have TypeScript/JavaScript symbols present.
fn stats_has_ts_js(stats: &ProjectStats) -> bool {
    // Total symbols minus Rust-specific symbols indicates TS/JS presence.
    let rust_total = stats.rust_fns
        + stats.rust_structs
        + stats.rust_enums
        + stats.rust_traits
        + stats.rust_impl_methods
        + stats.rust_type_aliases
        + stats.rust_consts
        + stats.rust_statics
        + stats.rust_macros;
    stats.symbol_count > rust_total
        || stats.functions > stats.rust_fns
        || stats.classes > 0
        || stats.interfaces > 0
        || stats.variables > 0
        || stats.components > 0
}

/// Format and print project stats to stdout according to the selected output format.
///
/// `language_filter`: if Some("rust"), show only Rust section; if Some("typescript"),
/// show only TypeScript section; if None, show all sections with totals.
pub fn format_stats(stats: &ProjectStats, format: &OutputFormat, language_filter: Option<&str>) {
    let show_rust = language_filter.is_none() || language_filter == Some("rust");
    let show_ts = language_filter.is_none()
        || language_filter == Some("typescript")
        || language_filter == Some("javascript");
    let show_totals = language_filter.is_none();

    let has_rust = stats_has_rust(stats);
    let has_ts = stats_has_ts_js(stats);

    match format {
        OutputFormat::Compact => {
            // File overview line
            if stats.non_parsed_files > 0 {
                println!(
                    "{} files ({} source, {} non-parsed), {} symbols",
                    stats.file_count,
                    stats.source_files,
                    stats.non_parsed_files,
                    stats.symbol_count
                );
                println!(
                    "non-parsed: doc {} config {} ci {} asset {} other {}",
                    stats.doc_files,
                    stats.config_files,
                    stats.ci_files,
                    stats.asset_files,
                    stats.other_files,
                );
            }
            // Per-language sections with per-language counts and combined totals.
            if show_rust && has_rust {
                let rust_symbol_total = stats.rust_fns
                    + stats.rust_structs
                    + stats.rust_enums
                    + stats.rust_traits
                    + stats.rust_impl_methods
                    + stats.rust_type_aliases
                    + stats.rust_consts
                    + stats.rust_statics
                    + stats.rust_macros;
                println!(
                    "Rust: {} symbols (fn: {} struct: {} enum: {} trait: {} impl_method: {} type: {} const: {} static: {} macro: {})",
                    rust_symbol_total,
                    stats.rust_fns,
                    stats.rust_structs,
                    stats.rust_enums,
                    stats.rust_traits,
                    stats.rust_impl_methods,
                    stats.rust_type_aliases,
                    stats.rust_consts,
                    stats.rust_statics,
                    stats.rust_macros,
                );
                println!(
                    "rust_use {} rust_pub_use {}",
                    stats.rust_imports, stats.rust_reexports,
                );
                // Dependencies section (Phase 9)
                let has_deps = stats.external_packages > 0 || stats.builtin_count > 0;
                if has_deps {
                    println!(
                        "dependencies external_crates {} (usages {}) builtins {} (usages {})",
                        stats.external_packages,
                        stats.external_usage_count,
                        stats.builtin_count,
                        stats.builtin_usage_count,
                    );
                }
                // Per-crate breakdown (Phase 9, only for workspaces with multiple crates)
                if !stats.rust_crate_stats.is_empty() {
                    for cs in &stats.rust_crate_stats {
                        println!(
                            "crate {} files {} symbols {}",
                            cs.crate_name, cs.file_count, cs.symbol_count
                        );
                    }
                }
            }
            if show_ts && has_ts {
                let ts_fns = stats.functions.saturating_sub(stats.rust_fns);
                let ts_enums = stats.enums.saturating_sub(stats.rust_enums);
                let ts_type_aliases = stats.type_aliases.saturating_sub(stats.rust_type_aliases);
                println!(
                    "TypeScript: {} symbols (function: {} class: {} interface: {} type: {} enum: {} variable: {} component: {} method: {} property: {})",
                    stats.symbol_count.saturating_sub(
                        stats.rust_fns
                            + stats.rust_structs
                            + stats.rust_enums
                            + stats.rust_traits
                            + stats.rust_impl_methods
                            + stats.rust_type_aliases
                            + stats.rust_consts
                            + stats.rust_statics
                            + stats.rust_macros
                    ),
                    ts_fns,
                    stats.classes,
                    stats.interfaces,
                    ts_type_aliases,
                    ts_enums,
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
            if show_totals && has_rust && has_ts {
                println!("---");
                println!(
                    "Total: {} files, {} symbols",
                    stats.file_count, stats.symbol_count
                );
            } else if show_totals {
                println!("files {}", stats.file_count);
                println!("symbols {}", stats.symbol_count);
            }
            // Fallback: show full stats if neither Rust nor TS-specific sections match
            if !has_rust && !has_ts {
                println!("files {}", stats.file_count);
                println!("symbols {}", stats.symbol_count);
                println!(
                    "imports {} external {} unresolved {}",
                    stats.import_edges, stats.external_packages, stats.unresolved_imports
                );
            }
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

            if show_totals || show_rust && !show_ts || show_ts && !show_rust {
                println!("{}", header("=== Project Overview ==="));
                println!(
                    "Files:    {} ({} source, {} non-parsed)",
                    stats.file_count, stats.source_files, stats.non_parsed_files
                );
                println!("Symbols:  {}", stats.symbol_count);
                if stats.non_parsed_files > 0 {
                    println!(
                        "  doc: {} config: {} ci: {} asset: {} other: {}",
                        stats.doc_files,
                        stats.config_files,
                        stats.ci_files,
                        stats.asset_files,
                        stats.other_files
                    );
                }
                println!();
            }

            if show_ts && has_ts {
                let ts_fns = stats.functions.saturating_sub(stats.rust_fns);
                let ts_enums = stats.enums.saturating_sub(stats.rust_enums);
                let ts_type_aliases = stats.type_aliases.saturating_sub(stats.rust_type_aliases);
                println!("{}", header("--- TypeScript/JavaScript ---"));
                println!("  Functions:    {}", ts_fns);
                println!("  Classes:      {}", stats.classes);
                println!("  Interfaces:   {}", stats.interfaces);
                println!("  Type Aliases: {}", ts_type_aliases);
                println!("  Enums:        {}", ts_enums);
                println!("  Variables:    {}", stats.variables);
                println!("  Components:   {}", stats.components);
                println!("  Methods:      {}", stats.methods);
                println!("  Properties:   {}", stats.properties);
                println!();
                println!("{}", header("--- Import Summary ---"));
                println!("  Resolved imports:  {}", stats.import_edges);
                println!("  External packages: {}", stats.external_packages);
                println!("  Unresolved:        {}", stats.unresolved_imports);
            } else if show_totals && !has_rust {
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

            // Rust section — only when Rust symbols are present and filter allows
            if show_rust && has_rust {
                println!();
                println!("{}", header("--- Rust Symbols ---"));
                println!("  fn:          {}", stats.rust_fns);
                println!("  struct:      {}", stats.rust_structs);
                println!("  enum:        {}", stats.rust_enums);
                println!("  trait:       {}", stats.rust_traits);
                println!("  impl method: {}", stats.rust_impl_methods);
                println!("  type:        {}", stats.rust_type_aliases);
                println!("  const:       {}", stats.rust_consts);
                println!("  static:      {}", stats.rust_statics);
                println!("  macro:       {}", stats.rust_macros);
                println!("  use (unresolved): {}", stats.rust_imports);
                println!("  pub use (re-exports): {}", stats.rust_reexports);

                // Dependencies section (Phase 9)
                let has_deps = stats.external_packages > 0 || stats.builtin_count > 0;
                if has_deps {
                    println!();
                    println!("{}", header("--- Dependencies ---"));
                    if stats.external_packages > 0 {
                        println!(
                            "  External crates: {} ({} usages)",
                            stats.external_packages, stats.external_usage_count
                        );
                    }
                    if stats.builtin_count > 0 {
                        println!(
                            "  Builtins (std/core/alloc): {} ({} usages)",
                            stats.builtin_count, stats.builtin_usage_count
                        );
                    }
                }

                // Per-crate breakdown (Phase 9, only for workspaces with multiple crates)
                if !stats.rust_crate_stats.is_empty() {
                    println!();
                    println!("{}", header("--- Per-Crate Breakdown ---"));
                    for cs in &stats.rust_crate_stats {
                        println!(
                            "  {} ({} files, {} symbols: fn={} struct={} enum={} trait={} impl={})",
                            cs.crate_name,
                            cs.file_count,
                            cs.symbol_count,
                            cs.fn_count,
                            cs.struct_count,
                            cs.enum_count,
                            cs.trait_count,
                            cs.impl_method_count,
                        );
                    }
                }
            }
        }

        OutputFormat::Json => {
            // Build per-crate breakdown as JSON array
            let crate_stats_json: Vec<serde_json::Value> = stats
                .rust_crate_stats
                .iter()
                .map(|cs| {
                    serde_json::json!({
                        "crate_name": cs.crate_name,
                        "file_count": cs.file_count,
                        "symbol_count": cs.symbol_count,
                        "fn_count": cs.fn_count,
                        "struct_count": cs.struct_count,
                        "enum_count": cs.enum_count,
                        "trait_count": cs.trait_count,
                        "impl_method_count": cs.impl_method_count,
                        "type_alias_count": cs.type_alias_count,
                        "const_count": cs.const_count,
                        "static_count": cs.static_count,
                        "macro_count": cs.macro_count,
                    })
                })
                .collect();

            let json = serde_json::json!({
                "file_count": stats.file_count,
                "source_files": stats.source_files,
                "non_parsed_files": stats.non_parsed_files,
                "doc_files": stats.doc_files,
                "config_files": stats.config_files,
                "ci_files": stats.ci_files,
                "asset_files": stats.asset_files,
                "other_files": stats.other_files,
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
                "rust_fns": stats.rust_fns,
                "rust_structs": stats.rust_structs,
                "rust_enums": stats.rust_enums,
                "rust_traits": stats.rust_traits,
                "rust_impl_methods": stats.rust_impl_methods,
                "rust_type_aliases": stats.rust_type_aliases,
                "rust_consts": stats.rust_consts,
                "rust_statics": stats.rust_statics,
                "rust_macros": stats.rust_macros,
                "rust_imports": stats.rust_imports,
                "rust_reexports": stats.rust_reexports,
                "dependencies": {
                    "external_crates": stats.external_packages,
                    "external_usage_count": stats.external_usage_count,
                    "builtin_crates": stats.builtin_count,
                    "builtin_usage_count": stats.builtin_usage_count,
                },
                "crate_stats": crate_stats_json,
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

/// Format find results to a String in compact prefix-free format for MCP tool responses.
///
/// No summary line. No "def " prefix. Line format: `{rel_path}:{line} {symbol_name} {kind}`
/// (with optional visibility suffix for Rust). In mixed-language results, groups by language
/// with `--- {Language} ---` section headers.
pub fn format_find_to_string(results: &[FindResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let show_vis = any_non_private(results);
    let mixed = is_mixed_language(results, |r: &FindResult| r.file_path.as_path());

    let mut sorted = results.to_vec();
    if mixed {
        sorted.sort_by(|a, b| {
            let la = language_of_file(&a.file_path);
            let lb = language_of_file(&b.file_path);
            language_sort_key(la)
                .cmp(&language_sort_key(lb))
                .then(a.file_path.cmp(&b.file_path))
                .then(a.line.cmp(&b.line))
        });
    }

    let mut buf = String::new();
    let mut last_lang: Option<&'static str> = None;
    for r in &sorted {
        if mixed {
            let lang = language_of_file(&r.file_path);
            if last_lang != Some(lang) {
                writeln!(buf, "--- {} ---", lang).unwrap();
                last_lang = Some(lang);
            }
        }
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        if show_vis {
            writeln!(
                buf,
                "{}:{} {} {} {}",
                rel.display(),
                r.line,
                r.symbol_name,
                kind_to_str(&r.kind),
                visibility_str(&r.visibility),
            )
            .unwrap();
        } else {
            writeln!(
                buf,
                "{}:{} {} {}",
                rel.display(),
                r.line,
                r.symbol_name,
                kind_to_str(&r.kind)
            )
            .unwrap();
        }
    }
    buf
}

/// Format project stats to a String in compact format for MCP tool responses.
///
/// Summary header (file + symbol counts) is written FIRST.
/// `language_filter`: if Some, only show the matching language section.
pub fn format_stats_to_string(stats: &ProjectStats, language_filter: Option<&str>) -> String {
    use std::fmt::Write;
    let mut buf = String::new();

    let show_rust = language_filter.is_none() || language_filter == Some("rust");
    let show_ts = language_filter.is_none()
        || language_filter == Some("typescript")
        || language_filter == Some("javascript");
    let show_totals = language_filter.is_none();

    let has_rust = stats_has_rust(stats);
    let has_ts = stats_has_ts_js(stats);

    writeln!(
        buf,
        "{} files ({} source, {} non-parsed), {} symbols",
        stats.file_count, stats.source_files, stats.non_parsed_files, stats.symbol_count
    )
    .unwrap();
    if stats.non_parsed_files > 0 {
        writeln!(
            buf,
            "non-parsed: doc {} config {} ci {} asset {} other {}",
            stats.doc_files,
            stats.config_files,
            stats.ci_files,
            stats.asset_files,
            stats.other_files,
        )
        .unwrap();
    }

    if show_rust && has_rust {
        let rust_symbol_total = stats.rust_fns
            + stats.rust_structs
            + stats.rust_enums
            + stats.rust_traits
            + stats.rust_impl_methods
            + stats.rust_type_aliases
            + stats.rust_consts
            + stats.rust_statics
            + stats.rust_macros;
        writeln!(buf, "Rust: {} symbols (fn: {} struct: {} enum: {} trait: {} impl_method: {} type: {} const: {} static: {} macro: {})",
            rust_symbol_total,
            stats.rust_fns, stats.rust_structs, stats.rust_enums,
            stats.rust_traits, stats.rust_impl_methods, stats.rust_type_aliases,
            stats.rust_consts, stats.rust_statics, stats.rust_macros,
        ).unwrap();
        writeln!(
            buf,
            "rust_use {} rust_pub_use {}",
            stats.rust_imports, stats.rust_reexports,
        )
        .unwrap();
        // Dependencies section (Phase 9)
        let has_deps = stats.external_packages > 0 || stats.builtin_count > 0;
        if has_deps {
            writeln!(
                buf,
                "dependencies external_crates {} (usages {}) builtins {} (usages {})",
                stats.external_packages,
                stats.external_usage_count,
                stats.builtin_count,
                stats.builtin_usage_count,
            )
            .unwrap();
        }
        // Per-crate breakdown (Phase 9, only for workspaces with multiple crates)
        if !stats.rust_crate_stats.is_empty() {
            for cs in &stats.rust_crate_stats {
                writeln!(
                    buf,
                    "crate {} files {} symbols {}",
                    cs.crate_name, cs.file_count, cs.symbol_count
                )
                .unwrap();
            }
        }
    }

    if show_ts && has_ts {
        let ts_fns = stats.functions.saturating_sub(stats.rust_fns);
        let ts_enums = stats.enums.saturating_sub(stats.rust_enums);
        let ts_type_aliases = stats.type_aliases.saturating_sub(stats.rust_type_aliases);
        writeln!(
            buf,
            "TypeScript: functions {} classes {} interfaces {} types {} enums {} variables {} components {} methods {} properties {}",
            ts_fns, stats.classes, stats.interfaces,
            ts_type_aliases, ts_enums,
            stats.variables, stats.components, stats.methods, stats.properties,
        ).unwrap();
        writeln!(
            buf,
            "imports {} external {} unresolved {}",
            stats.import_edges, stats.external_packages, stats.unresolved_imports,
        )
        .unwrap();
    }

    if show_totals && has_rust && has_ts {
        writeln!(buf, "---").unwrap();
        writeln!(
            buf,
            "Total: {} files, {} symbols",
            stats.file_count, stats.symbol_count
        )
        .unwrap();
    } else if show_totals && !has_rust {
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
    }

    buf
}

/// Format reference results to a String in compact prefix-free format for MCP tool responses.
///
/// No summary line. No "ref " prefix. Line formats:
/// - Import: `{rel_path} import`
/// - Call:   `{rel_path}:{line} call {caller_name}`
pub fn format_refs_to_string(results: &[RefResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        match r.ref_kind {
            RefKind::Import => {
                writeln!(buf, "{} import", rel.display()).unwrap();
            }
            RefKind::Call => {
                let caller = r.symbol_name.as_deref().unwrap_or("?");
                let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                writeln!(buf, "{}:{} call {}", rel.display(), line, caller).unwrap();
            }
        }
    }
    buf
}

/// Format impact (blast radius) results to a String in compact prefix-free flat format for MCP tool responses.
///
/// No summary line. No "impact " prefix. Line format: `{rel_path}`.
/// Uses flat (non-tree) format — MCP responses do not benefit from indentation.
pub fn format_impact_to_string(results: &[ImpactResult], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        writeln!(buf, "{}", rel.display()).unwrap();
    }
    buf
}

/// Format circular dependency results to a String in compact prefix-free format for MCP tool responses.
///
/// No summary line. No "cycle " prefix. Line format: `{file1} -> {file2} -> {file3}`.
pub fn format_circular_to_string(cycles: &[CircularDep], project_root: &Path) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
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
        writeln!(buf, "{}", parts.join(" -> ")).unwrap();
    }
    buf
}

/// Parse a sections filter string into an active set of section names.
///
/// - `None` input → `None` output (no filtering, all sections shown)
/// - Characters map to section names: r=references, c=callers, e=callees,
///   x=extends, i=implements, X=extended-by, I=implemented-by
/// - Commas and whitespace are separators (silently ignored)
/// - Unknown characters are silently ignored
/// - Returns `Some(HashSet)` with the matched section names
pub fn parse_sections(sections: Option<&str>) -> Option<std::collections::HashSet<&'static str>> {
    let s = sections?;
    let mut set = std::collections::HashSet::new();
    for ch in s.chars() {
        match ch {
            'r' => { set.insert("references"); }
            'c' => { set.insert("callers"); }
            'e' => { set.insert("callees"); }
            'x' => { set.insert("extends"); }
            'i' => { set.insert("implements"); }
            'X' => { set.insert("extended-by"); }
            'I' => { set.insert("implemented-by"); }
            _ => {} // separators (comma, space) and unknown chars silently ignored
        }
    }
    Some(set)
}

/// Format symbol context results to a String in compact prefix-free format for MCP tool responses.
///
/// No "N symbols" summary. No "symbol " prefix (bare symbol name on its own line).
/// No "--- section ---" delimiter lines. No "def ", "ref ", "called-by ", "calls ",
/// "extends ", "implements ", "extended-by ", "implemented-by " prefixes.
///
/// Per-section formats:
/// - Symbol header:          `{symbol_name}`
/// - Definitions:            `{rel_path}:{line} {kind}`
/// - References (import):    `{rel_path} import`
/// - References (call):      `{rel_path}:{line} call {caller}`
/// - Callers:                `{caller_name} {rel_path}:{line}`
/// - Callees:                `{callee_name} {rel_path}:{line}`
/// - Extends/implements/extended-by/implemented-by: `{name} {rel_path}:{line}`
///
/// Empty sections are silently omitted.
///
/// `sections`: optional filter string (e.g. `"r,c"`). Definitions are always included.
/// Non-empty sections that were filtered out are listed on an `omitted: ...` line.
pub fn format_context_to_string(
    contexts: &[SymbolContext],
    project_root: &Path,
    sections: Option<&str>,
) -> String {
    use std::fmt::Write;
    let active = parse_sections(sections);
    let mut buf = String::new();
    for ctx in contexts {
        writeln!(buf, "{}", ctx.symbol_name).unwrap();

        // Definitions are ALWAYS rendered regardless of filter.
        for def in &ctx.definitions {
            let rel = def
                .file_path
                .strip_prefix(project_root)
                .unwrap_or(&def.file_path);
            writeln!(
                buf,
                "{}:{} {}",
                rel.display(),
                def.line,
                kind_to_str(&def.kind)
            )
            .unwrap();
        }

        // Track non-empty sections that were filtered out.
        let mut omitted: Vec<&'static str> = Vec::new();

        // References
        if active.as_ref().map_or(true, |s| s.contains("references")) {
            for r in &ctx.references {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                match r.ref_kind {
                    RefKind::Import => {
                        writeln!(buf, "{} import", rel.display()).unwrap();
                    }
                    RefKind::Call => {
                        let caller = r.symbol_name.as_deref().unwrap_or("?");
                        let line = r.line.map_or_else(|| "?".to_string(), |l| l.to_string());
                        writeln!(buf, "{}:{} call {}", rel.display(), line, caller).unwrap();
                    }
                }
            }
        } else if !ctx.references.is_empty() {
            omitted.push("references");
        }

        // Callers
        if active.as_ref().map_or(true, |s| s.contains("callers")) {
            for caller in &ctx.callers {
                let rel = caller
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&caller.file_path);
                writeln!(buf, "{} {}:{}", caller.symbol_name, rel.display(), caller.line).unwrap();
            }
        } else if !ctx.callers.is_empty() {
            omitted.push("callers");
        }

        // Callees
        if active.as_ref().map_or(true, |s| s.contains("callees")) {
            for callee in &ctx.callees {
                let rel = callee
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&callee.file_path);
                writeln!(buf, "{} {}:{}", callee.symbol_name, rel.display(), callee.line).unwrap();
            }
        } else if !ctx.callees.is_empty() {
            omitted.push("callees");
        }

        // Extends
        if active.as_ref().map_or(true, |s| s.contains("extends")) {
            for ext in &ctx.extends {
                let rel = ext
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&ext.file_path);
                writeln!(buf, "{} {}:{}", ext.symbol_name, rel.display(), ext.line).unwrap();
            }
        } else if !ctx.extends.is_empty() {
            omitted.push("extends");
        }

        // Implements
        if active.as_ref().map_or(true, |s| s.contains("implements")) {
            for imp in &ctx.implements {
                let rel = imp
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&imp.file_path);
                writeln!(buf, "{} {}:{}", imp.symbol_name, rel.display(), imp.line).unwrap();
            }
        } else if !ctx.implements.is_empty() {
            omitted.push("implements");
        }

        // Extended-by
        if active.as_ref().map_or(true, |s| s.contains("extended-by")) {
            for ext_by in &ctx.extended_by {
                let rel = ext_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&ext_by.file_path);
                writeln!(buf, "{} {}:{}", ext_by.symbol_name, rel.display(), ext_by.line).unwrap();
            }
        } else if !ctx.extended_by.is_empty() {
            omitted.push("extended-by");
        }

        // Implemented-by
        if active.as_ref().map_or(true, |s| s.contains("implemented-by")) {
            for impl_by in &ctx.implemented_by {
                let rel = impl_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&impl_by.file_path);
                writeln!(buf, "{} {}:{}", impl_by.symbol_name, rel.display(), impl_by.line)
                    .unwrap();
            }
        } else if !ctx.implemented_by.is_empty() {
            omitted.push("implemented-by");
        }

        // Emit omitted line only when sections were filtered AND some were non-empty.
        if !omitted.is_empty() {
            writeln!(buf, "omitted: {}", omitted.join(", ")).unwrap();
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

// ---------------------------------------------------------------------------
// Unit tests for MCP formatters
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::graph::node::{SymbolKind, SymbolVisibility};
    use crate::query::circular::CircularDep;
    use crate::query::context::{CallInfo, SymbolContext};
    use crate::query::find::FindResult;
    use crate::query::impact::ImpactResult;
    use crate::query::refs::{RefKind, RefResult};

    fn make_find_result(name: &str, path: &str, line: usize, kind: SymbolKind) -> FindResult {
        FindResult {
            symbol_name: name.to_string(),
            kind,
            file_path: PathBuf::from(path),
            line,
            col: 0,
            is_exported: false,
            is_default: false,
            visibility: SymbolVisibility::Private,
        }
    }

    #[test]
    fn test_mcp_find_format_no_prefix() {
        let root = PathBuf::from("/project");
        let results = vec![make_find_result(
            "MyFunc",
            "/project/src/foo.ts",
            10,
            SymbolKind::Function,
        )];
        let output = format_find_to_string(&results, &root);

        // Must NOT contain old prefix
        assert!(!output.contains("def "), "output should not contain 'def ' prefix");
        // Must NOT contain old summary line
        assert!(
            !output.contains("definitions found"),
            "output should not contain 'definitions found' summary"
        );
        // Must contain new compact format: path:line name kind
        assert!(
            output.contains("src/foo.ts:10 MyFunc function"),
            "output should contain compact format 'src/foo.ts:10 MyFunc function', got: {output}"
        );
    }

    #[test]
    fn test_mcp_refs_format_no_prefix() {
        let root = PathBuf::from("/project");
        let results = vec![
            RefResult {
                file_path: PathBuf::from("/project/src/bar.ts"),
                ref_kind: RefKind::Import,
                symbol_name: None,
                line: None,
            },
            RefResult {
                file_path: PathBuf::from("/project/src/baz.ts"),
                ref_kind: RefKind::Call,
                symbol_name: Some("callerFn".to_string()),
                line: Some(42),
            },
        ];
        let output = format_refs_to_string(&results, &root);

        // Must NOT contain old prefix
        assert!(!output.contains("ref "), "output should not contain 'ref ' prefix");
        // Must NOT contain old summary line
        assert!(
            !output.contains("references found"),
            "output should not contain 'references found' summary"
        );
        // Must contain new compact formats
        assert!(
            output.contains("src/bar.ts import"),
            "output should contain import ref format, got: {output}"
        );
        assert!(
            output.contains("src/baz.ts:42 call callerFn"),
            "output should contain call ref format, got: {output}"
        );
    }

    #[test]
    fn test_mcp_impact_format_no_prefix() {
        let root = PathBuf::from("/project");
        let results = vec![ImpactResult {
            file_path: PathBuf::from("/project/src/affected.ts"),
            depth: 1,
        }];
        let output = format_impact_to_string(&results, &root);

        // Must NOT contain old prefix
        assert!(!output.contains("impact "), "output should not contain 'impact ' prefix");
        // Must NOT contain old summary line
        assert!(
            !output.contains("affected files"),
            "output should not contain 'affected files' summary"
        );
        // Must contain bare path
        assert!(
            output.contains("src/affected.ts"),
            "output should contain relative path, got: {output}"
        );
    }

    #[test]
    fn test_mcp_circular_format_no_prefix() {
        let root = PathBuf::from("/project");
        let cycles = vec![CircularDep {
            files: vec![
                PathBuf::from("/project/src/a.ts"),
                PathBuf::from("/project/src/b.ts"),
                PathBuf::from("/project/src/a.ts"),
            ],
        }];
        let output = format_circular_to_string(&cycles, &root);

        // Must NOT contain old prefix
        assert!(!output.contains("cycle "), "output should not contain 'cycle ' prefix");
        // Must NOT contain old summary line
        assert!(
            !output.contains("circular dependencies found"),
            "output should not contain 'circular dependencies found' summary"
        );
        // Must contain arrow chain format
        assert!(
            output.contains("src/a.ts -> src/b.ts -> src/a.ts"),
            "output should contain arrow-chain format, got: {output}"
        );
    }

    #[test]
    fn test_mcp_context_format_no_delimiters() {
        let root = PathBuf::from("/project");
        let def = make_find_result("MyStruct", "/project/src/lib.rs", 5, SymbolKind::Struct);
        let caller = CallInfo {
            symbol_name: "main".to_string(),
            kind: SymbolKind::Function,
            file_path: PathBuf::from("/project/src/main.rs"),
            line: 20,
        };
        let ctx = SymbolContext {
            symbol_name: "MyStruct".to_string(),
            definitions: vec![def],
            references: vec![],
            callees: vec![],
            callers: vec![caller],
            extends: vec![],
            implements: vec![],
            extended_by: vec![],
            implemented_by: vec![],
        };
        let output = format_context_to_string(&[ctx], &root, None);

        // Must NOT contain old section delimiters
        assert!(!output.contains("--- "), "output should not contain '--- ' delimiter lines");
        // Must NOT contain old symbol prefix
        assert!(!output.contains("symbol "), "output should not contain 'symbol ' prefix");
        // Must NOT contain old summary
        assert!(!output.contains(" symbols"), "output should not contain 'N symbols' summary");
        // Must NOT contain old "called-by " prefix
        assert!(
            !output.contains("called-by "),
            "output should not contain 'called-by ' prefix"
        );
        // Symbol name as bare header
        assert!(
            output.contains("MyStruct"),
            "output should contain symbol name, got: {output}"
        );
        // Definition in compact format: path:line kind
        assert!(
            output.contains("src/lib.rs:5 struct"),
            "output should contain definition in compact format, got: {output}"
        );
        // Caller in compact format: caller_name path:line
        assert!(
            output.contains("main src/main.rs:20"),
            "output should contain caller in compact format, got: {output}"
        );
    }

    #[test]
    fn test_truncation_format() {
        // Verify the truncation prefix string format used in server handlers.
        let limit = 20usize;
        let total = 45usize;
        let formatted_output = "src/foo.ts:10 MyFunc function\n";
        let truncated_output =
            format!("truncated: {}/{}\n{}", limit, total, formatted_output);

        assert!(
            truncated_output.starts_with("truncated: 20/45\n"),
            "truncated output should start with 'truncated: N/total\\n', got: {truncated_output}"
        );
        assert!(
            truncated_output.contains("src/foo.ts:10 MyFunc function"),
            "truncated output should include formatted results"
        );
    }

    // ---------------------------------------------------------------------------
    // Section scoping tests (SCOPE-01, SCOPE-02)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parse_sections_none() {
        // None input -> None output (all sections, no filtering)
        let result = parse_sections(None);
        assert!(result.is_none(), "parse_sections(None) should return None");
    }

    #[test]
    fn test_parse_sections_single() {
        // 'r' maps to "references" only
        let result = parse_sections(Some("r")).expect("should return Some");
        assert_eq!(result.len(), 1, "should have exactly 1 entry");
        assert!(result.contains("references"), "should contain 'references'");
    }

    #[test]
    fn test_parse_sections_multiple() {
        // "r,c" and "rc" both produce {"references", "callers"}
        let with_comma = parse_sections(Some("r,c")).expect("should return Some");
        assert!(with_comma.contains("references"), "should contain 'references'");
        assert!(with_comma.contains("callers"), "should contain 'callers'");
        assert_eq!(with_comma.len(), 2, "should have exactly 2 entries");

        let without_sep = parse_sections(Some("rc")).expect("should return Some");
        assert!(without_sep.contains("references"), "should contain 'references'");
        assert!(without_sep.contains("callers"), "should contain 'callers'");
        assert_eq!(without_sep.len(), 2, "should have exactly 2 entries");
    }

    #[test]
    fn test_parse_sections_unknown_ignored() {
        // Unknown char 'z' is silently ignored; 'r' still maps
        let result = parse_sections(Some("rz")).expect("should return Some");
        assert!(result.contains("references"), "should contain 'references'");
        assert_eq!(result.len(), 1, "unknown 'z' should be silently ignored");
    }

    fn make_call_info(name: &str, path: &str, line: usize) -> crate::query::context::CallInfo {
        crate::query::context::CallInfo {
            symbol_name: name.to_string(),
            kind: crate::graph::node::SymbolKind::Function,
            file_path: PathBuf::from(path),
            line,
        }
    }

    fn make_ref_result(path: &str, kind: RefKind) -> RefResult {
        RefResult {
            file_path: PathBuf::from(path),
            ref_kind: kind,
            symbol_name: None,
            line: None,
        }
    }

    #[test]
    fn test_context_sections_filter_references_only() {
        let root = PathBuf::from("/test/project");
        let def = make_find_result("MyFunc", "/test/project/src/foo.rs", 10, SymbolKind::Function);
        let r = make_ref_result("/test/project/src/bar.rs", RefKind::Import);
        let caller = make_call_info("main", "/test/project/src/main.rs", 5);
        let ctx = SymbolContext {
            symbol_name: "MyFunc".to_string(),
            definitions: vec![def],
            references: vec![r],
            callees: vec![],
            callers: vec![caller],
            extends: vec![],
            implements: vec![],
            extended_by: vec![],
            implemented_by: vec![],
        };
        let output = format_context_to_string(&[ctx], &root, Some("r"));

        // References must appear
        assert!(
            output.contains("src/bar.rs import"),
            "references should appear when 'r' requested, got: {output}"
        );
        // Definitions always included
        assert!(
            output.contains("src/foo.rs:10 function"),
            "definitions always included, got: {output}"
        );
        // Callers must NOT appear (filtered out)
        assert!(
            !output.contains("main src/main.rs"),
            "callers should NOT appear when only 'r' requested, got: {output}"
        );
        // Omitted line must mention callers (non-empty, filtered)
        assert!(
            output.contains("omitted: callers"),
            "omitted line should list 'callers', got: {output}"
        );
    }

    #[test]
    fn test_context_sections_definitions_always_included() {
        let root = PathBuf::from("/test/project");
        let def = make_find_result("MyFunc", "/test/project/src/foo.rs", 10, SymbolKind::Function);
        let ctx = SymbolContext {
            symbol_name: "MyFunc".to_string(),
            definitions: vec![def],
            references: vec![],
            callees: vec![],
            callers: vec![],
            extends: vec![],
            implements: vec![],
            extended_by: vec![],
            implemented_by: vec![],
        };
        // Request only callers — but definitions should still be rendered
        let output = format_context_to_string(&[ctx], &root, Some("c"));

        assert!(
            output.contains("src/foo.rs:10 function"),
            "definitions always included even when not in sections filter, got: {output}"
        );
    }

    #[test]
    fn test_context_sections_omitted_skips_empty() {
        let root = PathBuf::from("/test/project");
        let def = make_find_result("MyFunc", "/test/project/src/foo.rs", 10, SymbolKind::Function);
        let r = make_ref_result("/test/project/src/bar.rs", RefKind::Import);
        // callers is EMPTY
        let ctx = SymbolContext {
            symbol_name: "MyFunc".to_string(),
            definitions: vec![def],
            references: vec![r],
            callees: vec![],
            callers: vec![], // empty — should NOT appear in omitted
            extends: vec![],
            implements: vec![],
            extended_by: vec![],
            implemented_by: vec![],
        };
        // Request only references — callers is empty so should NOT appear in omitted
        let output = format_context_to_string(&[ctx], &root, Some("r"));

        assert!(
            !output.contains("callers"),
            "empty 'callers' section should not appear in omitted line, got: {output}"
        );
    }

    #[test]
    fn test_context_no_sections_returns_all() {
        let root = PathBuf::from("/test/project");
        let def = make_find_result("MyFunc", "/test/project/src/foo.rs", 10, SymbolKind::Function);
        let r = make_ref_result("/test/project/src/bar.rs", RefKind::Import);
        let caller = make_call_info("main", "/test/project/src/main.rs", 5);
        let callee = make_call_info("helper", "/test/project/src/lib.rs", 20);
        let ctx = SymbolContext {
            symbol_name: "MyFunc".to_string(),
            definitions: vec![def],
            references: vec![r],
            callees: vec![callee],
            callers: vec![caller],
            extends: vec![],
            implements: vec![],
            extended_by: vec![],
            implemented_by: vec![],
        };
        // sections=None means all sections
        let output = format_context_to_string(&[ctx], &root, None);

        // All non-empty sections must appear
        assert!(
            output.contains("src/bar.rs import"),
            "references should appear with no filter, got: {output}"
        );
        assert!(
            output.contains("main src/main.rs:5"),
            "callers should appear with no filter, got: {output}"
        );
        assert!(
            output.contains("helper src/lib.rs:20"),
            "callees should appear with no filter, got: {output}"
        );
        // No omitted line when no filter applied
        assert!(
            !output.contains("omitted:"),
            "omitted line should NOT appear when no filter, got: {output}"
        );
    }
}

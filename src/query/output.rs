use std::io::IsTerminal;
use std::path::Path;

use crate::query::structure::StructureNode;

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
        "py" => "Python",
        "go" => "Go",
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

/// Sort key for language grouping: Go < JavaScript < Python < Rust < TypeScript < Unknown.
fn language_sort_key(lang: &str) -> u8 {
    match lang {
        "Go" => 1,
        "JavaScript" => 2,
        "Python" => 3,
        "Rust" => 4,
        "TypeScript" => 5,
        _ => 6,
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
pub fn format_find_results(
    results: &[FindResult],
    format: &OutputFormat,
    project_root: &Path,
    symbol_name: &str,
) {
    let show_vis = any_non_private(results);
    let mixed = is_mixed_language(results, |r: &FindResult| r.file_path.as_path());

    // Sort results: by language first (for grouping), then file path, then line.
    // Only clone when sorting is needed to avoid unnecessary allocation.
    let sorted;
    let results_ref = if mixed {
        sorted = {
            let mut v = results.to_vec();
            v.sort_by(|a, b| {
                let la = language_of_file(&a.file_path);
                let lb = language_of_file(&b.file_path);
                language_sort_key(la)
                    .cmp(&language_sort_key(lb))
                    .then(a.file_path.cmp(&b.file_path))
                    .then(a.line.cmp(&b.line))
            });
            v
        };
        &sorted[..]
    } else {
        results
    };

    match format {
        OutputFormat::Compact => {
            let mut last_lang: Option<&'static str> = None;
            for r in results_ref {
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
            if results.is_empty() {
                println!("hint: no results found -- try a broader pattern or check spelling");
            } else {
                println!("hint: use refs {} to find all references", symbol_name);
            }
        }

        OutputFormat::Table => {
            let use_color = std::io::stdout().is_terminal();

            // Column widths: auto-sized to data (single pass).
            let (name_w, file_w) = results_ref.iter().fold((6usize, 4usize), |(nw, fw), r| {
                let file_len = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path)
                    .to_string_lossy()
                    .len();
                (nw.max(r.symbol_name.len()), fw.max(file_len))
            });

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
                for r in results_ref {
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
                for r in results_ref {
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
            let json_results: Vec<serde_json::Value> = results_ref
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
    // Total symbols minus Rust-specific, Python-specific, and Go-specific symbols indicates TS/JS presence.
    let rust_total = stats.rust_fns
        + stats.rust_structs
        + stats.rust_enums
        + stats.rust_traits
        + stats.rust_impl_methods
        + stats.rust_type_aliases
        + stats.rust_consts
        + stats.rust_statics
        + stats.rust_macros;
    let non_rust_non_py_non_go = stats
        .symbol_count
        .saturating_sub(rust_total + stats.python_symbol_count + stats.go_symbol_count);
    non_rust_non_py_non_go > 0
        || stats.classes > stats.python_classes
        || stats.interfaces > stats.go_interfaces
        || stats.variables > stats.python_variables + stats.go_variables
        || stats.methods > stats.python_methods + stats.go_methods
        || stats.components > 0
}

/// Determine if the stats have Python symbols or files present.
fn stats_has_python(stats: &ProjectStats) -> bool {
    stats.python_file_count > 0 || stats.python_symbol_count > 0
}

/// Determine if the stats have Go symbols or files present.
fn stats_has_go(stats: &ProjectStats) -> bool {
    stats.go_file_count > 0 || stats.go_symbol_count > 0
}

/// Format and print project stats to stdout according to the selected output format.
///
/// `language_filter`: if Some("rust"), show only Rust section; if Some("typescript"),
/// show only TypeScript section; if Some("python"), show Python section; if None, show all.
pub fn format_stats(stats: &ProjectStats, format: &OutputFormat, language_filter: Option<&str>) {
    let show_rust = language_filter.is_none() || language_filter == Some("rust");
    let show_ts = language_filter.is_none()
        || language_filter == Some("typescript")
        || language_filter == Some("javascript");
    let show_python = language_filter.is_none() || language_filter == Some("python");
    let show_go = language_filter.is_none() || language_filter == Some("go");
    let show_totals = language_filter.is_none();

    let has_rust = stats_has_rust(stats);
    let has_ts = stats_has_ts_js(stats);
    let has_python = stats_has_python(stats);
    let has_go = stats_has_go(stats);

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
                // Subtract Rust-specific, Python, and Go symbols to get TS/JS-only counts.
                let ts_fns = stats
                    .functions
                    .saturating_sub(stats.rust_fns + stats.python_fns + stats.go_fns);
                let ts_classes = stats.classes.saturating_sub(stats.python_classes);
                let ts_enums = stats.enums.saturating_sub(stats.rust_enums);
                let ts_type_aliases = stats.type_aliases.saturating_sub(
                    stats.rust_type_aliases + stats.python_type_aliases + stats.go_type_aliases,
                );
                let ts_variables = stats
                    .variables
                    .saturating_sub(stats.python_variables + stats.go_variables);
                let ts_methods = stats
                    .methods
                    .saturating_sub(stats.python_methods + stats.go_methods);
                let rust_total = stats.rust_fns
                    + stats.rust_structs
                    + stats.rust_enums
                    + stats.rust_traits
                    + stats.rust_impl_methods
                    + stats.rust_type_aliases
                    + stats.rust_consts
                    + stats.rust_statics
                    + stats.rust_macros;
                let ts_total = stats
                    .symbol_count
                    .saturating_sub(rust_total + stats.python_symbol_count + stats.go_symbol_count);
                println!(
                    "TypeScript: {} symbols (function: {} class: {} interface: {} type: {} enum: {} variable: {} component: {} method: {} property: {})",
                    ts_total,
                    ts_fns,
                    ts_classes,
                    stats.interfaces,
                    ts_type_aliases,
                    ts_enums,
                    ts_variables,
                    stats.components,
                    ts_methods,
                    stats.properties,
                );
                println!(
                    "imports {} external {} unresolved {}",
                    stats.import_edges, stats.external_packages, stats.unresolved_imports,
                );
            }
            if show_python && has_python {
                println!(
                    "Python: {} files, {} symbols (fn: {} class: {} method: {} type: {} variable: {})",
                    stats.python_file_count,
                    stats.python_symbol_count,
                    stats.python_fns,
                    stats.python_classes,
                    stats.python_methods,
                    stats.python_type_aliases,
                    stats.python_variables,
                );
            }
            if show_go && has_go {
                println!(
                    "Go: {} files, {} symbols (fn: {} struct: {} interface: {} method: {} const: {} var: {} type: {})",
                    stats.go_file_count,
                    stats.go_symbol_count,
                    stats.go_fns,
                    stats.go_structs,
                    stats.go_interfaces,
                    stats.go_methods,
                    stats.go_consts,
                    stats.go_variables,
                    stats.go_type_aliases,
                );
            }
            if show_totals && (has_rust || has_ts || has_python || has_go) {
                let language_count = [has_rust, has_ts, has_python, has_go]
                    .iter()
                    .filter(|&&x| x)
                    .count();
                if language_count > 1 {
                    println!("---");
                    println!(
                        "Total: {} files, {} symbols",
                        stats.file_count, stats.symbol_count
                    );
                } else {
                    println!("files {}", stats.file_count);
                    println!("symbols {}", stats.symbol_count);
                }
            } else if show_totals {
                println!("files {}", stats.file_count);
                println!("symbols {}", stats.symbol_count);
            }
            // Fallback: show full stats if no language-specific sections match
            if !has_rust && !has_ts && !has_python && !has_go {
                println!("files {}", stats.file_count);
                println!("symbols {}", stats.symbol_count);
                println!(
                    "imports {} external {} unresolved {}",
                    stats.import_edges, stats.external_packages, stats.unresolved_imports
                );
            }
            println!("hint: use dead-code to find unreferenced symbols");
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
                // Subtract both Rust-specific and Python symbols to get TS/JS-only counts.
                let ts_fns = stats
                    .functions
                    .saturating_sub(stats.rust_fns + stats.python_fns);
                let ts_classes = stats.classes.saturating_sub(stats.python_classes);
                let ts_enums = stats.enums.saturating_sub(stats.rust_enums);
                let ts_type_aliases = stats
                    .type_aliases
                    .saturating_sub(stats.rust_type_aliases + stats.python_type_aliases);
                let ts_variables = stats.variables.saturating_sub(stats.python_variables);
                let ts_methods = stats.methods.saturating_sub(stats.python_methods);
                println!("{}", header("--- TypeScript/JavaScript ---"));
                println!("  Functions:    {}", ts_fns);
                println!("  Classes:      {}", ts_classes);
                println!("  Interfaces:   {}", stats.interfaces);
                println!("  Type Aliases: {}", ts_type_aliases);
                println!("  Enums:        {}", ts_enums);
                println!("  Variables:    {}", ts_variables);
                println!("  Components:   {}", stats.components);
                println!("  Methods:      {}", ts_methods);
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

            // Python section — only when Python symbols/files are present and filter allows
            if show_python && has_python {
                println!();
                println!("{}", header("--- Python ---"));
                println!("  Files:        {}", stats.python_file_count);
                println!("  Symbols:      {}", stats.python_symbol_count);
                println!("  Functions:    {}", stats.python_fns);
                println!("  Classes:      {}", stats.python_classes);
                println!("  Methods:      {}", stats.python_methods);
                println!("  Type Aliases: {}", stats.python_type_aliases);
                println!("  Variables:    {}", stats.python_variables);
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
                "python_file_count": stats.python_file_count,
                "python_symbol_count": stats.python_symbol_count,
                "python_fns": stats.python_fns,
                "python_classes": stats.python_classes,
                "python_methods": stats.python_methods,
                "python_type_aliases": stats.python_type_aliases,
                "python_variables": stats.python_variables,
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
pub fn format_refs_results(
    results: &[RefResult],
    format: &OutputFormat,
    project_root: &Path,
    symbol_name: &str,
) {
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
            if results.is_empty() {
                println!("hint: no results found -- try a broader pattern or check spelling");
            } else {
                println!("hint: use impact {} to see blast radius", symbol_name);
            }
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
    symbol_name: &str,
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
                    println!(
                        "{}impact {} [{}: {}]",
                        indent,
                        rel.display(),
                        r.confidence,
                        r.basis
                    );
                }
            } else {
                for r in results {
                    let rel = r
                        .file_path
                        .strip_prefix(project_root)
                        .unwrap_or(&r.file_path);
                    println!("impact {} [{}: {}]", rel.display(), r.confidence, r.basis);
                }
            }
            println!("{} files affected", results.len());
            if results.is_empty() {
                println!("hint: no results found -- try a broader pattern or check spelling");
            } else {
                println!(
                    "hint: use context {} for full dependency picture",
                    symbol_name
                );
            }
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
                    "\x1b[1m{:>5}  {:<file_w$}  {:<10}  BASIS\x1b[0m",
                    "DEPTH",
                    "FILE",
                    "CONFIDENCE",
                    file_w = file_w,
                );
            } else {
                println!(
                    "{:>5}  {:<file_w$}  {:<10}  BASIS",
                    "DEPTH",
                    "FILE",
                    "CONFIDENCE",
                    file_w = file_w,
                );
            }
            println!("{}", "-".repeat(file_w + 8 + 14 + 20));

            for r in results {
                let rel = r
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&r.file_path);
                println!(
                    "{:>5}  {:<file_w$}  {:<10}  {}",
                    r.depth,
                    rel.display(),
                    r.confidence.to_string(),
                    r.basis,
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
                        "confidence": r.confidence.to_string(),
                        "basis": r.basis,
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
    symbol_name: &str,
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
            if contexts.is_empty() {
                println!("hint: no results found -- try a broader pattern or check spelling");
            } else {
                println!(
                    "hint: use flow {} <target> to trace data paths",
                    symbol_name
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
// String-returning formatters (siblings of the println!-based CLI formatters)
// ---------------------------------------------------------------------------

/// Format find results to a String in compact prefix-free format for CLI output.
///
/// No summary line. No "def " prefix. Line format: `{rel_path}:{line} {symbol_name} {kind}`
/// (with optional visibility suffix for Rust). In mixed-language results, groups by language
/// with `--- {Language} ---` section headers.
#[cfg(test)]
pub fn format_find_to_string(
    results: &[FindResult],
    project_root: &Path,
    symbol_name: &str,
) -> String {
    use std::fmt::Write;
    let show_vis = any_non_private(results);
    let mixed = is_mixed_language(results, |r: &FindResult| r.file_path.as_path());

    // Only clone when sorting is needed to avoid unnecessary allocation.
    let sorted;
    let results_ref = if mixed {
        sorted = {
            let mut v = results.to_vec();
            v.sort_by(|a, b| {
                let la = language_of_file(&a.file_path);
                let lb = language_of_file(&b.file_path);
                language_sort_key(la)
                    .cmp(&language_sort_key(lb))
                    .then(a.file_path.cmp(&b.file_path))
                    .then(a.line.cmp(&b.line))
            });
            v
        };
        &sorted[..]
    } else {
        results
    };

    let mut buf = String::new();
    let mut last_lang: Option<&'static str> = None;
    for r in results_ref {
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
        let line_range = if r.line_end > r.line {
            format!("L{}-L{}", r.line, r.line_end)
        } else {
            format!("L{}", r.line)
        };
        if show_vis {
            writeln!(
                buf,
                "{}:{} {} {} {}",
                rel.display(),
                line_range,
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
                line_range,
                r.symbol_name,
                kind_to_str(&r.kind)
            )
            .unwrap();
        }
    }
    if results.is_empty() {
        writeln!(
            buf,
            "hint: no results found -- try a broader pattern or check spelling"
        )
        .unwrap();
    } else {
        writeln!(buf, "hint: use refs {} to find all references", symbol_name).unwrap();
    }
    buf
}

/// Format find_by_decorator results to a String for CLI output.
///
/// Each result is formatted as:
/// `@decorator_name[args] symbol_name (kind) file:line`
/// Followed by `  framework: <fw>` if a framework label is available.
pub fn format_decorator_to_string(
    results: &[crate::query::decorators::DecoratorMatch],
    project_root: &Path,
    limit: usize,
) -> String {
    use std::fmt::Write;
    if results.is_empty() {
        return "No decorated symbols found.".to_string();
    }
    let mut out = String::new();
    for r in results {
        let rel_path = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        let kind_str = kind_to_str(&r.kind);
        // Build decorator suffix: name + optional args
        let args_str = r
            .decorator_args
            .as_deref()
            .map(|a| a.to_string())
            .unwrap_or_default();
        writeln!(
            out,
            "@{}{} {} {} {}:{}",
            r.decorator_name,
            args_str,
            r.symbol_name,
            kind_str,
            rel_path.display(),
            r.line,
        )
        .unwrap();
        if let Some(ref fw) = r.framework {
            writeln!(out, "  framework: {fw}").unwrap();
        }
    }
    if results.len() >= limit {
        writeln!(out, "… truncated at {} results", limit).unwrap();
    }
    out
}

/// Format reference results to a String in compact prefix-free format for CLI output.
///
/// No summary line. No "ref " prefix. Line formats:
/// - Import: `{rel_path} import`
/// - Call:   `{rel_path}:{line} call {caller_name}`
#[cfg(test)]
pub fn format_refs_to_string(
    results: &[RefResult],
    project_root: &Path,
    symbol_name: &str,
) -> String {
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
    if results.is_empty() {
        writeln!(
            buf,
            "hint: no results found -- try a broader pattern or check spelling"
        )
        .unwrap();
    } else {
        writeln!(buf, "hint: use impact {} to see blast radius", symbol_name).unwrap();
    }
    buf
}

/// Format impact (blast radius) results to a String in compact prefix-free flat format for CLI output.
///
/// No summary line. No "impact " prefix. Line format: `{rel_path} (depth N) [TIER: basis]`.
/// Uses flat (non-tree) format — flat format is more token-efficient.
#[cfg(test)]
pub fn format_impact_to_string(
    results: &[ImpactResult],
    project_root: &Path,
    symbol_name: &str,
) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    for r in results {
        let rel = r
            .file_path
            .strip_prefix(project_root)
            .unwrap_or(&r.file_path);
        writeln!(
            buf,
            "{} (depth {}) [{}: {}]",
            rel.display(),
            r.depth,
            r.confidence,
            r.basis
        )
        .unwrap();
    }
    if results.is_empty() {
        writeln!(
            buf,
            "hint: no results found -- try a broader pattern or check spelling"
        )
        .unwrap();
    } else {
        writeln!(
            buf,
            "hint: use context {} for full dependency picture",
            symbol_name
        )
        .unwrap();
    }
    buf
}

/// Format circular dependency results to a String in compact prefix-free format for CLI output.
///
/// No summary line. No "cycle " prefix. Line format: `{file1} -> {file2} -> {file3}`.
#[cfg(test)]
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
    if cycles.is_empty() {
        writeln!(
            buf,
            "hint: no results found -- try a broader pattern or check spelling"
        )
        .unwrap();
    } else {
        let first_file = cycles[0]
            .files
            .first()
            .map(|p| {
                p.strip_prefix(project_root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default();
        writeln!(
            buf,
            "hint: use file-summary {} to understand the circular dependency",
            first_file
        )
        .unwrap();
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
#[cfg(test)]
pub fn parse_sections(sections: Option<&str>) -> Option<std::collections::HashSet<&'static str>> {
    let s = sections?;
    let mut set = std::collections::HashSet::new();
    for ch in s.chars() {
        match ch {
            'r' => {
                set.insert("references");
            }
            'c' => {
                set.insert("callers");
            }
            'e' => {
                set.insert("callees");
            }
            'x' => {
                set.insert("extends");
            }
            'i' => {
                set.insert("implements");
            }
            'X' => {
                set.insert("extended-by");
            }
            'I' => {
                set.insert("implemented-by");
            }
            _ => {} // separators (comma, space) and unknown chars silently ignored
        }
    }
    Some(set)
}

/// Format symbol context results to a String in compact prefix-free format for CLI output.
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
#[cfg(test)]
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
            // Show decorators if present (e.g. "@Controller @Injectable")
            if !def.decorators.is_empty() {
                let decorator_str: Vec<String> = def
                    .decorators
                    .iter()
                    .map(|d| format!("@{}", d.name))
                    .collect();
                writeln!(buf, "{}", decorator_str.join(" ")).unwrap();
            }
            // Show line range (L5-L20) if line_end > line, else just line
            let line_range = if def.line_end > def.line {
                format!("L{}-L{}", def.line, def.line_end)
            } else {
                format!("L{}", def.line)
            };
            writeln!(
                buf,
                "{}:{} {}",
                rel.display(),
                line_range,
                kind_to_str(&def.kind)
            )
            .unwrap();
        }

        // Track non-empty sections that were filtered out.
        let mut omitted: Vec<&'static str> = Vec::new();

        // References
        if active.as_ref().is_none_or(|s| s.contains("references")) {
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
        if active.as_ref().is_none_or(|s| s.contains("callers")) {
            for caller in &ctx.callers {
                let rel = caller
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&caller.file_path);
                writeln!(
                    buf,
                    "{} {}:{}",
                    caller.symbol_name,
                    rel.display(),
                    caller.line
                )
                .unwrap();
            }
        } else if !ctx.callers.is_empty() {
            omitted.push("callers");
        }

        // Callees
        if active.as_ref().is_none_or(|s| s.contains("callees")) {
            for callee in &ctx.callees {
                let rel = callee
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&callee.file_path);
                writeln!(
                    buf,
                    "{} {}:{}",
                    callee.symbol_name,
                    rel.display(),
                    callee.line
                )
                .unwrap();
            }
        } else if !ctx.callees.is_empty() {
            omitted.push("callees");
        }

        // Extends
        if active.as_ref().is_none_or(|s| s.contains("extends")) {
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
        if active.as_ref().is_none_or(|s| s.contains("implements")) {
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
        if active.as_ref().is_none_or(|s| s.contains("extended-by")) {
            for ext_by in &ctx.extended_by {
                let rel = ext_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&ext_by.file_path);
                writeln!(
                    buf,
                    "{} {}:{}",
                    ext_by.symbol_name,
                    rel.display(),
                    ext_by.line
                )
                .unwrap();
            }
        } else if !ctx.extended_by.is_empty() {
            omitted.push("extended-by");
        }

        // Implemented-by
        if active.as_ref().is_none_or(|s| s.contains("implemented-by")) {
            for impl_by in &ctx.implemented_by {
                let rel = impl_by
                    .file_path
                    .strip_prefix(project_root)
                    .unwrap_or(&impl_by.file_path);
                writeln!(
                    buf,
                    "{} {}:{}",
                    impl_by.symbol_name,
                    rel.display(),
                    impl_by.line
                )
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
    if contexts.is_empty() {
        writeln!(
            buf,
            "hint: no results found -- try a broader pattern or check spelling"
        )
        .unwrap();
    } else {
        writeln!(
            buf,
            "hint: use flow {} <target> to trace data paths",
            &contexts[0].symbol_name
        )
        .unwrap();
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
            if cycles.is_empty() {
                println!("hint: no results found -- try a broader pattern or check spelling");
            } else {
                // Suggest investigating the first file in the first cycle.
                let first_file = cycles[0]
                    .files
                    .first()
                    .map(|p| {
                        p.strip_prefix(project_root)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .to_string()
                    })
                    .unwrap_or_default();
                println!(
                    "hint: use file-summary {} to understand the circular dependency",
                    first_file
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
// Structure formatter
// ---------------------------------------------------------------------------

/// Render a structure tree to an indented compact string.
///
/// Format:
/// ```text
/// src/
///   cache/
///     loader.rs
///       pub load_or_build (fn)
///   query/
///     structure.rs
///       pub file_structure (fn)
/// README.md [doc]
/// Cargo.toml [config]
/// ```
///
/// Rules:
/// - 2 spaces per depth level.
/// - Directories end with `/`.
/// - Source files show symbols indented one level deeper.
/// - Non-parsed files show `[kind_tag]` after the filename.
/// - Symbols: visibility prefix (if pub or pub(crate)), then `name (kind)`.
/// - Truncation nodes render as `... (N more items)`.
/// - No trailing newline.
pub fn format_structure_to_string(tree: &[StructureNode], _root: &Path) -> String {
    let mut lines: Vec<String> = Vec::new();
    format_nodes(tree, 0, &mut lines);
    lines.join("\n")
}

fn format_nodes(nodes: &[StructureNode], depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for node in nodes {
        match node {
            StructureNode::Dir { name, children } => {
                lines.push(format!("{}{}/", indent, name));
                format_nodes(children, depth + 1, lines);
            }
            StructureNode::SourceFile { name, symbols } => {
                lines.push(format!("{}{}", indent, name));
                let sym_indent = "  ".repeat(depth + 1);
                for sym in symbols {
                    let prefix = match sym.visibility.as_str() {
                        "pub" => "pub ",
                        "pub(crate)" => "pub(crate) ",
                        _ => "",
                    };
                    lines.push(format!(
                        "{}{}{} ({})",
                        sym_indent, prefix, sym.name, sym.kind
                    ));
                }
            }
            StructureNode::NonParsedFile { name, kind_tag } => {
                lines.push(format!("{}{} [{}]", indent, name, kind_tag));
            }
            StructureNode::Truncated { count } => {
                lines.push(format!("{}... ({} more items)", indent, count));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FileSummary formatter
// ---------------------------------------------------------------------------

/// Render a `FileSummary` to a compact string (compact format, no trailing newline).
///
/// Format:
/// ```text
/// src/cache/loader.rs
/// role: utility
/// lines: 200
/// symbols: 3 (2 fn, 1 struct)
/// exports: load_or_build (fn), apply_staleness_diff (fn)
/// imports: 12
/// importers: 0
/// graph: leaf
/// ```
///
/// - `symbols:` shows total then parenthesized kind breakdown (only kinds with > 0 count).
/// - `exports:` lists ALL exported symbols — no truncation.
/// - `graph:` line is omitted if graph_label is None.
pub fn format_file_summary_to_string(summary: &crate::query::file_summary::FileSummary) -> String {
    use crate::query::file_summary::{FileRole, GraphLabel};

    let mut lines: Vec<String> = Vec::new();

    // Line 1: relative path
    lines.push(summary.relative_path.clone());

    // role:
    let role_str = match summary.role {
        FileRole::EntryPoint => "entry_point",
        FileRole::LibraryRoot => "library_root",
        FileRole::Test => "test",
        FileRole::Config => "config",
        FileRole::Types => "types",
        FileRole::Utility => "utility",
    };
    lines.push(format!("role: {}", role_str));

    // lines:
    lines.push(format!("lines: {}", summary.line_count));

    // symbols: N (breakdown by kind)
    if summary.symbol_count == 0 {
        lines.push("symbols: 0".to_string());
    } else if summary.symbol_kinds.is_empty() {
        lines.push(format!("symbols: {}", summary.symbol_count));
    } else {
        // Build sorted kind breakdown string (sorted alphabetically for determinism)
        let mut kinds: Vec<(&String, &usize)> = summary.symbol_kinds.iter().collect();
        kinds.sort_by_key(|(k, _)| k.as_str());
        let breakdown: String = kinds
            .iter()
            .map(|(k, count)| format!("{} {}", count, k))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("symbols: {} ({})", summary.symbol_count, breakdown));
    }

    // exports:
    if summary.exports.is_empty() {
        lines.push("exports: none".to_string());
    } else {
        let export_list: String = summary
            .exports
            .iter()
            .map(|e| format!("{} ({})", e.name, e.kind))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("exports: {}", export_list));
    }

    // imports: / importers:
    lines.push(format!("imports: {}", summary.import_count));
    lines.push(format!("importers: {}", summary.importer_count));

    // graph: (only if Some)
    if let Some(ref label) = summary.graph_label {
        let label_str = match label {
            GraphLabel::Hub => "hub",
            GraphLabel::Leaf => "leaf",
            GraphLabel::Bridge => "bridge",
        };
        lines.push(format!("graph: {}", label_str));
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Imports formatter
// ---------------------------------------------------------------------------

/// Render a list of `ImportEntry` items to a compact string (compact format, no trailing newline).
///
/// Format:
/// ```text
/// src/cache/loader.rs imports:
/// ./envelope (internal)
/// ../graph (internal)
/// ../parser (internal)
/// rayon (external)
/// std::sync (builtin)
/// crate::query::structure [re-export] (internal)
/// ```
///
/// - If no imports, shows `{file_path} imports: none`.
/// - `[re-export]` label only appears when `is_reexport` is true.
/// - Insertion order preserved (no sorting or grouping).
pub fn format_imports_to_string(
    entries: &[crate::query::imports::ImportEntry],
    file_path: &str,
) -> String {
    use crate::query::imports::ImportCategory;

    if entries.is_empty() {
        return format!("{} imports: none", file_path);
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{} imports:", file_path));

    for entry in entries {
        let category_str = match entry.category {
            ImportCategory::Internal => "internal",
            ImportCategory::Workspace => "workspace",
            ImportCategory::External => "external",
            ImportCategory::Builtin => "builtin",
        };
        if entry.is_reexport {
            lines.push(format!(
                "{} [re-export] ({})",
                entry.specifier, category_str
            ));
        } else {
            lines.push(format!("{} ({})", entry.specifier, category_str));
        }
    }

    lines.join("\n")
}

/// Format dead code analysis results to a compact string.
///
/// Output format:
/// ```text
/// unreachable files (N):
///   src/unused_module.rs
///   src/old_helper.ts
///
/// unreferenced symbols (N in M files):
/// src/utils/helpers.rs:
///   fn unused_helper :10
///   fn old_function :25
/// src/lib/parser.ts:
///   function deadFunc :42
/// ```
///
/// Paths are relative to `root`.
pub fn format_dead_code_to_string(
    result: &crate::query::dead_code::DeadCodeResult,
    root: &Path,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    // --- Unreachable files section ---
    let file_count = result.unreachable_files.len();
    lines.push(format!("unreachable files ({}):", file_count));
    if file_count == 0 {
        lines.push("  none".to_string());
    } else {
        for file_path in &result.unreachable_files {
            let rel = file_path.strip_prefix(root).unwrap_or(file_path);
            lines.push(format!("  {}", rel.display()));
        }
    }

    lines.push(String::new()); // blank line between sections

    // --- Unreferenced symbols section ---
    let total_symbols: usize = result
        .unreferenced_symbols
        .iter()
        .map(|(_, syms)| syms.len())
        .sum();
    let file_groups = result.unreferenced_symbols.len();

    lines.push(format!(
        "unreferenced symbols ({} in {} files):",
        total_symbols, file_groups
    ));

    if total_symbols == 0 {
        lines.push("  none".to_string());
    } else {
        for (file_path, syms) in &result.unreferenced_symbols {
            let rel = file_path.strip_prefix(root).unwrap_or(file_path);
            lines.push(format!("{}:", rel.display()));
            for sym in syms {
                lines.push(format!("  {} {} :{}", sym.kind, sym.name, sym.line));
            }
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Clone detection output
// ---------------------------------------------------------------------------

/// Format clone detection results as a compact string for CLI output (token-optimized).
///
/// Example:
/// ```text
/// Clone Groups (3 groups, 8 symbols analyzed):
/// group#1 (3 members): kind=function body=10 out=0 in=1 decorators=0
///   function process_data src/utils.rs:1 body=10
///   function transform_data src/helpers.rs:5 body=10
///   function convert_data src/convert.rs:1 body=10
/// ```
pub fn format_clones_to_string(
    result: &crate::query::clones::CloneGroupResult,
    root: &Path,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!(
        "Clone Groups ({} groups, {} symbols analyzed):",
        result.groups.len(),
        result.total_symbols_analyzed
    ));

    if result.groups.is_empty() {
        lines.push("  none detected".to_string());
    } else {
        for (i, group) in result.groups.iter().enumerate() {
            lines.push(format!(
                "group#{} ({} members): {}",
                i + 1,
                group.members.len(),
                group.signature
            ));
            for m in &group.members {
                let rel = m.file.strip_prefix(root).unwrap_or(&m.file);
                lines.push(format!(
                    "  {} {} {}:{} body={}",
                    m.kind,
                    m.name,
                    rel.display(),
                    m.line,
                    m.body_size,
                ));
            }
        }
    }

    lines.join("\n")
}

/// Format clone detection results as a human-readable table for CLI output.
///
/// Example:
/// ```text
/// Clone Groups (2 groups, 10 symbols analyzed)
///
/// Group #1 (3 members) -- kind=function body=10 out=0 in=1 decorators=0
///   KIND       NAME             FILE                LINE  BODY
///   function   process_data     src/utils.rs          1    10
///   function   transform_data   src/helpers.rs        5    10
/// ```
pub fn format_clones_table(result: &crate::query::clones::CloneGroupResult, root: &Path) -> String {
    let mut lines: Vec<String> = Vec::new();

    let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());

    if use_color {
        lines.push(format!(
            "\x1b[1mClone Groups ({} groups, {} symbols analyzed)\x1b[0m",
            result.groups.len(),
            result.total_symbols_analyzed
        ));
    } else {
        lines.push(format!(
            "Clone Groups ({} groups, {} symbols analyzed)",
            result.groups.len(),
            result.total_symbols_analyzed
        ));
    }

    if result.groups.is_empty() {
        lines.push(String::new());
        lines.push("  No structural clones detected.".to_string());
    } else {
        for (i, group) in result.groups.iter().enumerate() {
            lines.push(String::new());
            if use_color {
                lines.push(format!(
                    "\x1b[1mGroup #{} ({} members)\x1b[0m -- {}",
                    i + 1,
                    group.members.len(),
                    group.signature
                ));
            } else {
                lines.push(format!(
                    "Group #{} ({} members) -- {}",
                    i + 1,
                    group.members.len(),
                    group.signature
                ));
            }

            // Compute column widths
            let (name_w, file_w) = group.members.iter().fold((4usize, 4usize), |(nw, fw), m| {
                let file_len = m
                    .file
                    .strip_prefix(root)
                    .unwrap_or(&m.file)
                    .as_os_str()
                    .len();
                (nw.max(m.name.len()), fw.max(file_len))
            });

            if use_color {
                lines.push(format!(
                    "  \x1b[1m{:<12}  {:<name_w$}  {:<file_w$}  {:>4}  {:>4}\x1b[0m",
                    "KIND",
                    "NAME",
                    "FILE",
                    "LINE",
                    "BODY",
                    name_w = name_w,
                    file_w = file_w,
                ));
            } else {
                lines.push(format!(
                    "  {:<12}  {:<name_w$}  {:<file_w$}  {:>4}  {:>4}",
                    "KIND",
                    "NAME",
                    "FILE",
                    "LINE",
                    "BODY",
                    name_w = name_w,
                    file_w = file_w,
                ));
            }

            lines.push(format!(
                "  {}",
                "-".repeat(12 + 2 + name_w + 2 + file_w + 2 + 4 + 2 + 4)
            ));

            for m in &group.members {
                let rel = m.file.strip_prefix(root).unwrap_or(&m.file);
                lines.push(format!(
                    "  {:<12}  {:<name_w$}  {:<file_w$}  {:>4}  {:>4}",
                    m.kind,
                    m.name,
                    rel.display(),
                    m.line,
                    m.body_size,
                    name_w = name_w,
                    file_w = file_w,
                ));
            }
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Diff output
// ---------------------------------------------------------------------------

/// Format a GraphDiff as a compact string for CLI output.
///
/// Example:
/// ```text
/// files: +2 -1
/// +  src/new_module.rs
/// -  src/removed.rs
///
/// symbols: +3 -2 ~1
/// +  src/new_module.rs :: new_function
/// -  src/removed.rs :: old_function
/// ~  src/utils.rs :: parse_input (line 10 → 15, callers 3 → 5)
/// ```
pub fn format_diff_to_string(diff: &crate::query::diff::GraphDiff) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Files header
    lines.push(format!(
        "files: +{} -{}",
        diff.added_files.len(),
        diff.removed_files.len()
    ));
    for f in &diff.added_files {
        lines.push(format!("+  {}", f));
    }
    for f in &diff.removed_files {
        lines.push(format!("-  {}", f));
    }

    lines.push(String::new()); // blank separator

    // Symbols header
    lines.push(format!(
        "symbols: +{} -{} ~{}",
        diff.added_symbols.len(),
        diff.removed_symbols.len(),
        diff.modified_symbols.len()
    ));
    for (file, sym) in &diff.added_symbols {
        lines.push(format!("+  {} :: {}", file, sym));
    }
    for (file, sym) in &diff.removed_symbols {
        lines.push(format!("-  {} :: {}", file, sym));
    }
    for change in &diff.modified_symbols {
        let change_str = change.changes.join(", ");
        lines.push(format!(
            "~  {} :: {} ({})",
            change.file, change.name, change_str
        ));
    }

    lines.join("\n")
}
// ---------------------------------------------------------------------------
// Unit tests for compact formatters
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
            line_end: 0,
            col: 0,
            is_exported: false,
            is_default: false,
            visibility: SymbolVisibility::Private,
            decorators: Vec::new(),
        }
    }

    #[test]
    fn test_find_compact_format_no_prefix() {
        let root = PathBuf::from("/project");
        let results = vec![make_find_result(
            "MyFunc",
            "/project/src/foo.ts",
            10,
            SymbolKind::Function,
        )];
        let output = format_find_to_string(&results, &root, "MyFunc");

        // Must NOT contain old prefix
        assert!(
            !output.contains("def "),
            "output should not contain 'def ' prefix"
        );
        // Must NOT contain old summary line
        assert!(
            !output.contains("definitions found"),
            "output should not contain 'definitions found' summary"
        );
        // Must contain new compact format: path:L{line} name kind
        assert!(
            output.contains("src/foo.ts:L10 MyFunc function"),
            "output should contain compact format 'src/foo.ts:L10 MyFunc function', got: {output}"
        );
    }

    #[test]
    fn test_refs_compact_format_no_prefix() {
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
        let output = format_refs_to_string(&results, &root, "MySymbol");

        // Must NOT contain old prefix
        assert!(
            !output.contains("ref "),
            "output should not contain 'ref ' prefix"
        );
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
    fn test_impact_compact_format_no_prefix() {
        use crate::query::impact::ConfidenceTier;

        let root = PathBuf::from("/project");
        let results = vec![ImpactResult {
            file_path: PathBuf::from("/project/src/affected.ts"),
            depth: 1,
            confidence: ConfidenceTier::High,
            basis: "direct caller at depth 1".to_string(),
        }];
        let output = format_impact_to_string(&results, &root, "MySymbol");

        // Must NOT contain old prefix
        assert!(
            !output.contains("impact "),
            "output should not contain 'impact ' prefix"
        );
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
        // Must contain confidence tag
        assert!(
            output.contains("[HIGH: direct caller at depth 1]"),
            "output should contain confidence tag, got: {output}"
        );
    }

    #[test]
    fn test_format_impact_with_confidence() {
        use crate::query::impact::ConfidenceTier;

        let root = PathBuf::from("/project");
        let results = vec![ImpactResult {
            file_path: PathBuf::from("/project/src/affected.ts"),
            depth: 1,
            confidence: ConfidenceTier::High,
            basis: "direct caller at depth 1".to_string(),
        }];
        let output = format_impact_to_string(&results, &root, "MySymbol");

        assert!(
            output.contains("[HIGH: direct caller at depth 1]"),
            "output should contain '[HIGH: direct caller at depth 1]', got: {output}"
        );
        assert!(
            output.contains("src/affected.ts"),
            "output should contain file path, got: {output}"
        );
        assert!(
            output.contains("(depth 1)"),
            "output should contain depth info, got: {output}"
        );
    }

    #[test]
    fn test_circular_compact_format_no_prefix() {
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
        assert!(
            !output.contains("cycle "),
            "output should not contain 'cycle ' prefix"
        );
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
    fn test_context_compact_format_no_delimiters() {
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
        assert!(
            !output.contains("--- "),
            "output should not contain '--- ' delimiter lines"
        );
        // Must NOT contain old symbol prefix
        assert!(
            !output.contains("symbol "),
            "output should not contain 'symbol ' prefix"
        );
        // Must NOT contain old summary
        assert!(
            !output.contains(" symbols"),
            "output should not contain 'N symbols' summary"
        );
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
        // Definition in compact format: path:L{line} kind
        assert!(
            output.contains("src/lib.rs:L5 struct"),
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
        let truncated_output = format!("truncated: {}/{}\n{}", limit, total, formatted_output);

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
        assert!(
            with_comma.contains("references"),
            "should contain 'references'"
        );
        assert!(with_comma.contains("callers"), "should contain 'callers'");
        assert_eq!(with_comma.len(), 2, "should have exactly 2 entries");

        let without_sep = parse_sections(Some("rc")).expect("should return Some");
        assert!(
            without_sep.contains("references"),
            "should contain 'references'"
        );
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
        let def = make_find_result(
            "MyFunc",
            "/test/project/src/foo.rs",
            10,
            SymbolKind::Function,
        );
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
            output.contains("src/foo.rs:L10 function"),
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
        let def = make_find_result(
            "MyFunc",
            "/test/project/src/foo.rs",
            10,
            SymbolKind::Function,
        );
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
            output.contains("src/foo.rs:L10 function"),
            "definitions always included even when not in sections filter, got: {output}"
        );
    }

    #[test]
    fn test_context_sections_omitted_skips_empty() {
        let root = PathBuf::from("/test/project");
        let def = make_find_result(
            "MyFunc",
            "/test/project/src/foo.rs",
            10,
            SymbolKind::Function,
        );
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
        let def = make_find_result(
            "MyFunc",
            "/test/project/src/foo.rs",
            10,
            SymbolKind::Function,
        );
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

// ---------------------------------------------------------------------------
// Cluster / Flow / Rename string formatters (for CLI output)
// ---------------------------------------------------------------------------

use crate::query::clusters::ClusterResult;
use crate::query::flow::FlowResult;
use crate::query::rename::RenameItem;

/// Format cluster results as a human-readable string for CLI output.
///
/// Output format:
/// ```text
/// Functional Clusters (N groups):
/// auth (3 symbols): authenticate, authorize, hash_password
/// api (2 symbols): get_users, create_user
/// ```
pub fn format_clusters_to_string(clusters: &[ClusterResult]) -> String {
    if clusters.is_empty() {
        return "Functional Clusters (0 groups): none detected.".to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Functional Clusters ({} groups):", clusters.len()));

    for c in clusters {
        let top = if c.top_symbols.is_empty() {
            "(no symbols)".to_string()
        } else {
            c.top_symbols.join(", ")
        };
        lines.push(format!("{} ({} symbols): {}", c.label, c.member_count, top));
    }

    lines.join("\n")
}

/// Format flow trace results as a human-readable string for CLI output.
///
/// Output format (paths found):
/// ```text
/// Flow Trace: entry -> target
/// A -> B -> C (2 hops)
/// A -> D -> C (2 hops)
/// ```
///
/// Output format (no paths):
/// ```text
/// Flow Trace: entry -> target
/// No direct path found between entry and target.
/// Shared dependency: SomeSharedNode
/// ```
pub fn format_flow_to_string(result: &FlowResult, entry: &str, target: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Flow Trace: {} -> {}", entry, target));

    if result.paths.is_empty() {
        lines.push(format!(
            "No direct path found between {} and {}.",
            entry, target
        ));
        if let Some(ref shared) = result.shared_dependency {
            lines.push(format!("Shared dependency: {}", shared));
        }
    } else {
        for path in &result.paths {
            let chain = path.hops.join(" -> ");
            lines.push(format!("{} ({} hops)", chain, path.depth));
        }
    }

    lines.join("\n")
}

/// Format rename plan items as a human-readable string for CLI output.
///
/// Output format:
/// ```text
/// Rename Plan: Foo -> Bar (3 sites)
/// src/foo.rs:10  Foo -> Bar
/// src/bar.rs:5   Foo -> Bar
/// src/baz.rs:0   Foo -> Bar  [import site — verify manually]
/// ```
pub fn format_rename_to_string(items: &[RenameItem], root: &Path) -> String {
    if items.is_empty() {
        return "Rename Plan: no sites found — symbol not in graph.".to_string();
    }

    // Derive old/new from the first item (all items share the same old/new).
    let old_text = &items[0].old_text;
    let new_text = &items[0].new_text;

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Rename Plan: {} -> {} ({} sites)",
        old_text,
        new_text,
        items.len()
    ));

    for item in items {
        let rel = item.file_path.strip_prefix(root).unwrap_or(&item.file_path);
        let line_str = if item.line == 0 {
            "?".to_string()
        } else {
            item.line.to_string()
        };
        let note_str = item
            .note
            .as_deref()
            .map(|n| format!("  [{}]", n))
            .unwrap_or_default();
        lines.push(format!(
            "{}:{}  {} -> {}{}",
            rel.display(),
            line_str,
            item.old_text,
            item.new_text,
            note_str,
        ));
    }

    lines.join("\n")
}

/// Format diff-impact results as a human-readable string.
///
/// Used by the diff-impact CLI subcommand.
///
/// Output format:
/// ```text
/// ## src/foo.rs [HIGH] (5 affected files)
///   src/bar.rs (depth 1) [high: direct import]
///   src/baz.rs (depth 2) [medium: transitive]
/// ```
pub fn format_diff_impact_to_string(
    results: &[crate::query::impact::DiffImpactResult],
    root: &Path,
) -> String {
    use std::fmt::Write;
    let mut buf = String::new();

    for r in results {
        let rel = r.changed_file.strip_prefix(root).unwrap_or(&r.changed_file);
        writeln!(
            buf,
            "## {} [{}] ({} affected files)",
            rel.display(),
            r.risk,
            r.affected.len()
        )
        .unwrap();
        for a in &r.affected {
            let arel = a.file_path.strip_prefix(root).unwrap_or(&a.file_path);
            writeln!(
                buf,
                "  {} (depth {}) [{}: {}]",
                arel.display(),
                a.depth,
                a.confidence,
                a.basis
            )
            .unwrap();
        }
    }

    if buf.is_empty() {
        buf.push_str("No impact detected from changed files.");
    }
    buf
}

#[cfg(test)]
mod formatter_tests {
    use super::*;
    use std::path::PathBuf;

    use crate::query::clusters::ClusterResult;
    use crate::query::flow::{FlowPath, FlowResult};
    use crate::query::rename::RenameItem;

    #[test]
    fn test_format_clusters_to_string() {
        let clusters = vec![
            ClusterResult {
                label: "auth".to_string(),
                member_count: 3,
                top_symbols: vec![
                    "authenticate".to_string(),
                    "authorize".to_string(),
                    "hash_pw".to_string(),
                ],
            },
            ClusterResult {
                label: "api".to_string(),
                member_count: 2,
                top_symbols: vec!["get_users".to_string(), "create_user".to_string()],
            },
        ];

        let output = format_clusters_to_string(&clusters);

        assert!(
            output.contains("Functional Clusters (2 groups):"),
            "header line missing, got: {output}"
        );
        assert!(
            output.contains("auth"),
            "auth cluster missing in output: {output}"
        );
        assert!(
            output.contains("authenticate"),
            "top symbol missing in output: {output}"
        );
        assert!(
            output.contains("api"),
            "api cluster missing in output: {output}"
        );
        assert!(
            output.contains("get_users"),
            "api top symbol missing in output: {output}"
        );
        // Member counts
        assert!(
            output.contains("3 symbols"),
            "auth member count missing: {output}"
        );
        assert!(
            output.contains("2 symbols"),
            "api member count missing: {output}"
        );
    }

    #[test]
    fn test_format_flow_to_string() {
        let result = FlowResult {
            paths: vec![FlowPath {
                hops: vec!["A".to_string(), "B".to_string(), "C".to_string()],
                depth: 2,
            }],
            shared_dependency: None,
        };

        let output = format_flow_to_string(&result, "A", "C");

        assert!(
            output.contains("Flow Trace: A -> C"),
            "header missing in output: {output}"
        );
        assert!(
            output.contains("A -> B -> C"),
            "chain notation missing in output: {output}"
        );
        assert!(
            output.contains("2 hops"),
            "hop count missing in output: {output}"
        );
    }

    #[test]
    fn test_format_flow_to_string_no_path() {
        let result = FlowResult {
            paths: vec![],
            shared_dependency: Some("SharedDep".to_string()),
        };

        let output = format_flow_to_string(&result, "A", "Z");

        assert!(
            output.contains("No direct path found"),
            "no-path message missing: {output}"
        );
        assert!(
            output.contains("SharedDep"),
            "shared dependency missing: {output}"
        );
    }

    #[test]
    fn test_format_rename_to_string() {
        let root = PathBuf::from("/proj");
        let items = vec![
            RenameItem {
                file_path: root.join("src/foo.rs"),
                line: 10,
                old_text: "Foo".to_string(),
                new_text: "Bar".to_string(),
                note: None,
            },
            RenameItem {
                file_path: root.join("src/importer.rs"),
                line: 0,
                old_text: "Foo".to_string(),
                new_text: "Bar".to_string(),
                note: Some("import site — verify manually".to_string()),
            },
        ];

        let output = format_rename_to_string(&items, &root);

        assert!(
            output.contains("Rename Plan: Foo -> Bar (2 sites)"),
            "header missing: {output}"
        );
        assert!(
            output.contains("src/foo.rs"),
            "foo.rs path missing: {output}"
        );
        assert!(output.contains("10"), "line number missing: {output}");
        assert!(
            output.contains("import site"),
            "import site note missing: {output}"
        );
    }
}

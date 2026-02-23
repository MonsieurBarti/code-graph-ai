use serde::Serialize;

/// Aggregate statistics produced by an indexing run.
#[derive(Debug, Serialize)]
pub struct IndexStats {
    pub file_count: usize,
    pub functions: usize,
    pub classes: usize,
    pub interfaces: usize,
    pub type_aliases: usize,
    pub enums: usize,
    pub variables: usize,
    pub components: usize,
    pub methods: usize,
    pub properties: usize,
    pub imports: usize,
    /// Number of ESM static imports (`import ... from`).
    pub esm_imports: usize,
    /// Number of CommonJS require imports (`require(...)`).
    pub cjs_imports: usize,
    /// Number of dynamic imports (`import(...)`).
    pub dynamic_imports: usize,
    pub exports: usize,
    /// Files skipped due to read or parse errors.
    pub skipped: usize,
    /// Wall-clock time for the indexing run in seconds.
    pub elapsed_secs: f64,
    // Resolution metrics (Phase 2)
    /// Number of imports successfully resolved to a local indexed file.
    pub resolved_imports: usize,
    /// Number of imports that could not be resolved to any target.
    pub unresolved_imports: usize,
    /// Number of imports resolved to external packages (node_modules).
    pub external_packages: usize,
    /// Number of imports classified as Node.js built-in modules.
    pub builtin_modules: usize,
    /// Number of symbol-level relationship edges added (Calls, Extends, Implements, TypeRef).
    pub relationship_edges: usize,
    /// Number of TypeScript (.ts/.tsx) files discovered.
    pub ts_file_count: usize,
    /// Number of JavaScript (.js/.jsx) files discovered.
    pub js_file_count: usize,
    /// Number of Rust (.rs) files discovered and parsed.
    pub rust_file_count: usize,
    // Rust symbol counts (Phase 8)
    pub rust_fns: usize,
    pub rust_structs: usize,
    pub rust_enums: usize,
    pub rust_traits: usize,
    pub rust_impl_methods: usize,
    pub rust_type_aliases: usize,
    pub rust_consts: usize,
    pub rust_statics: usize,
    pub rust_macros: usize,
    pub rust_use_statements: usize,
    pub rust_pub_use_reexports: usize,
}

/// Print a summary of the indexing run.
///
/// - `json = true`: emit a pretty-printed JSON object to stdout.
/// - `json = false`: emit a cargo-style human-readable summary to stdout.
///
/// If `stats.skipped > 0`, a warning line is written to **stderr** so that
/// the stdout stream remains clean for downstream JSON consumers.
pub fn print_summary(stats: &IndexStats, json: bool) {
    if json {
        match serde_json::to_string_pretty(stats) {
            Ok(s) => println!("{}", s),
            Err(e) => eprintln!("error serialising stats: {}", e),
        }
        return;
    }

    // Human-readable cargo-style summary.
    println!(
        "Indexed {} files in {:.2}s",
        stats.file_count, stats.elapsed_secs
    );

    // Per-language file counts — only show languages with files present.
    if stats.ts_file_count > 0 {
        println!("  TypeScript: {} files", stats.ts_file_count);
    }
    if stats.js_file_count > 0 {
        println!("  JavaScript: {} files", stats.js_file_count);
    }
    if stats.rust_file_count > 0 {
        println!("  Rust: {} files", stats.rust_file_count);
        println!(
            "    {} fn, {} struct, {} enum, {} trait, {} impl method, {} type, {} const, {} static, {} macro",
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
            "    {} use statements (unresolved), {} pub use re-exports",
            stats.rust_use_statements, stats.rust_pub_use_reexports,
        );
    }

    println!(
        "  {} functions, {} classes, {} interfaces, {} types, {} enums, {} variables",
        stats.functions,
        stats.classes,
        stats.interfaces,
        stats.type_aliases,
        stats.enums,
        stats.variables,
    );
    println!(
        "  {} components, {} methods, {} properties",
        stats.components, stats.methods, stats.properties,
    );
    println!("  {} imports, {} exports", stats.imports, stats.exports);
    println!(
        "  {} ESM, {} CJS, {} dynamic imports",
        stats.esm_imports, stats.cjs_imports, stats.dynamic_imports,
    );

    // Resolution section.
    println!(
        "  Resolved {} imports ({} external, {} unresolved, {} builtins)",
        stats.resolved_imports,
        stats.external_packages,
        stats.unresolved_imports,
        stats.builtin_modules,
    );
    println!("  Added {} relationship edges", stats.relationship_edges);

    if stats.skipped > 0 {
        eprintln!("  {} files skipped (parse errors)", stats.skipped);
    }
}

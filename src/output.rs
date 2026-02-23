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

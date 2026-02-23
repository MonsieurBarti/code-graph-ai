pub mod imports;
pub mod languages;
pub mod relationships;
pub mod symbols;

use std::cell::RefCell;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tree_sitter::Parser;

use crate::graph::node::SymbolInfo;

use imports::{ExportInfo, ImportInfo, extract_exports, extract_imports, extract_rust_use};
use languages::language_for_extension;
use relationships::{RelationshipInfo, extract_relationships};
use symbols::{extract_impl_methods, extract_rust_symbols, extract_symbols};

// Thread-local Parser instances — one per rayon worker thread, zero lock contention.
// Each Parser is initialised once per thread with the appropriate grammar.
thread_local! {
    static PARSER_TS: RefCell<Parser> = RefCell::new({
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        p
    });
    static PARSER_TSX: RefCell<Parser> = RefCell::new({
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
        p
    });
    static PARSER_JS: RefCell<Parser> = RefCell::new({
        let mut p = Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        p
    });
    static PARSER_RS: RefCell<Parser> = RefCell::new({
        let mut p = Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        p
    });
}

/// Parsed information from a Rust `use` declaration.
pub struct RustUseInfo {
    /// Raw use path string as written in source (e.g. `"std::collections::HashMap"`).
    pub path: String,
    /// `true` for `pub use` re-exports, `false` for regular `use`.
    pub is_pub_use: bool,
}

/// The result of parsing a single source file.
///
/// - `symbols`: extracted top-level and child symbols (see [`extract_symbols`])
/// - `imports`: ESM / CJS / dynamic imports extracted from the file
/// - `exports`: named / default / re-export statements extracted from the file
/// - `relationships`: symbol-level relationships (calls, extends, implements, type refs)
/// - `rust_uses`: Rust `use`/`pub use` declarations (empty for TS/JS files)
///
/// Note: the tree-sitter `Tree` is NOT retained — ASTs are dropped after extraction
/// to keep RSS well under the 100 MB budget for large codebases (Phase 6 memory opt).
pub struct ParseResult {
    /// Each entry is `(parent_symbol, child_symbols)`.
    pub symbols: Vec<(SymbolInfo, Vec<SymbolInfo>)>,
    /// All imports found in the file (ESM, CJS, dynamic).
    pub imports: Vec<ImportInfo>,
    /// All standalone export statements found in the file.
    pub exports: Vec<ExportInfo>,
    /// All symbol-level relationships found in the file.
    /// Includes direct calls, method calls, class extends/implements, interface extends,
    /// and type annotation references.
    pub relationships: Vec<RelationshipInfo>,
    /// Rust `use` and `pub use` declarations. Always empty for TS/JS files.
    /// Phase 8 populates this for `.rs` files; Plan 02 adds actual extraction logic.
    pub rust_uses: Vec<RustUseInfo>,
}

/// Parse a source file and extract all symbols, imports, exports, and relationships.
///
/// Allocates a fresh `Parser` on every call — suitable for single-file incremental
/// watcher updates where the overhead is negligible.  For bulk parsing use
/// [`parse_file_parallel`] instead.
///
/// # Parameters
/// - `path`: path to the file (used for extension-based language selection)
/// - `source`: raw UTF-8 source bytes
///
/// # Errors
/// Returns an error if:
/// - The file extension is unsupported (not `.ts`/`.tsx`/`.js`/`.jsx`)
/// - `tree-sitter` returns `None` (malformed / truncated source)
pub fn parse_file(path: &Path, source: &[u8]) -> Result<ParseResult> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // "rs" arm: parse with a fresh parser and extract Rust symbols + use declarations.
    if ext == "rs" {
        let language = language_for_extension("rs").expect("rs language is always Some");
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .with_context(|| "failed to set tree-sitter language for extension \"rs\"")?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow!("tree-sitter returned None for {:?}", path))?;
        let top_level = extract_rust_symbols(&tree, source, &language);
        let impl_methods = extract_impl_methods(&tree, source);
        let rust_uses = extract_rust_use(&tree, source);
        let mut all_symbols = top_level;
        all_symbols.extend(impl_methods);
        return Ok(ParseResult {
            symbols: all_symbols,
            imports: Vec::new(),
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses,
        });
    }

    let language = language_for_extension(ext)
        .ok_or_else(|| anyhow!("unsupported file extension: {:?}", ext))?;

    let is_tsx = matches!(ext, "tsx" | "jsx");

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .with_context(|| format!("failed to set tree-sitter language for extension {:?}", ext))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter returned None for {:?}", path))?;

    let symbols = extract_symbols(&tree, source, &language, is_tsx);
    let imports = extract_imports(&tree, source, &language, is_tsx);
    let exports = extract_exports(&tree, source, &language, is_tsx);
    let relationships_vec = extract_relationships(&tree, source, &language, is_tsx);

    Ok(ParseResult {
        symbols,
        imports,
        exports,
        relationships: relationships_vec,
        rust_uses: Vec::new(),
    })
}

/// Parse a source file using thread-local Parser instances (for rayon parallel use).
///
/// Same as [`parse_file`] but reuses a per-thread Parser instead of allocating a new one.
/// The `thread_local!` pattern avoids lock contention — each rayon thread gets its own
/// Parser, initialised lazily on first use.
///
/// # Parameters
/// - `path`: path to the file (used for extension-based language selection)
/// - `source`: raw UTF-8 source bytes
///
/// # Errors
/// Returns an error if:
/// - The file extension is unsupported (not `.ts`/`.tsx`/`.js`/`.jsx`)
/// - `tree-sitter` returns `None` (malformed / truncated source)
pub fn parse_file_parallel(path: &Path, source: &[u8]) -> Result<ParseResult> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let is_tsx = matches!(ext, "tsx" | "jsx");

    // "rs" arm: parse with PARSER_RS and extract Rust symbols + use declarations.
    if ext == "rs" {
        let language = language_for_extension("rs").expect("rs language is always Some");
        let tree = PARSER_RS
            .with(|p| p.borrow_mut().parse(source, None))
            .ok_or_else(|| anyhow!("tree-sitter returned None for {:?}", path))?;
        let top_level = extract_rust_symbols(&tree, source, &language);
        let impl_methods = extract_impl_methods(&tree, source);
        let rust_uses = extract_rust_use(&tree, source);
        let mut all_symbols = top_level;
        all_symbols.extend(impl_methods);
        return Ok(ParseResult {
            symbols: all_symbols,
            imports: Vec::new(),
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses,
        });
    }

    let tree = match ext {
        "ts" => PARSER_TS.with(|p| p.borrow_mut().parse(source, None)),
        "tsx" => PARSER_TSX.with(|p| p.borrow_mut().parse(source, None)),
        "js" | "jsx" => PARSER_JS.with(|p| p.borrow_mut().parse(source, None)),
        _ => return Err(anyhow!("unsupported file extension: {:?}", ext)),
    };
    let tree = tree.ok_or_else(|| anyhow!("tree-sitter returned None for {:?}", path))?;

    let language = language_for_extension(ext)
        .ok_or_else(|| anyhow!("unsupported file extension: {:?}", ext))?;

    let symbols = extract_symbols(&tree, source, &language, is_tsx);
    let imports = extract_imports(&tree, source, &language, is_tsx);
    let exports = extract_exports(&tree, source, &language, is_tsx);
    let relationships_vec = extract_relationships(&tree, source, &language, is_tsx);

    Ok(ParseResult {
        symbols,
        imports,
        exports,
        relationships: relationships_vec,
        rust_uses: Vec::new(),
    })
}

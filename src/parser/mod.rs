pub mod imports;
pub mod languages;
pub mod relationships;
pub mod symbols;

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tree_sitter::{Parser, Tree};

use crate::graph::node::SymbolInfo;

use imports::{ExportInfo, ImportInfo, extract_exports, extract_imports};
use languages::language_for_extension;
use relationships::{RelationshipInfo, extract_relationships};
use symbols::extract_symbols;

/// The result of parsing a single source file.
///
/// - `symbols`: extracted top-level and child symbols (see [`extract_symbols`])
/// - `imports`: ESM / CJS / dynamic imports extracted from the file
/// - `exports`: named / default / re-export statements extracted from the file
/// - `relationships`: symbol-level relationships (calls, extends, implements, type refs)
/// - `tree`: the raw tree-sitter syntax tree (retained for debugging / future queries)
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
    /// The syntax tree — retained for debugging or future query passes.
    pub tree: Tree,
}

/// Parse a source file and extract all symbols, imports, exports, and relationships.
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
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

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
        tree,
    })
}

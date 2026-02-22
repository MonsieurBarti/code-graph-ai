pub mod languages;
pub mod symbols;

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tree_sitter::{Parser, Tree};

use crate::graph::node::SymbolInfo;

use languages::language_for_extension;
use symbols::extract_symbols;

/// The result of parsing a single source file.
///
/// - `symbols`: extracted top-level and child symbols (see [`extract_symbols`])
/// - `tree`: the raw tree-sitter syntax tree, kept for Plan 03 import/export extraction
pub struct ParseResult {
    /// Each entry is `(parent_symbol, child_symbols)`.
    pub symbols: Vec<(SymbolInfo, Vec<SymbolInfo>)>,
    /// The syntax tree — retained for later import/export query passes.
    pub tree: Tree,
}

/// Parse a source file and extract all symbols.
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

    Ok(ParseResult { symbols, tree })
}

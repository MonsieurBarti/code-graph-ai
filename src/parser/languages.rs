use tree_sitter::Language;

/// Return the tree-sitter [`Language`] for the given file extension, or `None` if the extension
/// is not supported.
///
/// # Grammar selection rules
/// - `.ts`       -> TypeScript grammar  (`LANGUAGE_TYPESCRIPT`)
/// - `.tsx`      -> TSX grammar         (`LANGUAGE_TSX`)
///   These MUST be different: the TypeScript grammar cannot parse JSX, and the TSX grammar
///   breaks angle-bracket type assertions (`<T>expr`). Mixing them causes parse errors.
/// - `.js`/`.jsx` -> JavaScript grammar (`LANGUAGE`)
pub fn language_for_extension(ext: &str) -> Option<Language> {
    match ext {
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    }
}

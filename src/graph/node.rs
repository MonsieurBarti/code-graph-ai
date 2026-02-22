use std::path::PathBuf;

/// The kind of symbol extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// A function declaration or top-level const arrow function.
    Function,
    /// A class declaration.
    Class,
    /// A TypeScript interface declaration.
    Interface,
    /// A TypeScript type alias declaration.
    TypeAlias,
    /// A TypeScript or JavaScript enum declaration.
    Enum,
    /// An exported variable (non-arrow-function const/let/var).
    Variable,
    /// A React component: a function that returns JSX (tagged in addition to being a function).
    Component,
    /// A class method or object literal method.
    Method,
    /// An interface property or method signature (child symbol of an interface).
    Property,
}

/// Metadata about a symbol extracted from source code.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// The symbol's identifier name.
    pub name: String,
    /// The kind/category of this symbol.
    pub kind: SymbolKind,
    /// 1-based line number where the symbol begins.
    pub line: usize,
    /// 0-based column offset where the symbol begins.
    pub col: usize,
    /// Whether the symbol is explicitly exported.
    pub is_exported: bool,
    /// Whether the symbol is a default export.
    pub is_default: bool,
}

/// Metadata about a source file.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// Canonical path to the file.
    pub path: PathBuf,
    /// The language grammar used: "typescript", "tsx", or "javascript".
    pub language: String,
}

/// A node in the code graph — either a file or a symbol within a file.
#[derive(Debug, Clone)]
pub enum GraphNode {
    File(FileInfo),
    Symbol(SymbolInfo),
}

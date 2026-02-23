use std::path::PathBuf;

/// The kind of symbol extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    /// Canonical path to the file.
    pub path: PathBuf,
    /// The language grammar used: "typescript", "tsx", or "javascript".
    pub language: String,
}

/// Metadata about an external package (node_modules dependency).
/// Internals are not indexed — external package nodes are terminal in the graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalPackageInfo {
    /// The npm package name (e.g. "react", "@org/utils").
    pub name: String,
    /// Package version, if available from package.json.
    pub version: Option<String>,
}

/// A node in the code graph — a file, a symbol within a file, an external package,
/// or an unresolved import.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphNode {
    /// A source file node.
    File(FileInfo),
    /// A symbol (function, class, interface, etc.) within a source file.
    Symbol(SymbolInfo),
    /// An external package node (node_modules dependency — internals not indexed).
    ExternalPackage(ExternalPackageInfo),
    /// An import specifier that could not be resolved to a file or known package.
    UnresolvedImport { specifier: String, reason: String },
}

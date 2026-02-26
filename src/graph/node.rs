use std::path::PathBuf;

/// Visibility level of a Rust symbol.
///
/// TypeScript/JavaScript symbols always use `Private` here; their export status is tracked
/// separately via `SymbolInfo::is_exported`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SymbolVisibility {
    /// `pub` — visible everywhere.
    Pub,
    /// `pub(crate)`, `pub(super)`, `pub(in path)` — all collapse to this variant.
    PubCrate,
    /// No visibility modifier (default in Rust).
    Private,
}

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
    /// A Rust struct declaration.
    Struct,
    /// A Rust trait declaration.
    Trait,
    /// A method inside a Rust impl block (named as `Type::method`).
    ImplMethod,
    /// A Rust const item.
    Const,
    /// A Rust static item.
    Static,
    /// A Rust macro_rules! definition.
    Macro,
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
    /// Rust visibility level. TypeScript/JavaScript symbols default to `Private`
    /// (they use `is_exported` instead).
    pub visibility: SymbolVisibility,
    /// For Rust impl methods: the trait name if this is a trait impl (e.g. `"Display"`).
    /// `None` for inherent impls and all TypeScript/JavaScript symbols.
    pub trait_impl: Option<String>,
}

/// Classification of a file's role in the project.
///
/// Source files have full symbol extraction and import resolution.
/// All other kinds are indexed as File nodes only (no symbols, no imports).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FileKind {
    /// Source code file with symbol extraction (ts, tsx, js, jsx, rs).
    Source,
    /// Documentation file (md, txt, rst, adoc).
    Doc,
    /// Configuration file (toml, yaml, yml, json, ini, env, cfg, xml, etc.).
    Config,
    /// CI/CD file (files in .github/, .gitlab/, .circleci/, or named Jenkinsfile).
    Ci,
    /// Asset file (images, fonts, media).
    Asset,
    /// Any other non-source file.
    Other,
}

impl Default for FileKind {
    fn default() -> Self {
        FileKind::Source
    }
}

/// Classify a file path into a `FileKind` based on its extension and path components.
///
/// CI classification is path-based (files inside `.github/`, `.gitlab/`, `.circleci/`).
/// All other classification is extension-based.
pub fn classify_file_kind(path: &std::path::Path) -> FileKind {
    // Check CI directories first (path-based, not extension-based)
    if path.components().any(|c| {
        let s = c.as_os_str().to_str().unwrap_or("");
        s == ".github" || s == ".gitlab" || s == ".circleci"
    }) {
        return FileKind::Ci;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        // Source files
        "ts" | "tsx" | "js" | "jsx" | "rs" => FileKind::Source,
        // Documentation
        "md" | "txt" | "rst" | "adoc" => FileKind::Doc,
        // Configuration
        "toml" | "yaml" | "yml" | "json" | "ini" | "env" | "cfg"
        | "conf" | "properties" | "xml" => FileKind::Config,
        // Assets
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "woff"
        | "woff2" | "ttf" | "eot" | "mp3" | "mp4" | "webm" | "pdf" => FileKind::Asset,
        // Special files by name
        _ => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            match name {
                "Dockerfile" | "Makefile" | "Jenkinsfile" | "Procfile" => FileKind::Config,
                ".gitlab-ci.yml" => FileKind::Ci,
                _ => FileKind::Other,
            }
        }
    }
}

/// Metadata about a source file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    /// Canonical path to the file.
    pub path: PathBuf,
    /// The language grammar used: "typescript", "tsx", "javascript", "rust", or empty for non-parsed.
    pub language: String,
    /// The owning crate's normalized name (hyphens replaced by underscores).
    ///
    /// `None` for TypeScript/JavaScript files; set during Rust indexing when
    /// the crate's Cargo.toml is parsed. Used for per-crate stats breakdowns.
    pub crate_name: Option<String>,
    /// Classification of this file's role (source, doc, config, ci, asset, other).
    pub kind: FileKind,
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
/// a Rust built-in crate, or an unresolved import.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphNode {
    /// A source file node.
    File(FileInfo),
    /// A symbol (function, class, interface, etc.) within a source file.
    Symbol(SymbolInfo),
    /// An external package node (node_modules dependency — internals not indexed).
    ExternalPackage(ExternalPackageInfo),
    /// A Rust built-in crate node: `std`, `core`, or `alloc`.
    ///
    /// Terminal node like `ExternalPackage` — traversal stops here.
    /// Deduplicated by name via `builtin_index` in `CodeGraph`.
    Builtin { name: String },
    /// An import specifier that could not be resolved to a file or known package.
    UnresolvedImport { specifier: String, reason: String },
}

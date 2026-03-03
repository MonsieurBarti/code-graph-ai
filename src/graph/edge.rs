/// The kind of directed edge between two nodes in the code graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EdgeKind {
    /// File -> Symbol: the file contains (declares) this symbol.
    Contains,
    /// File -> File: the source file imports from the target file.
    /// `specifier` is the raw import path string as written in source.
    Imports { specifier: String },
    /// File -> Symbol: the file explicitly exports this symbol.
    /// `name` is the exported name; `is_default` is true for default exports.
    Exports { name: String, is_default: bool },
    /// Symbol -> Symbol: a child symbol belongs to a parent symbol.
    /// Used for interface properties/method signatures and class methods.
    ChildOf,

    // Phase 2 additions:
    /// Resolved import edge: importing file -> resolved target file.
    /// specifier is the original raw import string from source.
    ResolvedImport { specifier: String },
    /// Symbol -> symbol: direct function/method call (foo() or obj.method()).
    Calls,
    /// Symbol -> symbol: class extends class, or interface extends interface.
    Extends,
    /// Symbol -> symbol: class implements interface.
    Implements,
    /// File -> file: barrel file re-exports everything from source (export * from './x').
    /// Resolved lazily at query time per user decision.
    BarrelReExportAll,

    // Phase 8 additions (Rust):
    /// Rust `pub use` re-export: source file -> unresolved target path.
    /// Created in Phase 8; resolved to an actual node in Phase 9.
    ReExport { path: String },
    /// Rust `use` statement (non-pub): unresolved import edge.
    /// `path` is the raw use path string. Resolution deferred to Phase 9.
    RustImport { path: String },

    // Phase 17 additions (Python):
    /// Python conditional import (e.g. `if TYPE_CHECKING:` block or try/except import).
    /// `specifier` is the raw import path string.
    ConditionalImport { specifier: String },

    // Phase 18 additions (Go):
    /// Go blank import (`import _ "pkg"`) — side-effect only import.
    SideEffectImport { specifier: String },
    /// Go dot import (`import . "pkg"`) — all exported names imported into scope.
    DotImport { specifier: String },
    /// Go struct embedding: `type Server struct { http.Handler }` — Server embeds Handler.
    Embeds,
    /// Symbol has a decorator/attribute. `name` is the decorator name.
    /// Used for graph-level "has any decorator" traversal queries.
    HasDecorator { name: String },
}

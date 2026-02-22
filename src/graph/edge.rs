/// The kind of directed edge between two nodes in the code graph.
#[derive(Debug, Clone)]
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
}

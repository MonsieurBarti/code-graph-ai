use std::sync::OnceLock;

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// The kind of import statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// ESM static import: `import { X } from './module'`
    ESM,
    /// CommonJS require: `const X = require('./module')`
    CJS,
    /// Dynamic import: `import('./module')`
    DynamicImport,
}

/// A single imported name from a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSpecifier {
    /// The local name used in this file.
    pub name: String,
    /// The alias (original name) when using `import { original as alias }`.
    pub alias: Option<String>,
    /// True for `import React from 'react'` (default import).
    pub is_default: bool,
    /// True for `import * as ns from 'module'` (namespace import).
    pub is_namespace: bool,
}

/// An import extracted from a source file.
#[derive(Debug, Clone)]
pub struct ImportInfo {
    /// Kind of import (ESM / CJS / dynamic).
    pub kind: ImportKind,
    /// The raw module specifier string, e.g. `"react"` or `"./utils"`.
    pub module_path: String,
    /// The names imported from the module.
    pub specifiers: Vec<ImportSpecifier>,
}

/// The kind of export statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportKind {
    /// `export { Foo, Bar }`
    Named,
    /// `export default X`
    Default,
    /// `export { X } from './module'`
    ReExport,
    /// `export * from './module'`
    ReExportAll,
}

/// An export extracted from a source file.
#[derive(Debug, Clone)]
pub struct ExportInfo {
    /// Kind of export.
    pub kind: ExportKind,
    /// The names being exported (empty for Default and ReExportAll).
    pub names: Vec<String>,
    /// The source module for re-exports.
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

/// Tree-sitter query for ESM static imports.
/// Matches `import { X } from 'module'`, `import X from 'module'`, `import * as X from 'module'`.
const IMPORT_QUERY_TS: &str = r#"
    (import_statement
      source: (string (string_fragment) @module_path)) @import
"#;

/// Tree-sitter query for CJS require calls.
/// Note: we do not use #eq? predicate here because tree-sitter 0.26 StreamingIterator
/// does not auto-filter custom predicates. We filter for "require" in code instead.
const REQUIRE_QUERY: &str = r#"
    (call_expression
      function: (identifier) @fn
      arguments: (arguments (string (string_fragment) @module_path)))
"#;

/// Tree-sitter query for dynamic import() calls.
const DYNAMIC_IMPORT_QUERY: &str = r#"
    (call_expression
      function: (import)
      arguments: (arguments (string (string_fragment) @module_path))) @dynamic_import
"#;

/// Tree-sitter query for export statements.
const EXPORT_QUERY: &str = r#"
    (export_statement) @export_stmt
"#;

// ---------------------------------------------------------------------------
// Query cache
// ---------------------------------------------------------------------------

static TS_IMPORT_QUERY: OnceLock<Query> = OnceLock::new();
static REQUIRE_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static DYNAMIC_IMPORT_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static EXPORT_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn import_query(language: &Language) -> &'static Query {
    TS_IMPORT_QUERY.get_or_init(|| {
        Query::new(language, IMPORT_QUERY_TS).expect("invalid import query")
    })
}

fn require_query(language: &Language) -> &'static Query {
    REQUIRE_QUERY_CACHE.get_or_init(|| {
        Query::new(language, REQUIRE_QUERY).expect("invalid require query")
    })
}

fn dynamic_import_query(language: &Language) -> &'static Query {
    DYNAMIC_IMPORT_QUERY_CACHE.get_or_init(|| {
        Query::new(language, DYNAMIC_IMPORT_QUERY).expect("invalid dynamic import query")
    })
}

fn export_query(language: &Language) -> &'static Query {
    EXPORT_QUERY_CACHE.get_or_init(|| {
        Query::new(language, EXPORT_QUERY).expect("invalid export query")
    })
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

/// Extract all ESM specifiers from an import_statement node.
///
/// Handles:
/// - Named: `import { useState, useEffect } from 'react'`
/// - Default: `import React from 'react'`
/// - Namespace: `import * as path from 'path'`
/// - Combined: `import React, { useState } from 'react'`
fn extract_esm_specifiers(import_node: Node, source: &[u8]) -> Vec<ImportSpecifier> {
    let mut specifiers = Vec::new();

    // Walk direct children of the import_statement to find import clauses.
    let mut cursor = import_node.walk();
    for child in import_node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                extract_import_clause(child, source, &mut specifiers);
            }
            "namespace_import" => {
                // `import * as ns from 'module'` (direct child, rare but handle it)
                if let Some(name) = extract_namespace_import_name(child, source) {
                    specifiers.push(ImportSpecifier {
                        name,
                        alias: None,
                        is_default: false,
                        is_namespace: true,
                    });
                }
            }
            _ => {}
        }
    }

    specifiers
}

/// Recursively extract specifiers from an `import_clause` node.
fn extract_import_clause(clause_node: Node, source: &[u8], specifiers: &mut Vec<ImportSpecifier>) {
    let mut cursor = clause_node.walk();
    for child in clause_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                // Default import: `import React from ...`
                specifiers.push(ImportSpecifier {
                    name: node_text(child, source).to_owned(),
                    alias: None,
                    is_default: true,
                    is_namespace: false,
                });
            }
            "named_imports" => {
                // Named: `{ useState, useEffect as UE }`
                extract_named_imports(child, source, specifiers);
            }
            "namespace_import" => {
                // `* as ns` — the identifier has no field name, just find the identifier child
                if let Some(name) = extract_namespace_import_name(child, source) {
                    specifiers.push(ImportSpecifier {
                        name,
                        alias: None,
                        is_default: false,
                        is_namespace: true,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Extract the identifier name from a `namespace_import` node (`* as identifier`).
/// The identifier is not assigned a field name in the grammar — find it by kind.
fn extract_namespace_import_name(ns_node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = ns_node.walk();
    for child in ns_node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(node_text(child, source).to_owned());
        }
    }
    None
}

/// Extract individual import_specifier nodes from a named_imports node.
fn extract_named_imports(
    named_imports_node: Node,
    source: &[u8],
    specifiers: &mut Vec<ImportSpecifier>,
) {
    let mut cursor = named_imports_node.walk();
    for child in named_imports_node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            // `name` field: the local binding name
            // `alias` field in tree-sitter is actually the "name" field (what was exported)
            // In `import { foo as bar }`: name="foo", alias="bar"
            // tree-sitter field: name -> original, alias -> local
            let name_node = child.child_by_field_name("name");
            let alias_node = child.child_by_field_name("alias");

            match (name_node, alias_node) {
                (Some(n), Some(a)) => {
                    // `import { foo as bar }` — n="foo", a="bar"
                    specifiers.push(ImportSpecifier {
                        name: node_text(a, source).to_owned(), // local binding
                        alias: Some(node_text(n, source).to_owned()), // original name
                        is_default: false,
                        is_namespace: false,
                    });
                }
                (Some(n), None) => {
                    specifiers.push(ImportSpecifier {
                        name: node_text(n, source).to_owned(),
                        alias: None,
                        is_default: false,
                        is_namespace: false,
                    });
                }
                _ => {}
            }
        }
    }
}

/// Find the variable name from a CJS require statement's parent variable_declarator.
/// e.g. `const fs = require('fs')` → "fs"
fn find_require_binding(call_node: Node, source: &[u8]) -> Option<String> {
    // Walk up to variable_declarator
    let mut current = call_node.parent();
    while let Some(n) = current {
        if n.kind() == "variable_declarator" {
            if let Some(name_node) = n.child_by_field_name("name") {
                return Some(node_text(name_node, source).to_owned());
            }
            break;
        }
        current = n.parent();
    }
    None
}

/// Extract all imports (ESM, CJS, dynamic) from a parsed syntax tree.
pub fn extract_imports(tree: &Tree, source: &[u8], language: &Language) -> Vec<ImportInfo> {
    let mut imports = Vec::new();

    // --- ESM static imports ---
    {
        let query = import_query(language);
        let module_path_idx = query
            .capture_index_for_name("module_path")
            .expect("import query must have @module_path");
        let import_idx = query
            .capture_index_for_name("import")
            .expect("import query must have @import");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let mut import_node: Option<Node> = None;
            let mut module_path: Option<String> = None;

            for capture in m.captures {
                if capture.index == import_idx {
                    import_node = Some(capture.node);
                } else if capture.index == module_path_idx {
                    module_path = Some(node_text(capture.node, source).to_owned());
                }
            }

            if let (Some(imp_node), Some(path)) = (import_node, module_path) {
                let specifiers = extract_esm_specifiers(imp_node, source);
                imports.push(ImportInfo {
                    kind: ImportKind::ESM,
                    module_path: path,
                    specifiers,
                });
            }
        }
    }

    // --- CJS require() calls ---
    {
        // The require query matches ALL call_expression(identifier, ...) patterns.
        // We filter for "require" in code (tree-sitter 0.26 StreamingIterator does not
        // auto-apply #eq? predicates).
        let query = require_query(language);
        let module_path_idx = match query.capture_index_for_name("module_path") {
            Some(idx) => idx,
            None => {
                // query compiled without expected captures — skip CJS
                return imports;
            }
        };
        let fn_idx = query.capture_index_for_name("fn");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let mut module_path: Option<String> = None;
            let mut call_node: Option<Node> = None;
            let mut fn_name: Option<String> = None;

            for capture in m.captures {
                if capture.index == module_path_idx {
                    module_path = Some(node_text(capture.node, source).to_owned());
                    call_node = Some(capture.node);
                } else if fn_idx == Some(capture.index) {
                    fn_name = Some(node_text(capture.node, source).to_owned());
                }
            }

            // Only process calls to `require(...)`, not arbitrary identifier calls.
            if fn_name.as_deref() != Some("require") {
                continue;
            }

            if let Some(path) = module_path {
                // Walk up to find the call_expression node for binding detection.
                let call_expr = call_node.and_then(|n| {
                    let mut c = Some(n);
                    while let Some(node) = c {
                        if node.kind() == "call_expression" {
                            return Some(node);
                        }
                        c = node.parent();
                    }
                    None
                });

                let mut specifiers = Vec::new();
                if let Some(call) = call_expr {
                    if let Some(binding) = find_require_binding(call, source) {
                        specifiers.push(ImportSpecifier {
                            name: binding,
                            alias: None,
                            is_default: false,
                            is_namespace: false,
                        });
                    }
                }

                imports.push(ImportInfo {
                    kind: ImportKind::CJS,
                    module_path: path,
                    specifiers,
                });
            }
        }
    }

    // --- Dynamic import() calls ---
    {
        let query = dynamic_import_query(language);
        let module_path_idx = query
            .capture_index_for_name("module_path")
            .expect("dynamic import query must have @module_path");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            let mut module_path: Option<String> = None;

            for capture in m.captures {
                if capture.index == module_path_idx {
                    module_path = Some(node_text(capture.node, source).to_owned());
                }
            }

            if let Some(path) = module_path {
                imports.push(ImportInfo {
                    kind: ImportKind::DynamicImport,
                    module_path: path,
                    specifiers: Vec::new(),
                });
            }
        }
    }

    imports
}

// ---------------------------------------------------------------------------
// Export extraction
// ---------------------------------------------------------------------------

/// Extract all exports from a parsed syntax tree.
pub fn extract_exports(tree: &Tree, source: &[u8], language: &Language) -> Vec<ExportInfo> {
    let mut exports = Vec::new();

    let query = export_query(language);
    let export_stmt_idx = query
        .capture_index_for_name("export_stmt")
        .expect("export query must have @export_stmt");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    while let Some(m) = matches.next() {
        let mut export_node: Option<Node> = None;

        for capture in m.captures {
            if capture.index == export_stmt_idx {
                export_node = Some(capture.node);
            }
        }

        if let Some(node) = export_node {
            if let Some(info) = classify_export(node, source) {
                exports.push(info);
            }
        }
    }

    exports
}

/// Classify a single export_statement node.
fn classify_export(node: Node, source: &[u8]) -> Option<ExportInfo> {
    // Check if this is a re-export (has a `source` field).
    let source_str = find_export_source(node, source);

    // Check for wildcard export: `export * from './module'`
    let has_star = (0..node.child_count()).any(|i| {
        node.child(i as u32)
            .map(|c| c.kind() == "*")
            .unwrap_or(false)
    });

    if has_star {
        // `export * from './module'`
        return Some(ExportInfo {
            kind: ExportKind::ReExportAll,
            names: Vec::new(),
            source: source_str,
        });
    }

    // Check for export_clause (named or re-export with names).
    let export_clause = find_child_of_kind(node, "export_clause");

    if let Some(clause) = export_clause {
        let names = extract_export_clause_names(clause, source);
        if source_str.is_some() {
            // `export { X } from './module'`
            return Some(ExportInfo {
                kind: ExportKind::ReExport,
                names,
                source: source_str,
            });
        } else {
            // `export { X, Y }`
            return Some(ExportInfo {
                kind: ExportKind::Named,
                names,
                source: None,
            });
        }
    }

    // Check for `default` keyword — `export default X`
    let has_default = (0..node.child_count()).any(|i| {
        node.child(i as u32)
            .map(|c| node_text(c, source) == "default")
            .unwrap_or(false)
    });

    if has_default {
        return Some(ExportInfo {
            kind: ExportKind::Default,
            names: Vec::new(),
            source: None,
        });
    }

    // Inline export (export function/class/etc.) — skip here; symbols.rs captures these.
    None
}

/// Find the source module string from a re-export statement.
/// e.g. `export { X } from './utils'` → Some("./utils")
fn find_export_source(export_node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = export_node.walk();
    for child in export_node.children(&mut cursor) {
        if child.kind() == "string" {
            // The string's first named child is string_fragment
            if let Some(frag) = child.named_child(0) {
                return Some(node_text(frag, source).to_owned());
            }
        }
    }
    None
}

/// Find the first direct child of `node` with the given kind.
fn find_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Extract the exported names from an export_clause node.
fn extract_export_clause_names(clause_node: Node, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = clause_node.walk();
    for child in clause_node.children(&mut cursor) {
        if child.kind() == "export_specifier" {
            // The `name` field holds the original name being exported.
            if let Some(name_node) = child.child_by_field_name("name") {
                names.push(node_text(name_node, source).to_owned());
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::languages::language_for_extension;

    fn parse_ts(source: &str) -> (tree_sitter::Tree, Language) {
        let lang = language_for_extension("ts").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    fn parse_js(source: &str) -> (tree_sitter::Tree, Language) {
        let lang = language_for_extension("js").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    // Test 1: ESM named imports
    #[test]
    fn test_esm_named_imports() {
        let src = "import { useState, useEffect } from 'react';";
        let (tree, lang) = parse_ts(src);
        let imports = extract_imports(&tree, src.as_bytes(), &lang);
        assert_eq!(imports.len(), 1, "should find 1 import");
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::ESM);
        assert_eq!(imp.module_path, "react");
        assert_eq!(imp.specifiers.len(), 2, "should have 2 specifiers");
        let names: Vec<_> = imp.specifiers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"useState"), "missing useState");
        assert!(names.contains(&"useEffect"), "missing useEffect");
        assert!(imp.specifiers.iter().all(|s| !s.is_default && !s.is_namespace));
    }

    // Test 2: ESM default import
    #[test]
    fn test_esm_default_import() {
        let src = "import React from 'react';";
        let (tree, lang) = parse_ts(src);
        let imports = extract_imports(&tree, src.as_bytes(), &lang);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::ESM);
        assert_eq!(imp.module_path, "react");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "React");
        assert!(imp.specifiers[0].is_default);
        assert!(!imp.specifiers[0].is_namespace);
    }

    // Test 3: ESM namespace import
    #[test]
    fn test_esm_namespace_import() {
        let src = "import * as path from 'path';";
        let (tree, lang) = parse_ts(src);
        let imports = extract_imports(&tree, src.as_bytes(), &lang);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::ESM);
        assert_eq!(imp.module_path, "path");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "path");
        assert!(imp.specifiers[0].is_namespace);
        assert!(!imp.specifiers[0].is_default);
    }

    // Test 4: CJS require
    #[test]
    fn test_cjs_require() {
        let src = "const fs = require('fs');";
        let (tree, lang) = parse_js(src);
        let imports = extract_imports(&tree, src.as_bytes(), &lang);
        assert_eq!(imports.len(), 1, "should find 1 import");
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::CJS);
        assert_eq!(imp.module_path, "fs");
    }

    // Test 5: Dynamic import
    #[test]
    fn test_dynamic_import() {
        let src = "const mod = await import('./lazy');";
        let (tree, lang) = parse_ts(src);
        let imports = extract_imports(&tree, src.as_bytes(), &lang);
        assert_eq!(imports.len(), 1, "should find 1 dynamic import");
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::DynamicImport);
        assert_eq!(imp.module_path, "./lazy");
    }

    // Test 6: Named export
    #[test]
    fn test_named_export() {
        let src = "export { foo, bar };";
        let (tree, lang) = parse_ts(src);
        let exports = extract_exports(&tree, src.as_bytes(), &lang);
        assert_eq!(exports.len(), 1, "should find 1 export");
        let exp = &exports[0];
        assert_eq!(exp.kind, ExportKind::Named);
        assert_eq!(exp.names.len(), 2, "should have 2 names");
        assert!(exp.names.contains(&"foo".to_string()));
        assert!(exp.names.contains(&"bar".to_string()));
        assert!(exp.source.is_none());
    }

    // Test 7: Default export
    #[test]
    fn test_default_export() {
        let src = "export default MyComponent;";
        let (tree, lang) = parse_ts(src);
        let exports = extract_exports(&tree, src.as_bytes(), &lang);
        assert_eq!(exports.len(), 1, "should find 1 export");
        let exp = &exports[0];
        assert_eq!(exp.kind, ExportKind::Default);
        assert!(exp.names.is_empty());
        assert!(exp.source.is_none());
    }

    // Test 8: Re-export
    #[test]
    fn test_reexport() {
        let src = "export { helper } from './utils';";
        let (tree, lang) = parse_ts(src);
        let exports = extract_exports(&tree, src.as_bytes(), &lang);
        assert_eq!(exports.len(), 1, "should find 1 re-export");
        let exp = &exports[0];
        assert_eq!(exp.kind, ExportKind::ReExport);
        assert!(exp.names.contains(&"helper".to_string()));
        assert_eq!(exp.source.as_deref(), Some("./utils"));
    }

    // Test 9: Re-export all
    #[test]
    fn test_reexport_all() {
        let src = "export * from './types';";
        let (tree, lang) = parse_ts(src);
        let exports = extract_exports(&tree, src.as_bytes(), &lang);
        assert_eq!(exports.len(), 1, "should find 1 re-export-all");
        let exp = &exports[0];
        assert_eq!(exp.kind, ExportKind::ReExportAll);
        assert!(exp.names.is_empty());
        assert_eq!(exp.source.as_deref(), Some("./types"));
    }

}

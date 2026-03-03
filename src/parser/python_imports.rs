use tree_sitter::{Node, Tree};

use crate::parser::imports::{ImportInfo, ImportKind, ImportSpecifier};

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Import extraction helpers
// ---------------------------------------------------------------------------

/// Extract an `import_statement` node: `import os` or `import os.path`.
///
/// In tree-sitter-python, `import_statement` has `name` fields, each a `dotted_name`
/// or `aliased_import`. Each `name` becomes a separate `ImportInfo`.
///
/// AST: `(import_statement name: (dotted_name ...))`
fn extract_import_statement(node: Node, source: &[u8]) -> Vec<ImportInfo> {
    let mut results = Vec::new();
    let line = node.start_position().row + 1;

    // Iterate over `name` field children
    let mut i = 0u32;
    while (i as usize) < node.child_count() {
        let child = node.child(i).unwrap();
        i += 1;
        match child.kind() {
            "dotted_name" => {
                let module_path = node_text(child, source).to_owned();
                results.push(ImportInfo {
                    kind: ImportKind::PythonAbsolute,
                    module_path: module_path.clone(),
                    specifiers: vec![ImportSpecifier {
                        name: module_path,
                        alias: None,
                        is_default: false,
                        is_namespace: false,
                    }],
                    line,
                });
            }
            "aliased_import" => {
                // `import os.path as p` or `import sys as system`
                let name_node = child.child_by_field_name("name");
                let alias_node = child.child_by_field_name("alias");
                let module_path = name_node
                    .map(|n| node_text(n, source).to_owned())
                    .unwrap_or_default();
                let local = alias_node
                    .map(|n| node_text(n, source).to_owned())
                    .unwrap_or_else(|| module_path.clone());
                results.push(ImportInfo {
                    kind: ImportKind::PythonAbsolute,
                    module_path: module_path.clone(),
                    specifiers: vec![ImportSpecifier {
                        name: local.clone(),
                        alias: if local != module_path {
                            Some(module_path)
                        } else {
                            None
                        },
                        is_default: false,
                        is_namespace: false,
                    }],
                    line,
                });
            }
            _ => {}
        }
    }

    results
}

/// Extract an `import_from_statement` node.
///
/// Actual tree-sitter-python 0.25 AST structure:
///
/// Absolute: `from os import path`
///   `(import_from_statement module_name: (dotted_name ...) name: (dotted_name ...))`
///
/// Relative with module: `from ..pkg import Foo`
///   `(import_from_statement module_name: (relative_import (import_prefix) (dotted_name ...)) name: (dotted_name ...))`
///
/// Relative without module: `from . import utils`
///   `(import_from_statement module_name: (relative_import (import_prefix)) name: (dotted_name ...))`
///
/// Multiple names: `from os import path, getcwd`
///   `(import_from_statement module_name: (dotted_name ...) name: (dotted_name ...) name: (dotted_name ...))`
///
/// Aliased: `from pkg import Foo as Bar`
///   `(import_from_statement module_name: (dotted_name ...) name: (aliased_import ...))`
///
/// Wildcard: `from module import *`
///   `(import_from_statement module_name: (dotted_name ...) (wildcard_import))`
fn extract_import_from_statement(
    node: Node,
    source: &[u8],
    is_conditional: bool,
) -> Option<ImportInfo> {
    let line = node.start_position().row + 1;

    let mut dot_count = 0usize;
    let mut module_name = String::new();
    let mut specifiers: Vec<ImportSpecifier> = Vec::new();
    let mut is_wildcard = false;

    // Extract module_name field
    if let Some(mod_node) = node.child_by_field_name("module_name") {
        match mod_node.kind() {
            "dotted_name" => {
                // Absolute import: module path is directly the dotted_name text
                module_name = node_text(mod_node, source).to_owned();
            }
            "relative_import" => {
                // Relative import: contains import_prefix (dots) and optional dotted_name
                let mut rel_cursor = mod_node.walk();
                for rel_child in mod_node.children(&mut rel_cursor) {
                    match rel_child.kind() {
                        "import_prefix" => {
                            let text = node_text(rel_child, source);
                            dot_count = text.chars().filter(|&c| c == '.').count();
                        }
                        "dotted_name" => {
                            module_name = node_text(rel_child, source).to_owned();
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                // Fallback
                module_name = node_text(mod_node, source).to_owned();
            }
        }
    }

    // Extract all `name` field children as specifiers
    // Note: tree-sitter allows multiple fields with same name by iterating children
    let mut i = 0u32;
    while (i as usize) < node.child_count() {
        let child = node.child(i).unwrap();
        i += 1;
        match child.kind() {
            "wildcard_import" => {
                is_wildcard = true;
            }
            "dotted_name" => {
                // This is a `name` field (specifier)
                // Check it's NOT the module_name (module_name is the first child with field name)
                // By checking the field name of this child's role in parent
                if let Some(field) = node.field_name_for_child(i - 1)
                    && field == "name"
                {
                    let name = node_text(child, source).to_owned();
                    specifiers.push(ImportSpecifier {
                        name,
                        alias: None,
                        is_default: false,
                        is_namespace: false,
                    });
                }
            }
            "aliased_import" => {
                // `from pkg import Foo as Bar`
                // field_name_for_child check is redundant here since aliased_import only appears as name
                collect_aliased_import(child, source, &mut specifiers);
            }
            _ => {}
        }
    }

    if is_wildcard {
        specifiers.push(ImportSpecifier {
            name: "*".to_owned(),
            alias: None,
            is_default: false,
            is_namespace: false,
        });
    }

    // Determine kind
    let kind = if dot_count > 0 {
        if is_conditional {
            ImportKind::PythonConditionalRelative { level: dot_count }
        } else {
            ImportKind::PythonRelative { level: dot_count }
        }
    } else if is_conditional {
        ImportKind::PythonConditionalAbsolute
    } else {
        ImportKind::PythonAbsolute
    };

    Some(ImportInfo {
        kind,
        module_path: module_name,
        specifiers,
        line,
    })
}

/// Collect an `aliased_import` node into the specifiers list.
///
/// In `from pkg import Foo as Bar`:
/// - `name` field = "Foo" (original name being imported)
/// - `alias` field = "Bar" (local binding name)
///
/// We store: `specifier.name = "Bar"` (local), `specifier.alias = Some("Foo")` (original)
fn collect_aliased_import(node: Node, source: &[u8], specifiers: &mut Vec<ImportSpecifier>) {
    let orig_node = node.child_by_field_name("name");
    let alias_node = node.child_by_field_name("alias");
    let orig = orig_node
        .map(|n| node_text(n, source).to_owned())
        .unwrap_or_default();
    let local = alias_node
        .map(|n| node_text(n, source).to_owned())
        .unwrap_or_else(|| orig.clone());
    specifiers.push(ImportSpecifier {
        name: local,
        alias: Some(orig),
        is_default: false,
        is_namespace: false,
    });
}

/// Recursively find all import nodes inside a try/except block.
fn extract_conditional_imports(try_node: Node, source: &[u8]) -> Vec<ImportInfo> {
    let mut results = Vec::new();
    let mut cursor = try_node.walk();
    for child in try_node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                // Convert to conditional absolute
                let inner = extract_import_statement(child, source);
                for mut imp in inner {
                    imp.kind = ImportKind::PythonConditionalAbsolute;
                    results.push(imp);
                }
            }
            "import_from_statement" => {
                if let Some(imp) = extract_import_from_statement(child, source, true) {
                    results.push(imp);
                }
            }
            "block" | "except_clause" | "try_statement" => {
                // Recurse into sub-blocks
                results.extend(extract_conditional_imports(child, source));
            }
            _ => {}
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract all Python imports from a parsed syntax tree.
///
/// Handles:
/// - `import os` — `PythonAbsolute`
/// - `from pkg import name` — `PythonAbsolute`
/// - `from . import X` — `PythonRelative { level: 1 }`
/// - `from ..pkg import Y` — `PythonRelative { level: 2 }`
/// - `from module import *` — wildcard, specifiers=[{name:"*"}]
/// - `try: from fast import X` — `PythonConditionalAbsolute`
/// - `try: from . import X` — `PythonConditionalRelative { level }`
/// - Aliased imports: `from pkg import Foo as Bar`
///
/// Dynamic imports (`importlib.import_module(...)`) are ignored (static analysis only).
pub fn extract_python_imports(tree: &Tree, source: &[u8]) -> Vec<ImportInfo> {
    let mut results = Vec::new();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                results.extend(extract_import_statement(child, source));
            }
            "import_from_statement" => {
                if let Some(imp) = extract_import_from_statement(child, source, false) {
                    results.push(imp);
                }
            }
            "try_statement" => {
                results.extend(extract_conditional_imports(child, source));
            }
            _ => {}
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::languages::language_for_extension;

    fn parse_py(source: &str) -> Tree {
        let lang = language_for_extension("py").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    fn extract(source: &str) -> Vec<ImportInfo> {
        let tree = parse_py(source);
        extract_python_imports(&tree, source.as_bytes())
    }

    // Test 1: simple import
    #[test]
    fn test_python_import_simple() {
        let src = "import os\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonAbsolute);
        assert_eq!(imp.module_path, "os");
    }

    // Test 2: dotted import
    #[test]
    fn test_python_import_dotted() {
        let src = "import os.path\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonAbsolute);
        assert_eq!(imp.module_path, "os.path");
    }

    // Test 3: from import
    #[test]
    fn test_python_from_import() {
        let src = "from os import path\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonAbsolute);
        assert_eq!(imp.module_path, "os");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "path");
    }

    // Test 4: from import multiple
    #[test]
    fn test_python_from_import_multiple() {
        let src = "from os import path, getcwd\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.module_path, "os");
        assert_eq!(imp.specifiers.len(), 2, "should have 2 specifiers");
        let names: Vec<_> = imp.specifiers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"path"));
        assert!(names.contains(&"getcwd"));
    }

    // Test 5: relative import dot (from . import utils)
    #[test]
    fn test_python_relative_import_dot() {
        let src = "from . import utils\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonRelative { level: 1 });
        assert_eq!(imp.module_path, "");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "utils");
    }

    // Test 6: relative import dot-dot
    #[test]
    fn test_python_relative_import_dotdot() {
        let src = "from ..pkg import Foo\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonRelative { level: 2 });
        assert_eq!(imp.module_path, "pkg");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "Foo");
    }

    // Test 7: relative import submodule
    #[test]
    fn test_python_relative_import_submodule() {
        let src = "from .sub import helper\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.kind, ImportKind::PythonRelative { level: 1 });
        assert_eq!(imp.module_path, "sub");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "helper");
    }

    // Test 8: wildcard import
    #[test]
    fn test_python_wildcard_import() {
        let src = "from module import *\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.module_path, "module");
        assert_eq!(imp.specifiers.len(), 1);
        assert_eq!(imp.specifiers[0].name, "*");
    }

    // Test 9: conditional imports (try/except)
    #[test]
    fn test_python_conditional_import() {
        let src = "try:\n    from fast import X\nexcept ImportError:\n    from slow import X\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 2, "should extract both branches");
        for imp in &imports {
            assert_eq!(
                imp.kind,
                ImportKind::PythonConditionalAbsolute,
                "conditional imports should have PythonConditionalAbsolute kind"
            );
        }
        let modules: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(modules.contains(&"fast"), "should contain fast import");
        assert!(modules.contains(&"slow"), "should contain slow import");
    }

    // Test 10: aliased import
    #[test]
    fn test_python_aliased_import() {
        let src = "from pkg import Foo as Bar\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.module_path, "pkg");
        assert_eq!(imp.specifiers.len(), 1);
        // name is local name (Bar), alias is original name (Foo)
        assert_eq!(imp.specifiers[0].name, "Bar");
        assert_eq!(imp.specifiers[0].alias.as_deref(), Some("Foo"));
    }

    // Test 11: line numbers
    #[test]
    fn test_python_import_line_numbers() {
        let src = "import os\nfrom sys import argv\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].line, 1, "first import should be on line 1");
        assert_eq!(imports[1].line, 2, "second import should be on line 2");
    }
}

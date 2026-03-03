use tree_sitter::{Node, Tree};

use crate::parser::imports::{ImportInfo, ImportKind, ImportSpecifier};

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Strip surrounding double-quotes from a Go import path string.
/// e.g. `"fmt"` → `fmt`, `"github.com/pkg/errors"` → `github.com/pkg/errors`
fn strip_quotes(s: &str) -> String {
    s.trim_matches('"').to_owned()
}

/// Return the last path segment of an import path.
/// e.g. `"github.com/pkg/errors"` → `"errors"`, `"fmt"` → `"fmt"`.
fn last_path_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Core extraction
// ---------------------------------------------------------------------------

/// Process a single `import_spec` node into an `ImportInfo`.
fn process_import_spec(spec: Node, source: &[u8]) -> Option<ImportInfo> {
    // path field is required (interpreted_string_literal or raw_string_literal)
    let path_node = spec.child_by_field_name("path")?;
    let raw_path = node_text(path_node, source);

    // For interpreted_string_literal, the content is the first child (string content)
    // For raw_string_literal, the content is also wrapped.
    // The node text includes the quotes — strip them.
    let module_path = match path_node.kind() {
        "interpreted_string_literal" => {
            // The content is between the quotes — find interpreted_string_literal_content child
            let mut cursor = path_node.walk();
            let mut content = String::new();
            for child in path_node.children(&mut cursor) {
                if child.kind() == "interpreted_string_literal_content"
                    || child.kind() == "escape_sequence"
                {
                    content.push_str(node_text(child, source));
                }
            }
            if content.is_empty() {
                // Fallback: strip quotes from raw text
                strip_quotes(raw_path)
            } else {
                content
            }
        }
        "raw_string_literal" => {
            // Raw string content is between backticks
            let mut cursor = path_node.walk();
            let mut content = String::new();
            for child in path_node.children(&mut cursor) {
                if child.kind() == "raw_string_literal_content" {
                    content.push_str(node_text(child, source));
                }
            }
            if content.is_empty() {
                strip_quotes(raw_path)
            } else {
                content
            }
        }
        _ => strip_quotes(raw_path),
    };

    let line = spec.start_position().row + 1;

    // name field: optional, determines kind
    let name_node = spec.child_by_field_name("name");
    let (kind, specifiers) = match name_node {
        None => {
            // Regular import: `import "fmt"` — no name field
            (ImportKind::GoAbsolute, Vec::new())
        }
        Some(name_n) => {
            match name_n.kind() {
                "blank_identifier" => {
                    // `import _ "pkg"` — blank/side-effect import
                    (ImportKind::GoBlank, Vec::new())
                }
                "dot" => {
                    // `import . "pkg"` — dot import
                    (ImportKind::GoDot, Vec::new())
                }
                "package_identifier" => {
                    // `import alias "pkg"` — aliased import
                    let alias = node_text(name_n, source).to_owned();
                    let original = last_path_segment(&module_path).to_owned();
                    let specifier = ImportSpecifier {
                        name: alias.clone(),
                        alias: Some(original),
                        is_default: false,
                        is_namespace: false,
                    };
                    (ImportKind::GoAbsolute, vec![specifier])
                }
                _ => (ImportKind::GoAbsolute, Vec::new()),
            }
        }
    };

    Some(ImportInfo {
        kind,
        module_path,
        specifiers,
        line,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract all Go import declarations from a parsed syntax tree.
///
/// Handles:
/// - Single inline import: `import "fmt"` → 1 ImportInfo
/// - Block import: `import ( "fmt"; "os" )` → multiple ImportInfo
/// - Blank import: `import _ "net/http/pprof"` → GoBlank
/// - Dot import: `import . "math"` → GoDot
/// - Aliased import: `import f "fmt"` → GoAbsolute with specifier alias
pub fn extract_go_imports(tree: &Tree, source: &[u8]) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "import_declaration" {
            continue;
        }

        // import_declaration can contain either:
        // - A single import_spec (inline: `import "fmt"`)
        // - An import_spec_list (block: `import ( "fmt"; "os" )`)
        let mut child_cursor = child.walk();
        for inner in child.children(&mut child_cursor) {
            match inner.kind() {
                "import_spec" => {
                    if let Some(info) = process_import_spec(inner, source) {
                        imports.push(info);
                    }
                }
                "import_spec_list" => {
                    let mut list_cursor = inner.walk();
                    for spec in inner.children(&mut list_cursor) {
                        if spec.kind() == "import_spec"
                            && let Some(info) = process_import_spec(spec, source)
                        {
                            imports.push(info);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    imports
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::languages::language_for_extension;

    fn parse_go(source: &str) -> Tree {
        let lang = language_for_extension("go").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    fn extract(source: &str) -> Vec<ImportInfo> {
        let tree = parse_go(source);
        extract_go_imports(&tree, source.as_bytes())
    }

    // Test 1: simple single import
    #[test]
    fn test_go_import_simple() {
        let src = "package main\n\nimport \"fmt\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_path, "fmt");
        assert_eq!(imports[0].kind, ImportKind::GoAbsolute);
    }

    // Test 2: block import with multiple packages
    #[test]
    fn test_go_import_block() {
        let src = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
        let imports = extract(src);
        assert_eq!(
            imports.len(),
            2,
            "Expected 2 imports, got {}",
            imports.len()
        );
        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"os"));
        for imp in &imports {
            assert_eq!(imp.kind, ImportKind::GoAbsolute);
        }
    }

    // Test 3: blank import (side-effect)
    #[test]
    fn test_go_import_blank() {
        let src = "package main\n\nimport _ \"net/http/pprof\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::GoBlank);
        assert_eq!(imports[0].module_path, "net/http/pprof");
    }

    // Test 4: dot import
    #[test]
    fn test_go_import_dot() {
        let src = "package main\n\nimport . \"math\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::GoDot);
        assert_eq!(imports[0].module_path, "math");
    }

    // Test 5: aliased import
    #[test]
    fn test_go_import_aliased() {
        let src = "package main\n\nimport f \"fmt\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::GoAbsolute);
        assert_eq!(imports[0].module_path, "fmt");
        assert_eq!(imports[0].specifiers.len(), 1);
        assert_eq!(imports[0].specifiers[0].name, "f");
        assert_eq!(imports[0].specifiers[0].alias.as_deref(), Some("fmt"));
    }

    // Test 6: import path with domain
    #[test]
    fn test_go_import_path_with_domain() {
        let src = "package main\n\nimport \"github.com/pkg/errors\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::GoAbsolute);
        assert_eq!(imports[0].module_path, "github.com/pkg/errors");
    }

    // Test 7: line numbers are correct
    #[test]
    fn test_go_import_line_numbers() {
        let src = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
        let imports = extract(src);
        // "fmt" is on line 4, "os" is on line 5 (1-based)
        assert_eq!(imports.len(), 2);
        let fmt_imp = imports.iter().find(|i| i.module_path == "fmt").unwrap();
        let os_imp = imports.iter().find(|i| i.module_path == "os").unwrap();
        assert!(fmt_imp.line > 0, "line should be > 0");
        assert!(os_imp.line > fmt_imp.line, "os line should be > fmt line");
    }

    // Test 8: multiple import blocks
    #[test]
    fn test_go_import_multi_block() {
        let src = "package main\n\nimport \"fmt\"\n\nimport \"os\"\n";
        let imports = extract(src);
        assert_eq!(imports.len(), 2);
        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"os"));
    }
}

use std::collections::HashSet;

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::graph::node::{DecoratorInfo, SymbolInfo, SymbolKind, SymbolVisibility};

// ---------------------------------------------------------------------------
// Query string
// ---------------------------------------------------------------------------

/// Tree-sitter query for Python top-level symbol definitions.
///
/// Captures:
/// - Module-level function definitions (sync and async)
/// - Module-level class definitions
/// - decorated_definition (which wraps decorated functions and classes)
/// - Module-level variable assignments
///
/// Note: decorated_definition captures both the outer wrapper AND the inner
/// function_definition / class_definition. We deduplicate in code by skipping
/// bare function_definition / class_definition nodes whose parent is a
/// decorated_definition.
///
/// PEP 695 type alias statements are extracted via a separate tree walk because
/// tree-sitter-python 0.25 uses `left: (type ...)` (not a `name` field) for the
/// alias name, and the @name capture pattern cannot be directly applied.
const PYTHON_SYMBOL_QUERY: &str = r#"
(function_definition name: (identifier) @name) @symbol
(class_definition name: (identifier) @name) @symbol
(decorated_definition
  (function_definition name: (identifier) @name)) @symbol
(decorated_definition
  (class_definition name: (identifier) @name)) @symbol
(module
  (expression_statement
    (assignment left: (identifier) @name))) @symbol
"#;

// ---------------------------------------------------------------------------
// OnceLock query cache
// ---------------------------------------------------------------------------

static PY_SYMBOL_QUERY: std::sync::OnceLock<Query> = std::sync::OnceLock::new();

fn py_symbol_query(language: &Language) -> &'static Query {
    PY_SYMBOL_QUERY.get_or_init(|| {
        Query::new(language, PYTHON_SYMBOL_QUERY).expect("invalid Python symbol query")
    })
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Determine Python symbol visibility based on naming convention.
///
/// Any name starting with `_` (single underscore, double underscore, or
/// name-mangling prefix) is Private. All other names are Pub.
fn python_visibility(name: &str) -> SymbolVisibility {
    if name.starts_with('_') {
        SymbolVisibility::Private
    } else {
        SymbolVisibility::Pub
    }
}

// ---------------------------------------------------------------------------
// Decorator extraction (Python-specific)
// ---------------------------------------------------------------------------

/// Parse a single Python `decorator` node into a `DecoratorInfo`.
///
/// Python decorator grammar variants:
/// - Simple: `@name` → `(decorator (identifier))`
/// - Attribute: `@obj.attr` → `(decorator (attribute object: (identifier) attribute: (identifier)))`
/// - Call: `@name(args)` or `@obj.attr(args)` → `(decorator (call function: ... arguments: ...))`
fn parse_python_decorator(decorator_node: Node, source: &[u8]) -> DecoratorInfo {
    // The decorator node in tree-sitter-python 0.25 has its content as the
    // first (and only relevant) named child, after the '@' token.
    let inner = decorator_node.named_child(0);

    match inner.map(|n| n.kind()) {
        Some("identifier") => {
            let name = node_text(inner.unwrap(), source).to_owned();
            DecoratorInfo {
                name,
                object: None,
                attribute: None,
                args_raw: None,
                framework: None,
            }
        }
        Some("attribute") => {
            let attr_node = inner.unwrap();
            let obj = attr_node
                .child_by_field_name("object")
                .map(|n| node_text(n, source).to_owned());
            let attr = attr_node
                .child_by_field_name("attribute")
                .map(|n| node_text(n, source).to_owned());
            let name = format!(
                "{}.{}",
                obj.as_deref().unwrap_or(""),
                attr.as_deref().unwrap_or("")
            );
            DecoratorInfo {
                name,
                object: obj,
                attribute: attr,
                args_raw: None,
                framework: None,
            }
        }
        Some("call") => {
            let call = inner.unwrap();
            let func = call.child_by_field_name("function");
            let args = call
                .child_by_field_name("arguments")
                .map(|n| node_text(n, source).to_owned());

            let (name, obj, attr) = match func.map(|f| f.kind()) {
                Some("identifier") => {
                    let n = node_text(func.unwrap(), source).to_owned();
                    (n, None, None)
                }
                Some("attribute") => {
                    let f = func.unwrap();
                    let o = f
                        .child_by_field_name("object")
                        .map(|n| node_text(n, source).to_owned());
                    let a = f
                        .child_by_field_name("attribute")
                        .map(|n| node_text(n, source).to_owned());
                    let n = format!(
                        "{}.{}",
                        o.as_deref().unwrap_or(""),
                        a.as_deref().unwrap_or("")
                    );
                    (n, o, a)
                }
                _ => (node_text(call, source).to_owned(), None, None),
            };
            DecoratorInfo {
                name,
                object: obj,
                attribute: attr,
                args_raw: args,
                framework: None,
            }
        }
        _ => {
            // Fallback: use full decorator text
            DecoratorInfo {
                name: node_text(decorator_node, source).to_owned(),
                object: None,
                attribute: None,
                args_raw: None,
                framework: None,
            }
        }
    }
}

/// Extract all decorators from a `decorated_definition` node.
///
/// In tree-sitter-python, a `decorated_definition` has `decorator` children
/// listed before the actual definition. They appear in source order.
fn extract_python_decorators(decorated_node: Node, source: &[u8]) -> Vec<DecoratorInfo> {
    let mut decorators = Vec::new();
    let mut cursor = decorated_node.walk();
    for child in decorated_node.children(&mut cursor) {
        if child.kind() == "decorator" {
            decorators.push(parse_python_decorator(child, source));
        }
    }
    decorators
}

// ---------------------------------------------------------------------------
// __all__ extraction (Pass 1)
// ---------------------------------------------------------------------------

/// Walk the module root to find `__all__ = [...]` or `__all__ = (...)` and
/// collect all string literal values into a HashSet.
///
/// Returns `None` if no `__all__` assignment is found at module level.
fn extract_all_exports(root: Node, source: &[u8]) -> Option<HashSet<String>> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "expression_statement" {
            continue;
        }
        // expression_statement should have exactly one child: assignment
        let mut expr_cursor = child.walk();
        for expr_child in child.children(&mut expr_cursor) {
            if expr_child.kind() == "assignment" {
                let left = expr_child.child_by_field_name("left")?;
                if left.kind() == "identifier" && node_text(left, source) == "__all__" {
                    let right = expr_child.child_by_field_name("right")?;
                    return Some(collect_string_list(right, source));
                }
            }
        }
    }
    None
}

/// Collect string literal values from a list/tuple node.
fn collect_string_list(list_node: Node, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = list_node.walk();
    for child in list_node.children(&mut cursor) {
        if child.kind() == "string" {
            // String content is in a string_content or string_fragment child
            let text = node_text(child, source);
            // Strip surrounding quotes: 'Foo' → Foo, "Foo" → Foo
            let stripped = text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'');
            names.insert(stripped.to_owned());
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Nested class member extraction
// ---------------------------------------------------------------------------

/// Extract methods and nested classes from a class body block.
///
/// Walks `class_definition → block → function_definition | class_definition | decorated_definition`.
/// Methods get `SymbolKind::Method`, nested classes get `SymbolKind::Class`.
fn extract_python_class_members(class_node: Node, source: &[u8]) -> Vec<SymbolInfo> {
    let mut children = Vec::new();

    // Find the block (class body)
    let block = {
        let mut found = None;
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "block" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(b) => b,
            None => return children,
        }
    };

    let mut cursor = block.walk();
    for child in block.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source).to_owned();
                    let pos = name_node.start_position();
                    children.push(SymbolInfo {
                        name: name.clone(),
                        kind: SymbolKind::Method,
                        line: pos.row + 1,
                        col: pos.column,
                        line_end: child.end_position().row + 1,
                        visibility: python_visibility(&name),
                        ..Default::default()
                    });
                }
            }
            "decorated_definition" => {
                // Find the inner function or class definition
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    match inner_child.kind() {
                        "function_definition" => {
                            if let Some(name_node) = inner_child.child_by_field_name("name") {
                                let name = node_text(name_node, source).to_owned();
                                let pos = name_node.start_position();
                                let decorators = extract_python_decorators(child, source);
                                children.push(SymbolInfo {
                                    name: name.clone(),
                                    kind: SymbolKind::Method,
                                    line: pos.row + 1,
                                    col: pos.column,
                                    line_end: child.end_position().row + 1,
                                    visibility: python_visibility(&name),
                                    decorators,
                                    ..Default::default()
                                });
                            }
                        }
                        "class_definition" => {
                            if let Some(name_node) = inner_child.child_by_field_name("name") {
                                let name = node_text(name_node, source).to_owned();
                                let pos = name_node.start_position();
                                let decorators = extract_python_decorators(child, source);
                                children.push(SymbolInfo {
                                    name: name.clone(),
                                    kind: SymbolKind::Class,
                                    line: pos.row + 1,
                                    col: pos.column,
                                    line_end: child.end_position().row + 1,
                                    visibility: python_visibility(&name),
                                    decorators,
                                    ..Default::default()
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "class_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source).to_owned();
                    let pos = name_node.start_position();
                    children.push(SymbolInfo {
                        name: name.clone(),
                        kind: SymbolKind::Class,
                        line: pos.row + 1,
                        col: pos.column,
                        line_end: child.end_position().row + 1,
                        visibility: python_visibility(&name),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }

    children
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract all top-level symbols from a Python source file.
///
/// Returns a vector of `(parent_symbol, child_symbols)` pairs, where
/// child_symbols are the methods/nested classes of a class definition.
///
/// **Two-pass algorithm:**
/// 1. Extract `__all__` list (if present) to determine `is_exported`.
/// 2. Run the tree-sitter query to extract all symbol definitions.
pub fn extract_python_symbols(
    tree: &Tree,
    source: &[u8],
    language: &Language,
) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let root = tree.root_node();

    // Pass 1: collect __all__ exports
    let all_exports_opt = extract_all_exports(root, source);

    // Pass 2: run query to collect symbols
    let query = py_symbol_query(language);
    let name_idx = query
        .capture_index_for_name("name")
        .expect("python symbol query must have @name");
    let symbol_idx = query
        .capture_index_for_name("symbol")
        .expect("python symbol query must have @symbol");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);

    let mut results: Vec<(SymbolInfo, Vec<SymbolInfo>)> = Vec::new();

    while let Some(m) = matches.next() {
        let mut name_node: Option<Node> = None;
        let mut symbol_node: Option<Node> = None;

        for capture in m.captures {
            if capture.index == name_idx {
                name_node = Some(capture.node);
            } else if capture.index == symbol_idx {
                symbol_node = Some(capture.node);
            }
        }

        let (name_n, sym_n) = match (name_node, symbol_node) {
            (Some(n), Some(s)) => (n, s),
            _ => continue,
        };

        let name = node_text(name_n, source).to_owned();
        let sym_kind = sym_n.kind();

        // Dedup: skip bare function_definition / class_definition if parent is decorated_definition
        // (the decorated_definition match will handle it with decorator info)
        if (sym_kind == "function_definition" || sym_kind == "class_definition")
            && let Some(parent) = sym_n.parent()
            && parent.kind() == "decorated_definition"
        {
            continue;
        }

        // Determine symbol kind
        let kind = match sym_kind {
            "function_definition" => SymbolKind::Function,
            "class_definition" => SymbolKind::Class,
            "decorated_definition" => {
                // Look at the inner definition
                let mut inner_kind = SymbolKind::Function;
                let mut inner_cursor = sym_n.walk();
                for child in sym_n.children(&mut inner_cursor) {
                    match child.kind() {
                        "function_definition" => {
                            inner_kind = SymbolKind::Function;
                            break;
                        }
                        "class_definition" => {
                            inner_kind = SymbolKind::Class;
                            break;
                        }
                        _ => {}
                    }
                }
                inner_kind
            }
            "module" => {
                // This is the (module (expression_statement (assignment ...))) @symbol pattern
                SymbolKind::Variable
            }
            "expression_statement" => {
                // Also a variable assignment
                SymbolKind::Variable
            }
            _ => continue,
        };

        // The actual definition node for positional info
        // For module-level assignments, the @symbol is "module" (the whole file node)
        // We need to find the actual expression_statement for positional info
        let def_node = if sym_kind == "module" {
            // Walk to find the specific expression_statement containing this assignment
            find_assignment_node(root, name_n)
        } else {
            Some(sym_n)
        };

        let def_node = match def_node {
            Some(n) => n,
            None => continue,
        };

        // Extract decorators for decorated_definition nodes
        let decorators = if sym_kind == "decorated_definition" {
            extract_python_decorators(sym_n, source)
        } else {
            Vec::new()
        };

        // Visibility from naming convention
        let visibility = python_visibility(&name);

        // is_exported logic
        let is_exported = match &all_exports_opt {
            Some(all_exports) => all_exports.contains(&name),
            None => !name.starts_with('_'),
        };

        // Position from the name identifier node
        let pos = name_n.start_position();
        let line = pos.row + 1;
        let col = pos.column;
        let line_end = def_node.end_position().row + 1;

        let symbol = SymbolInfo {
            name: name.clone(),
            kind: kind.clone(),
            line,
            col,
            line_end,
            is_exported,
            is_default: false,
            visibility,
            trait_impl: None,
            decorators,
        };

        // Extract children for class definitions
        let children = if kind == SymbolKind::Class {
            // Find the actual class_definition node
            let class_node = if sym_kind == "decorated_definition" {
                let mut found = None;
                let mut c = sym_n.walk();
                for child in sym_n.children(&mut c) {
                    if child.kind() == "class_definition" {
                        found = Some(child);
                        break;
                    }
                }
                found.unwrap_or(sym_n)
            } else {
                sym_n
            };
            extract_python_class_members(class_node, source)
        } else {
            Vec::new()
        };

        results.push((symbol, children));
    }

    // Also extract PEP 695 type alias statements (handled separately due to grammar structure)
    results.extend(extract_type_aliases(root, source, &all_exports_opt));

    results
}

/// Find the `expression_statement` node containing the assignment for a given identifier.
///
/// Used to get correct positional info for module-level assignments where the
/// @symbol capture is the entire `module` node.
fn find_assignment_node<'a>(root: Node<'a>, name_node: Node<'a>) -> Option<Node<'a>> {
    // The name_node is the identifier. Its parent should be the assignment,
    // and the assignment's parent should be expression_statement.
    let assignment = name_node.parent()?;
    if assignment.kind() == "assignment" {
        let expr_stmt = assignment.parent()?;
        if expr_stmt.kind() == "expression_statement"
            && let Some(p) = expr_stmt.parent()
            && p.id() == root.id()
        {
            return Some(expr_stmt);
        }
    }
    None
}

/// Extract PEP 695 type alias statements from the module root.
///
/// tree-sitter-python 0.25 represents `type Alias = int` as:
/// `(type_alias_statement left: (type (identifier)) right: (type ...))`
///
/// We walk the module root directly to find these nodes since the identifier
/// is nested within a `type` wrapper that can't be cleanly captured by a
/// tree-sitter query pattern with @name.
fn extract_type_aliases(
    root: Node,
    source: &[u8],
    all_exports_opt: &Option<HashSet<String>>,
) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let mut results = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "type_alias_statement"
            && let Some(name) = extract_type_alias_name(child, source)
        {
            let pos_node = child.child_by_field_name("left").unwrap_or(child);
            let pos = pos_node.start_position();
            let visibility = python_visibility(&name);
            let is_exported = match all_exports_opt {
                Some(all_exports) => all_exports.contains(&name),
                None => !name.starts_with('_'),
            };
            results.push((
                SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::TypeAlias,
                    line: pos.row + 1,
                    col: pos.column,
                    line_end: child.end_position().row + 1,
                    is_exported,
                    is_default: false,
                    visibility,
                    trait_impl: None,
                    decorators: Vec::new(),
                },
                Vec::new(),
            ));
        }
    }
    results
}

/// Extract the name from a `type_alias_statement` node.
///
/// The name is stored in `left: (type <content>)`. The `type` wrapper node's
/// first named child should be the identifier (or an identifier embedded in
/// a primary_expression).
fn extract_type_alias_name<'a>(node: Node<'a>, source: &'a [u8]) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    // The `type` node wraps the actual identifier; try to find first named child
    let inner = left.named_child(0)?;
    // The inner node could be an identifier directly, or a primary_expression
    // containing an identifier
    let text = node_text(inner, source);
    if !text.is_empty() {
        Some(text.to_owned())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::languages::language_for_extension;

    fn parse_py(source: &str) -> (Tree, Language) {
        let lang = language_for_extension("py").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    fn extract(source: &str) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
        let (tree, lang) = parse_py(source);
        extract_python_symbols(&tree, source.as_bytes(), &lang)
    }

    // Test 1: basic sync function
    #[test]
    fn test_python_function() {
        let src = "def hello():\n    pass\n";
        let syms = extract(src);
        assert_eq!(
            syms.len(),
            1,
            "expected 1 symbol, got {}: {:?}",
            syms.len(),
            syms.iter().map(|(s, _)| &s.name).collect::<Vec<_>>()
        );
        let (sym, children) = &syms[0];
        assert_eq!(sym.name, "hello");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
        assert!(children.is_empty());
    }

    // Test 2: async function
    #[test]
    fn test_python_async_function() {
        let src = "async def fetch():\n    pass\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "fetch");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 3: class definition
    #[test]
    fn test_python_class() {
        let src = "class MyClass:\n    pass\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "MyClass");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 4: module-level assignment → Variable
    #[test]
    fn test_python_assignment() {
        let src = "MAX_SIZE = 100\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1, "expected 1 symbol");
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "MAX_SIZE");
        assert_eq!(sym.kind, SymbolKind::Variable);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 5: PEP 695 type alias
    #[test]
    fn test_python_type_alias() {
        let src = "type Alias = int\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1, "expected 1 symbol");
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "Alias");
        assert_eq!(sym.kind, SymbolKind::TypeAlias);
    }

    // Test 6: single-underscore private
    #[test]
    fn test_python_visibility_private() {
        let src = "_helper = 1\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "_helper");
        assert_eq!(sym.visibility, SymbolVisibility::Private);
        assert!(!sym.is_exported);
    }

    // Test 7: double-underscore private (name-mangled)
    #[test]
    fn test_python_visibility_dunder() {
        let src = "__secret = 1\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "__secret");
        assert_eq!(sym.visibility, SymbolVisibility::Private);
        assert!(!sym.is_exported);
    }

    // Test 8: dunder method __init__ in class → Method, Private
    #[test]
    fn test_python_dunder_method() {
        let src = "class MyClass:\n    def __init__(self):\n        pass\n";
        let syms = extract(src);
        let (class_sym, children) = &syms[0];
        assert_eq!(class_sym.name, "MyClass");
        assert_eq!(children.len(), 1);
        let method = &children[0];
        assert_eq!(method.name, "__init__");
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.visibility, SymbolVisibility::Private);
    }

    // Test 9: __all__ controls is_exported
    #[test]
    fn test_python_all_exports() {
        let src = "__all__ = [\"Foo\"]\n\nclass Foo:\n    pass\n\nclass Bar:\n    pass\n";
        let syms = extract(src);
        // Should have __all__ (Variable), Foo (Class), Bar (Class)
        let foo = syms.iter().find(|(s, _)| s.name == "Foo").unwrap();
        let bar = syms.iter().find(|(s, _)| s.name == "Bar").unwrap();
        let all = syms.iter().find(|(s, _)| s.name == "__all__");
        assert!(foo.0.is_exported, "Foo should be exported");
        assert!(!bar.0.is_exported, "Bar should NOT be exported");
        // __all__ itself is a Variable that is private by convention
        if let Some((all_sym, _)) = all {
            assert_eq!(all_sym.kind, SymbolKind::Variable);
        }
    }

    // Test 10: __all__ exports a _prefixed symbol → is_exported=true, visibility=Private
    #[test]
    fn test_python_all_exports_private() {
        let src = "__all__ = [\"_helper\"]\n\n_helper = 1\n";
        let syms = extract(src);
        let helper = syms.iter().find(|(s, _)| s.name == "_helper").unwrap();
        assert!(
            helper.0.is_exported,
            "is_exported should be true (in __all__)"
        );
        assert_eq!(
            helper.0.visibility,
            SymbolVisibility::Private,
            "visibility should be Private"
        );
    }

    // Test 11: decorated function
    #[test]
    fn test_python_decorated_function() {
        let src = "@decorator\ndef foo():\n    pass\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1, "expected 1 symbol (no duplicate)");
        let (sym, _) = &syms[0];
        assert_eq!(sym.name, "foo");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.decorators.len(), 1);
        assert_eq!(sym.decorators[0].name, "decorator");
        assert!(sym.decorators[0].object.is_none());
        assert!(sym.decorators[0].attribute.is_none());
    }

    // Test 12: stacked decorators in source order
    #[test]
    fn test_python_stacked_decorators() {
        let src = "@first\n@second\ndef foo():\n    pass\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.decorators.len(), 2);
        assert_eq!(sym.decorators[0].name, "first");
        assert_eq!(sym.decorators[1].name, "second");
    }

    // Test 13: attribute decorator @app.route("/api")
    #[test]
    fn test_python_attribute_decorator() {
        let src = "@app.route(\"/api\")\ndef handler():\n    pass\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert_eq!(sym.decorators.len(), 1);
        let dec = &sym.decorators[0];
        assert_eq!(dec.object.as_deref(), Some("app"));
        assert_eq!(dec.attribute.as_deref(), Some("route"));
        assert!(
            dec.args_raw.is_some(),
            "args_raw should be present for call decorator"
        );
    }

    // Test 14: multi-line function → line_end > line
    #[test]
    fn test_python_line_end() {
        let src = "def long_func():\n    x = 1\n    y = 2\n    return x + y\n";
        let syms = extract(src);
        assert_eq!(syms.len(), 1);
        let (sym, _) = &syms[0];
        assert!(
            sym.line_end > sym.line,
            "line_end ({}) should be > line ({})",
            sym.line_end,
            sym.line
        );
    }

    // Test 15: decorated function appears only once (no duplicate)
    #[test]
    fn test_python_no_duplicate_decorated() {
        let src = "@my_decorator\ndef process():\n    pass\n";
        let syms = extract(src);
        let count = syms.iter().filter(|(s, _)| s.name == "process").count();
        assert_eq!(
            count, 1,
            "decorated function should appear exactly once, got {}",
            count
        );
    }

    // Test 16: class with methods → parent has children
    #[test]
    fn test_python_nested_class_methods() {
        let src = "class Animal:\n    def speak(self):\n        pass\n    def move(self):\n        pass\n";
        let syms = extract(src);
        let (class_sym, children) = &syms[0];
        assert_eq!(class_sym.name, "Animal");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(children.len(), 2, "expected 2 methods");
        let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"speak"));
        assert!(names.contains(&"move"));
        for child in children {
            assert_eq!(child.kind, SymbolKind::Method);
        }
    }

    // Test 17: assignments inside functions NOT extracted (module-level anchoring)
    #[test]
    fn test_python_module_level_only_assignments() {
        let src = "def my_func():\n    local_var = 42\n    return local_var\n\ntop_level = 1\n";
        let syms = extract(src);
        // Should have: my_func (Function), top_level (Variable)
        // Should NOT have: local_var
        let names: Vec<_> = syms.iter().map(|(s, _)| s.name.as_str()).collect();
        assert!(names.contains(&"my_func"), "should have my_func");
        assert!(names.contains(&"top_level"), "should have top_level");
        assert!(
            !names.contains(&"local_var"),
            "should NOT have local_var (function-body assignment)"
        );
    }
}

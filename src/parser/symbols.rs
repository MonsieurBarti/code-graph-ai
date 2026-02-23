use std::sync::OnceLock;

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::graph::node::{SymbolInfo, SymbolKind};

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

/// Tree-sitter S-expression query for TypeScript (`.ts`) files.
const SYMBOL_QUERY_TS: &str = r#"
    ; Top-level function declarations
    (function_declaration
      name: (identifier) @name) @symbol

    ; Class declarations
    (class_declaration
      name: (type_identifier) @name) @symbol

    ; Interface declarations (TS-only)
    (interface_declaration
      name: (type_identifier) @name) @symbol

    ; Type alias declarations (TS-only)
    (type_alias_declaration
      name: (type_identifier) @name) @symbol

    ; Enum declarations
    (enum_declaration
      name: (identifier) @name) @symbol

    ; Exported arrow-function constants: export const Foo = () => {}
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Top-level non-exported arrow-function constants: const Foo = () => {}
    (program
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Exported variables that are NOT arrow functions: export const Foo = value
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (_) @val))) @symbol
"#;

/// Tree-sitter S-expression query for TSX (`.tsx`) and JSX (`.jsx`) files.
const SYMBOL_QUERY_TSX: &str = r#"
    ; Top-level function declarations
    (function_declaration
      name: (identifier) @name) @symbol

    ; Class declarations
    (class_declaration
      name: (type_identifier) @name) @symbol

    ; Interface declarations (TS-only but TSX grammar supports it)
    (interface_declaration
      name: (type_identifier) @name) @symbol

    ; Type alias declarations (TS-only but TSX grammar supports it)
    (type_alias_declaration
      name: (type_identifier) @name) @symbol

    ; Enum declarations
    (enum_declaration
      name: (identifier) @name) @symbol

    ; Exported arrow-function constants
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Top-level non-exported arrow-function constants
    (program
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Exported variables that are NOT arrow functions
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (_) @val))) @symbol
"#;

/// Tree-sitter S-expression query for JavaScript (`.js`/`.jsx`) files.
/// JavaScript does not have interface/type-alias/enum declarations.
const SYMBOL_QUERY_JS: &str = r#"
    ; Top-level function declarations
    (function_declaration
      name: (identifier) @name) @symbol

    ; Class declarations
    (class_declaration
      name: (identifier) @name) @symbol

    ; Exported arrow-function constants
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Top-level non-exported arrow-function constants
    (program
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (arrow_function)))) @symbol

    ; Exported variables that are NOT arrow functions
    (export_statement
      (lexical_declaration
        (variable_declarator
          name: (identifier) @name
          value: (_) @val))) @symbol
"#;

// ---------------------------------------------------------------------------
// Query cache (compiled once per language via OnceLock)
// ---------------------------------------------------------------------------

static TS_QUERY: OnceLock<Query> = OnceLock::new();
static TSX_QUERY: OnceLock<Query> = OnceLock::new();
static JS_QUERY: OnceLock<Query> = OnceLock::new();

fn ts_query(language: &Language) -> &'static Query {
    TS_QUERY.get_or_init(|| Query::new(language, SYMBOL_QUERY_TS).expect("invalid TS symbol query"))
}

fn tsx_query(language: &Language) -> &'static Query {
    TSX_QUERY
        .get_or_init(|| Query::new(language, SYMBOL_QUERY_TSX).expect("invalid TSX symbol query"))
}

fn js_query(language: &Language) -> &'static Query {
    JS_QUERY.get_or_init(|| Query::new(language, SYMBOL_QUERY_JS).expect("invalid JS symbol query"))
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

/// Extract the UTF-8 text of a node from the original source bytes.
fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Check whether `node` is — or is nested inside — an `export_statement`.
/// Returns `(is_exported, is_default)`.
fn detect_export(node: Node, source: &[u8]) -> (bool, bool) {
    // Start from `node` itself (the @symbol capture may BE the export_statement)
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "export_statement" {
            // Check for `default` keyword among direct children
            let is_default = (0..n.child_count()).any(|i| {
                n.child(i as u32)
                    .map(|c| node_text(c, source) == "default")
                    .unwrap_or(false)
            });
            return (true, is_default);
        }
        current = n.parent();
    }
    (false, false)
}

/// Return true when the tree rooted at `node` contains a `jsx_element`,
/// `jsx_fragment`, or `jsx_self_closing_element` anywhere in its descendants.
fn contains_jsx(node: Node) -> bool {
    if matches!(
        node.kind(),
        "jsx_element" | "jsx_fragment" | "jsx_self_closing_element"
    ) {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_jsx(child) {
            return true;
        }
    }
    false
}

/// Return true if `node` is an `arrow_function` or a `function` expression.
fn is_arrow_or_function_value(node: Node) -> bool {
    matches!(node.kind(), "arrow_function" | "function")
}

// ---------------------------------------------------------------------------
// Symbol classification
// ---------------------------------------------------------------------------

/// Classify the kind of a top-level match node.
///
/// `symbol_node` is the `@symbol` capture (the outer statement node).
/// `name_node` is the `@name` capture (the identifier node).
/// `val_node` is the optional `@val` capture (value in a `variable_declarator`).
/// `is_tsx` enables JSX component detection.
fn classify_symbol(
    symbol_node: Node,
    name_node: Node,
    val_node: Option<Node>,
    is_tsx: bool,
    _source: &[u8],
) -> Option<SymbolKind> {
    let kind = find_declaration_kind(symbol_node, name_node);

    match kind.as_deref() {
        Some("function_declaration") => {
            if is_tsx && function_body_contains_jsx(symbol_node) {
                Some(SymbolKind::Component)
            } else {
                Some(SymbolKind::Function)
            }
        }
        Some("class_declaration") => Some(SymbolKind::Class),
        Some("interface_declaration") => Some(SymbolKind::Interface),
        Some("type_alias_declaration") => Some(SymbolKind::TypeAlias),
        Some("enum_declaration") => Some(SymbolKind::Enum),
        Some("arrow_function_decl") => {
            if is_tsx && arrow_body_contains_jsx(symbol_node, name_node) {
                Some(SymbolKind::Component)
            } else {
                Some(SymbolKind::Function)
            }
        }
        Some("exported_variable") => {
            if let Some(val) = val_node {
                if is_arrow_or_function_value(val) {
                    // arrow function — should have been caught earlier but handle defensively
                    if is_tsx && arrow_body_contains_jsx(symbol_node, name_node) {
                        Some(SymbolKind::Component)
                    } else {
                        Some(SymbolKind::Function)
                    }
                } else {
                    Some(SymbolKind::Variable)
                }
            } else {
                Some(SymbolKind::Variable)
            }
        }
        _ => None,
    }
}

/// Classify `symbol_node` by inspecting its kind and children.
/// Returns a synthetic kind string.
fn find_declaration_kind(symbol_node: Node, _name_node: Node) -> Option<String> {
    let kind = symbol_node.kind();
    match kind {
        "function_declaration" => Some("function_declaration".into()),
        "class_declaration" => Some("class_declaration".into()),
        "interface_declaration" => Some("interface_declaration".into()),
        "type_alias_declaration" => Some("type_alias_declaration".into()),
        "enum_declaration" => Some("enum_declaration".into()),
        "export_statement" => {
            let mut cursor = symbol_node.walk();
            for child in symbol_node.children(&mut cursor) {
                match child.kind() {
                    "function_declaration" => return Some("function_declaration".into()),
                    "class_declaration" => return Some("class_declaration".into()),
                    "interface_declaration" => return Some("interface_declaration".into()),
                    "type_alias_declaration" => return Some("type_alias_declaration".into()),
                    "enum_declaration" => return Some("enum_declaration".into()),
                    "lexical_declaration" => {
                        return classify_lexical_declaration(child);
                    }
                    _ => {}
                }
            }
            None
        }
        "lexical_declaration" => classify_lexical_declaration(symbol_node),
        _ => None,
    }
}

/// Inspect a `lexical_declaration` to determine if its declarator value is an
/// arrow function or plain expression.
fn classify_lexical_declaration(lex_decl: Node) -> Option<String> {
    let mut cursor = lex_decl.walk();
    for child in lex_decl.children(&mut cursor) {
        if child.kind() == "variable_declarator"
            && let Some(value_node) = child.child_by_field_name("value")
        {
            if is_arrow_or_function_value(value_node) {
                return Some("arrow_function_decl".into());
            } else {
                return Some("exported_variable".into());
            }
        }
    }
    None
}

/// True if a `function_declaration` node's body contains JSX.
fn function_body_contains_jsx(func_node: Node) -> bool {
    if let Some(body) = func_node.child_by_field_name("body") {
        return contains_jsx(body);
    }
    false
}

/// True if the arrow function value for the variable named by `name_node`
/// has a body containing JSX.
fn arrow_body_contains_jsx(symbol_node: Node, name_node: Node) -> bool {
    find_arrow_body(symbol_node, name_node)
        .map(contains_jsx)
        .unwrap_or(false)
}

/// Locate the `body` of the arrow function whose declarator matches `name_node`.
fn find_arrow_body<'a>(node: Node<'a>, name_node: Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "variable_declarator"
        && let Some(decl_name) = node.child_by_field_name("name")
        && decl_name.id() == name_node.id()
        && let Some(value) = node.child_by_field_name("value")
        && is_arrow_or_function_value(value)
    {
        return value.child_by_field_name("body");
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_arrow_body(child, name_node) {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Child-symbol extraction
// ---------------------------------------------------------------------------

/// Extract `property_signature` and `method_signature` children from an
/// `interface_body` as `SymbolKind::Property` child symbols.
fn extract_interface_children(iface_node: Node, source: &[u8]) -> Vec<SymbolInfo> {
    let mut children = Vec::new();
    // Find the interface_body child
    let body = {
        let mut found = None;
        let mut cursor = iface_node.walk();
        for child in iface_node.children(&mut cursor) {
            if child.kind() == "interface_body" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(b) => b,
            None => return children,
        }
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "property_signature" | "method_signature" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source).to_owned();
                    let pos = name_node.start_position();
                    children.push(SymbolInfo {
                        name,
                        kind: SymbolKind::Property,
                        line: pos.row + 1,
                        col: pos.column,
                        is_exported: false,
                        is_default: false,
                    });
                }
            }
            _ => {}
        }
    }
    children
}

/// Extract `method_definition` children from a `class_body` as
/// `SymbolKind::Method` child symbols.
fn extract_class_children(class_node: Node, source: &[u8]) -> Vec<SymbolInfo> {
    let mut children = Vec::new();
    let body = {
        let mut found = None;
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "class_body" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(b) => b,
            None => return children,
        }
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "method_definition"
            && let Some(name_node) = child.child_by_field_name("name")
        {
            let name = node_text(name_node, source).to_owned();
            let pos = name_node.start_position();
            children.push(SymbolInfo {
                name,
                kind: SymbolKind::Method,
                line: pos.row + 1,
                col: pos.column,
                is_exported: false,
                is_default: false,
            });
        }
    }
    children
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Which language group a file falls into (used for query selection).
enum LangKind {
    TypeScript,
    Tsx,
    JavaScript,
}

fn lang_kind(language: &Language, is_tsx: bool) -> LangKind {
    // Use the language name to distinguish TS / TSX / JS.
    match language.name().unwrap_or("") {
        "javascript" => LangKind::JavaScript,
        _ => {
            if is_tsx {
                LangKind::Tsx
            } else {
                LangKind::TypeScript
            }
        }
    }
}

/// Extract all symbols from a parsed syntax tree.
///
/// Returns a `Vec` of `(parent_symbol, child_symbols)` tuples.
///
/// # Parameters
/// - `tree`: the tree-sitter syntax tree
/// - `source`: the raw UTF-8 source bytes
/// - `language`: the grammar used to parse `source`
/// - `is_tsx`: `true` for `.tsx`/`.jsx` files — enables JSX component detection
pub fn extract_symbols(
    tree: &Tree,
    source: &[u8],
    language: &Language,
    is_tsx: bool,
) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let query = match lang_kind(language, is_tsx) {
        LangKind::JavaScript => js_query(language),
        LangKind::Tsx => tsx_query(language),
        LangKind::TypeScript => ts_query(language),
    };

    let name_idx = query
        .capture_index_for_name("name")
        .expect("query must have @name capture");
    let symbol_idx = query
        .capture_index_for_name("symbol")
        .expect("query must have @symbol capture");
    let val_idx = query.capture_index_for_name("val");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    // De-duplicate by (name, line) to avoid double-matches from overlapping patterns.
    let mut seen: std::collections::HashSet<(String, usize)> = std::collections::HashSet::new();
    let mut results: Vec<(SymbolInfo, Vec<SymbolInfo>)> = Vec::new();

    while let Some(m) = matches.next() {
        let mut symbol_node: Option<Node> = None;
        let mut name_node: Option<Node> = None;
        let mut val_node: Option<Node> = None;

        for capture in m.captures {
            if capture.index == symbol_idx {
                symbol_node = Some(capture.node);
            } else if capture.index == name_idx {
                name_node = Some(capture.node);
            } else if val_idx == Some(capture.index) {
                val_node = Some(capture.node);
            }
        }

        let (sym_node, name_node) = match (symbol_node, name_node) {
            (Some(s), Some(n)) => (s, n),
            _ => continue,
        };

        let name = node_text(name_node, source).to_owned();
        let pos = name_node.start_position();
        let key = (name.clone(), pos.row);

        // Skip duplicate matches (overlapping patterns)
        if !seen.insert(key) {
            continue;
        }

        // Classify the symbol kind
        let kind = match classify_symbol(sym_node, name_node, val_node, is_tsx, source) {
            Some(k) => k,
            None => continue,
        };

        // For exported-variable matches that are actually arrow functions — skip.
        if kind == SymbolKind::Variable
            && let Some(val) = val_node
            && is_arrow_or_function_value(val)
        {
            continue;
        }

        let (is_exported, is_default) = detect_export(sym_node, source);

        let info = SymbolInfo {
            name,
            kind: kind.clone(),
            line: pos.row + 1,
            col: pos.column,
            is_exported,
            is_default,
        };

        // Extract child symbols
        let children = match kind {
            SymbolKind::Interface => {
                let iface_node = find_declaration_node(sym_node, "interface_declaration");
                iface_node
                    .map(|n| extract_interface_children(n, source))
                    .unwrap_or_default()
            }
            SymbolKind::Class => {
                let class_node = find_declaration_node(sym_node, "class_declaration");
                class_node
                    .map(|n| extract_class_children(n, source))
                    .unwrap_or_default()
            }
            _ => vec![],
        };

        results.push((info, children));
    }

    results
}

/// Walk down from `node` to find a child (or the node itself) of kind `target_kind`.
fn find_declaration_node<'a>(node: Node<'a>, target_kind: &str) -> Option<Node<'a>> {
    if node.kind() == target_kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_declaration_node(child, target_kind) {
            return Some(found);
        }
    }
    None
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

    fn parse_tsx(source: &str) -> (tree_sitter::Tree, Language) {
        let lang = language_for_extension("tsx").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    fn first_symbol(results: &[(SymbolInfo, Vec<SymbolInfo>)]) -> &SymbolInfo {
        &results
            .first()
            .unwrap_or_else(|| panic!("expected at least one symbol, got none"))
            .0
    }

    // Test 1: Function declaration
    #[test]
    fn test_export_function_declaration() {
        let src = "export function hello() {}";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "hello");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.is_exported, "should be exported");
    }

    // Test 2: Exported const arrow function
    #[test]
    fn test_export_const_arrow_function() {
        let src = "export const greet = () => {};";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.is_exported, "should be exported");
    }

    // Test 3: Class declaration (non-exported)
    #[test]
    fn test_class_declaration() {
        let src = "class MyClass {}";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "MyClass");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert!(!sym.is_exported);
    }

    // Test 4: Interface with child symbols
    #[test]
    fn test_interface_with_children() {
        let src = "interface IUser { name: string; getId(): number; }";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let (sym, children) = results.first().expect("expected interface symbol");
        assert_eq!(sym.name, "IUser");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(children.len(), 2, "expected 2 child symbols (name, getId)");
        let child_names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"name"), "missing 'name' child");
        assert!(child_names.contains(&"getId"), "missing 'getId' child");
        assert!(
            children.iter().all(|c| c.kind == SymbolKind::Property),
            "all children should be Property kind"
        );
    }

    // Test 5: Type alias
    #[test]
    fn test_type_alias() {
        let src = "type ID = string;";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "ID");
        assert_eq!(sym.kind, SymbolKind::TypeAlias);
    }

    // Test 6: Enum declaration
    #[test]
    fn test_enum_declaration() {
        let src = "enum Color { Red, Blue }";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "Color");
        assert_eq!(sym.kind, SymbolKind::Enum);
    }

    // Test 7: React component in TSX
    #[test]
    fn test_tsx_component_detection() {
        let src = "export const App = () => <div/>;";
        let (tree, lang) = parse_tsx(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, true);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "App");
        assert_eq!(sym.kind, SymbolKind::Component);
        assert!(sym.is_exported);
    }

    // Bonus: Non-JSX arrow function in TSX should stay as Function
    #[test]
    fn test_tsx_non_component_arrow_fn() {
        let src = "export const add = (a: number, b: number) => a + b;";
        let (tree, lang) = parse_tsx(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, true);
        let sym = first_symbol(&results);
        assert_eq!(sym.name, "add");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    // Bonus: class with methods
    #[test]
    fn test_class_with_methods() {
        let src = "class Dog { bark() {} sit() {} }";
        let (tree, lang) = parse_ts(src);
        let results = extract_symbols(&tree, src.as_bytes(), &lang, false);
        let (sym, children) = results.first().expect("expected class");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(children.len(), 2, "expected 2 methods");
        assert!(children.iter().all(|c| c.kind == SymbolKind::Method));
    }
}

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

use crate::graph::node::{DecoratorInfo, SymbolInfo, SymbolKind, SymbolVisibility};

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Determine Go symbol visibility: exported = first letter uppercase.
fn go_visibility(name: &str) -> (SymbolVisibility, bool) {
    let exported = name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    if exported {
        (SymbolVisibility::Pub, true)
    } else {
        (SymbolVisibility::Private, false)
    }
}

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

/// Tree-sitter query for Go primary symbol declarations.
///
/// Captures function declarations, method declarations, type declarations (type_spec),
/// and type aliases (type_alias). Const/var blocks use manual walking due to
/// multi-name specs (e.g. `const A, B = 1, 2`).
const GO_SYMBOL_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @symbol
(method_declaration name: (field_identifier) @name) @symbol
(type_declaration (type_spec name: (type_identifier) @name)) @symbol
(type_declaration (type_alias name: (type_identifier) @name)) @symbol
"#;

static GO_SYMBOL_QUERY_CACHE: std::sync::OnceLock<Query> = std::sync::OnceLock::new();

fn go_symbol_query(language: &Language) -> &'static Query {
    GO_SYMBOL_QUERY_CACHE
        .get_or_init(|| Query::new(language, GO_SYMBOL_QUERY).expect("invalid Go symbol query"))
}

// ---------------------------------------------------------------------------
// Decorator extraction — struct tags and go: directives
// ---------------------------------------------------------------------------

/// Parse struct tags from a struct_type node.
///
/// Walks `struct_type → field_declaration_list → field_declaration → tag`
/// and creates one `DecoratorInfo` per unique tag key (e.g. `json`, `gorm`).
///
/// Tag format (Go raw string): `` `json:"name,omitempty" gorm:"column:id"` ``
fn extract_struct_tags(struct_type_node: Node, source: &[u8]) -> Vec<DecoratorInfo> {
    let mut decorators = Vec::new();
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Find field_declaration_list
    let fdl = {
        let mut cursor = struct_type_node.walk();
        let mut found = None;
        for child in struct_type_node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return decorators,
        }
    };

    let mut cursor = fdl.walk();
    for field in fdl.children(&mut cursor) {
        if field.kind() != "field_declaration" {
            continue;
        }

        // Find tag child (raw_string_literal or interpreted_string_literal)
        let tag_node = {
            let mut c = field.walk();
            let mut found = None;
            for child in field.children(&mut c) {
                if child.kind() == "raw_string_literal"
                    || child.kind() == "interpreted_string_literal"
                {
                    // The tag node is the last string literal in a field_declaration
                    found = Some(child);
                }
            }
            found
        };

        let tag_node = match tag_node {
            Some(n) => n,
            None => continue,
        };

        // Get the raw tag text, stripping backticks
        let tag_text = node_text(tag_node, source);
        let tag_content = tag_text.trim_matches('`');

        // Parse key:"value" pairs using simple scanning
        parse_struct_tag_keys(tag_content, &mut seen_keys, &mut decorators);
    }

    decorators
}

/// Parse Go struct tag string to extract unique key names.
///
/// Format: `key:"value,options" key2:"value2"` (space-separated key:"value" pairs).
fn parse_struct_tag_keys(
    tag_content: &str,
    seen_keys: &mut std::collections::HashSet<String>,
    decorators: &mut Vec<DecoratorInfo>,
) {
    let mut remaining = tag_content.trim();
    while !remaining.is_empty() {
        // Find the key (up to the colon)
        let colon_pos = match remaining.find(':') {
            Some(p) => p,
            None => break,
        };
        let key = remaining[..colon_pos].trim().to_owned();
        remaining = &remaining[colon_pos + 1..];

        // The value must start with a quote
        let quote_char = remaining.chars().next();
        let quote = match quote_char {
            Some('"') => '"',
            Some('`') => '`',
            _ => break,
        };
        remaining = &remaining[1..]; // skip opening quote

        // Find closing quote
        let end_pos = match remaining.find(quote) {
            Some(p) => p,
            None => break,
        };
        let value = remaining[..end_pos].to_owned();
        remaining = remaining[end_pos + 1..].trim_start();

        if !key.is_empty() && !seen_keys.contains(&key) {
            seen_keys.insert(key.clone());
            decorators.push(DecoratorInfo {
                name: key,
                args_raw: Some(value),
                object: None,
                attribute: None,
                framework: None,
            });
        }
    }
}

/// Extract Go compiler directives (//go:...) from comment nodes preceding a symbol.
///
/// Walks the parent's children up to `symbol_node`, collecting `attribute_item`-like
/// comment nodes that start with `//go:`.
fn extract_go_directives(symbol_node: Node, source: &[u8]) -> Vec<DecoratorInfo> {
    let mut directives = Vec::new();

    let parent = match symbol_node.parent() {
        Some(p) => p,
        None => return directives,
    };

    // Walk parent's children up to symbol_node, collecting preceding comment siblings
    let mut cursor = parent.walk();
    let mut preceding_comments: Vec<Node> = Vec::new();
    for child in parent.children(&mut cursor) {
        if child.id() == symbol_node.id() {
            break;
        }
        if child.kind() == "comment" {
            preceding_comments.push(child);
        } else {
            // Non-comment resets the "adjacent" comment window
            preceding_comments.clear();
        }
    }

    for comment_node in preceding_comments {
        let text = node_text(comment_node, source);
        if let Some(rest) = text.strip_prefix("//go:") {
            // rest = "generate stringer -type=Weekday" etc.
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            let directive_name = format!("go:{}", parts[0]);
            let args = parts.get(1).map(|s| s.trim().to_owned());
            directives.push(DecoratorInfo {
                name: directive_name,
                args_raw: args,
                object: None,
                attribute: None,
                framework: None,
            });
        }
    }

    directives
}

// ---------------------------------------------------------------------------
// Embedded fields detection
// ---------------------------------------------------------------------------

/// Extract embedded type names from a struct type node.
///
/// An embedded field in Go is a `field_declaration` with a `type` field but no `name` field.
/// We store embedded type names as a sentinel `DecoratorInfo` with `name = "__embedded__"`
/// so Plan 03 can create `Embeds` edges without re-parsing.
fn extract_embedded_fields(struct_type_node: Node, source: &[u8]) -> Option<DecoratorInfo> {
    let mut embedded_names = Vec::new();

    let fdl = {
        let mut cursor = struct_type_node.walk();
        let mut found = None;
        for child in struct_type_node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                found = Some(child);
                break;
            }
        }
        found?
    };

    let mut cursor = fdl.walk();
    for field in fdl.children(&mut cursor) {
        if field.kind() != "field_declaration" {
            continue;
        }

        // Embedded field: has a type but no name field
        let has_name = field.child_by_field_name("name").is_some();
        if has_name {
            continue;
        }

        // Get the type node
        let type_node = match field.child_by_field_name("type") {
            Some(n) => n,
            None => continue,
        };

        // Resolve the embedded type name (handle pointer_type and qualified_type)
        let type_name = resolve_type_name(type_node, source);
        if !type_name.is_empty() {
            embedded_names.push(type_name.to_owned());
        }
    }

    if embedded_names.is_empty() {
        return None;
    }

    Some(DecoratorInfo {
        name: "__embedded__".to_owned(),
        args_raw: Some(embedded_names.join(",")),
        object: None,
        attribute: None,
        framework: None,
    })
}

/// Resolve the name of a type node, handling pointer_type, qualified_type, generic_type.
fn resolve_type_name<'a>(type_node: Node<'a>, source: &'a [u8]) -> &'a str {
    match type_node.kind() {
        "type_identifier" => node_text(type_node, source),
        "pointer_type" => {
            // pointer_type has a child type_identifier
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" => return node_text(child, source),
                    "qualified_type" | "generic_type" => {
                        return resolve_type_name(child, source);
                    }
                    _ => {}
                }
            }
            ""
        }
        "qualified_type" => {
            // qualified_type: package.Type — return the Type part (name field)
            type_node
                .child_by_field_name("name")
                .map(|n| node_text(n, source))
                .unwrap_or("")
        }
        "generic_type" => {
            // generic_type: Type[Params] — return the type field
            type_node
                .child_by_field_name("type")
                .map(|n| node_text(n, source))
                .unwrap_or("")
        }
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Interface method extraction
// ---------------------------------------------------------------------------

/// Extract method signatures from an interface_type node as child SymbolInfo entries.
fn extract_interface_methods(interface_node: Node, source: &[u8]) -> Vec<SymbolInfo> {
    let mut methods = Vec::new();

    // interface_type can have method_elem children in tree-sitter-go 0.25
    let mut cursor = interface_node.walk();
    for child in interface_node.children(&mut cursor) {
        if child.kind() == "method_elem" {
            // method_elem has a name (field_identifier) child
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, source).to_owned();
                let pos = name_node.start_position();
                let (visibility, is_exported) = go_visibility(&name);
                methods.push(SymbolInfo {
                    name,
                    kind: SymbolKind::Method,
                    line: pos.row + 1,
                    col: pos.column,
                    line_end: child.end_position().row + 1,
                    is_exported,
                    is_default: false,
                    visibility,
                    trait_impl: None,
                    decorators: Vec::new(),
                });
            }
        }
    }

    methods
}

// ---------------------------------------------------------------------------
// Method receiver extraction
// ---------------------------------------------------------------------------

/// Extract the receiver struct name from a method_declaration node.
///
/// Go method: `func (r *Router) Handle() {}`
/// Returns the base type name (e.g., "Router"), ignoring pointer indirection.
fn extract_receiver_type(method_node: Node, source: &[u8]) -> Option<String> {
    // receiver field is a parameter_list
    let receiver_list = method_node.child_by_field_name("receiver")?;

    // Walk the parameter_list to find the first parameter_declaration
    let mut cursor = receiver_list.walk();
    for child in receiver_list.children(&mut cursor) {
        if child.kind() == "parameter_declaration"
            || child.kind() == "variadic_parameter_declaration"
        {
            // The type field of the parameter_declaration
            let type_node = child.child_by_field_name("type")?;
            let type_name = resolve_type_name(type_node, source);
            if !type_name.is_empty() {
                return Some(type_name.to_owned());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Const/var block walking
// ---------------------------------------------------------------------------

/// Walk const_declaration and var_declaration blocks to extract individual symbols.
///
/// These blocks can have multiple spec nodes, each with potentially multiple names.
/// E.g.: `const ( A, B = 1, 2; C = 3 )` → A, B, C all as Const symbols.
///
/// Note: `var_declaration` in tree-sitter-go can contain either:
///   - Direct `var_spec` children (single var: `var x int`)
///   - A `var_spec_list` child that wraps multiple `var_spec` nodes (block: `var ( x int; y string )`)
///
/// `const_declaration` always has direct `const_spec` children.
fn walk_const_var_declarations(root: Node, source: &[u8]) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let mut results = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "const_declaration" => {
                let mut spec_cursor = child.walk();
                for spec in child.children(&mut spec_cursor) {
                    if spec.kind() == "const_spec" {
                        extract_spec_names(spec, source, SymbolKind::Const, &mut results);
                    }
                }
            }
            "var_declaration" => {
                let mut spec_cursor = child.walk();
                for spec_or_list in child.children(&mut spec_cursor) {
                    match spec_or_list.kind() {
                        "var_spec" => {
                            extract_spec_names(
                                spec_or_list,
                                source,
                                SymbolKind::Variable,
                                &mut results,
                            );
                        }
                        "var_spec_list" => {
                            // var_spec_list wraps multiple var_spec nodes (block form)
                            let mut list_cursor = spec_or_list.walk();
                            for spec in spec_or_list.children(&mut list_cursor) {
                                if spec.kind() == "var_spec" {
                                    extract_spec_names(
                                        spec,
                                        source,
                                        SymbolKind::Variable,
                                        &mut results,
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    results
}

/// Extract individual names from a const_spec or var_spec node.
fn extract_spec_names(
    spec_node: Node,
    source: &[u8],
    kind: SymbolKind,
    results: &mut Vec<(SymbolInfo, Vec<SymbolInfo>)>,
) {
    // Specs can have multiple "name" field children (e.g. `const A, B = 1, 2`)
    let mut name_cursor = spec_node.walk();
    for name_child in spec_node.children_by_field_name("name", &mut name_cursor) {
        let name = node_text(name_child, source).to_owned();
        let pos = name_child.start_position();
        let (visibility, is_exported) = go_visibility(&name);
        let symbol = SymbolInfo {
            name,
            kind: kind.clone(),
            line: pos.row + 1,
            col: pos.column,
            line_end: spec_node.end_position().row + 1,
            is_exported,
            is_default: false,
            visibility,
            trait_impl: None,
            decorators: Vec::new(),
        };
        results.push((symbol, Vec::new()));
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract all top-level symbols from a Go source file.
///
/// Returns `Vec<(parent_symbol, child_symbols)>` where child_symbols are:
/// - Interface method signatures (for interface types)
///
/// Method receiver type is stored in `SymbolInfo::trait_impl` for later
/// Plan 03 edge wiring (ChildOf edges from method → struct).
///
/// Struct tags are stored as `DecoratorInfo` on the struct symbol.
/// Go compiler directives (`//go:`) are stored as `DecoratorInfo` on the following symbol.
/// Embedded fields are stored as a sentinel `DecoratorInfo { name: "__embedded__" }`.
pub fn extract_go_symbols(
    tree: &Tree,
    source: &[u8],
    language: &Language,
) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
    let root = tree.root_node();
    let mut results = Vec::new();

    // --- Query-based extraction: functions, methods, type_spec, type_alias ---
    let query = go_symbol_query(language);
    let name_idx = query
        .capture_index_for_name("name")
        .expect("Go symbol query must have @name");
    let symbol_idx = query
        .capture_index_for_name("symbol")
        .expect("Go symbol query must have @symbol");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);

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

        let sym_kind = sym_n.kind();
        let name = node_text(name_n, source).to_owned();
        let pos = name_n.start_position();
        let (visibility, is_exported) = go_visibility(&name);

        // The actual node for line_end is the outermost node.
        // For type_spec inside type_declaration, we want the type_declaration for line_end.
        let outer_node = if sym_kind == "function_declaration" || sym_kind == "method_declaration" {
            sym_n
        } else {
            // type_spec / type_alias — the @symbol capture is the type_declaration wrapper
            sym_n
        };

        match sym_kind {
            "function_declaration" => {
                let symbol = SymbolInfo {
                    name,
                    kind: SymbolKind::Function,
                    line: pos.row + 1,
                    col: pos.column,
                    line_end: outer_node.end_position().row + 1,
                    is_exported,
                    is_default: false,
                    visibility,
                    trait_impl: None,
                    decorators: extract_go_directives(sym_n, source),
                };
                results.push((symbol, Vec::new()));
            }
            "method_declaration" => {
                let receiver = extract_receiver_type(sym_n, source);
                let symbol = SymbolInfo {
                    name,
                    kind: SymbolKind::Method,
                    line: pos.row + 1,
                    col: pos.column,
                    line_end: outer_node.end_position().row + 1,
                    is_exported,
                    is_default: false,
                    visibility,
                    trait_impl: receiver,
                    decorators: extract_go_directives(sym_n, source),
                };
                results.push((symbol, Vec::new()));
            }
            "type_declaration" => {
                // The @symbol for type_spec and type_alias captures the parent type_declaration.
                // Walk it to find the inner spec.
                let mut inner_cursor = sym_n.walk();
                for inner in sym_n.children(&mut inner_cursor) {
                    match inner.kind() {
                        "type_spec" => {
                            let spec_name_node = match inner.child_by_field_name("name") {
                                Some(n) => n,
                                None => continue,
                            };
                            let spec_name = node_text(spec_name_node, source).to_owned();
                            let spec_pos = spec_name_node.start_position();
                            let (spec_vis, spec_exported) = go_visibility(&spec_name);

                            // Determine what kind of type this is
                            let type_child = inner.child_by_field_name("type");
                            let (kind, children, extra_decorators) =
                                match type_child.map(|n| n.kind()) {
                                    Some("struct_type") => {
                                        let struct_node = type_child.unwrap();
                                        let tags = extract_struct_tags(struct_node, source);
                                        let embedded = extract_embedded_fields(struct_node, source);
                                        let mut extra = tags;
                                        if let Some(emb) = embedded {
                                            extra.push(emb);
                                        }
                                        (SymbolKind::Struct, Vec::new(), extra)
                                    }
                                    Some("interface_type") => {
                                        let iface_node = type_child.unwrap();
                                        let methods = extract_interface_methods(iface_node, source);
                                        (SymbolKind::Interface, methods, Vec::new())
                                    }
                                    _ => (SymbolKind::TypeAlias, Vec::new(), Vec::new()),
                                };

                            let mut decorators = extract_go_directives(sym_n, source);
                            decorators.extend(extra_decorators);

                            let symbol = SymbolInfo {
                                name: spec_name,
                                kind,
                                line: spec_pos.row + 1,
                                col: spec_pos.column,
                                line_end: sym_n.end_position().row + 1,
                                is_exported: spec_exported,
                                is_default: false,
                                visibility: spec_vis,
                                trait_impl: None,
                                decorators,
                            };
                            results.push((symbol, children));
                        }
                        "type_alias" => {
                            let alias_name_node = match inner.child_by_field_name("name") {
                                Some(n) => n,
                                None => continue,
                            };
                            let alias_name = node_text(alias_name_node, source).to_owned();
                            let alias_pos = alias_name_node.start_position();
                            let (alias_vis, alias_exported) = go_visibility(&alias_name);

                            let decorators = extract_go_directives(sym_n, source);
                            let symbol = SymbolInfo {
                                name: alias_name,
                                kind: SymbolKind::TypeAlias,
                                line: alias_pos.row + 1,
                                col: alias_pos.column,
                                line_end: sym_n.end_position().row + 1,
                                is_exported: alias_exported,
                                is_default: false,
                                visibility: alias_vis,
                                trait_impl: None,
                                decorators,
                            };
                            results.push((symbol, Vec::new()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // --- Manual walk for const/var declarations ---
    results.extend(walk_const_var_declarations(root, source));

    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::languages::language_for_extension;

    fn parse_go(source: &str) -> (Tree, Language) {
        let lang = language_for_extension("go").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    fn extract(source: &str) -> Vec<(SymbolInfo, Vec<SymbolInfo>)> {
        let (tree, lang) = parse_go(source);
        extract_go_symbols(&tree, source.as_bytes(), &lang)
    }

    // Test 1: basic exported function
    #[test]
    fn test_go_function() {
        let src = "package main\n\nfunc Hello() {}\n";
        let syms = extract(src);
        let (sym, children) = syms.iter().find(|(s, _)| s.name == "Hello").unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
        assert!(children.is_empty());
    }

    // Test 2: unexported function
    #[test]
    fn test_go_unexported_function() {
        let src = "package main\n\nfunc helper() {}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "helper").unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, SymbolVisibility::Private);
        assert!(!sym.is_exported);
    }

    // Test 3: Go has no async keyword — regular function
    #[test]
    fn test_go_async_function_not_special() {
        // Go goroutines use `go` keyword, not `async`. This is just a regular function.
        let src = "package main\n\nfunc Fetch() {}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "Fetch").unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    // Test 4: method with pointer receiver
    #[test]
    fn test_go_method_declaration() {
        let src = "package main\n\ntype Router struct{}\n\nfunc (r *Router) Handle() {}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "Handle").unwrap();
        assert_eq!(sym.kind, SymbolKind::Method);
        assert_eq!(sym.trait_impl.as_deref(), Some("Router"));
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 5: method with value receiver
    #[test]
    fn test_go_method_value_receiver() {
        let src = "package main\n\ntype Router struct{}\n\nfunc (r Router) Get() {}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "Get").unwrap();
        assert_eq!(sym.kind, SymbolKind::Method);
        assert_eq!(sym.trait_impl.as_deref(), Some("Router"));
    }

    // Test 6: exported struct
    #[test]
    fn test_go_struct() {
        let src = "package main\n\ntype User struct { Name string }\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "User").unwrap();
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 7: interface
    #[test]
    fn test_go_interface() {
        let src = "package main\n\ntype Reader interface { Read() }\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "Reader").unwrap();
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 8: type alias (with =)
    #[test]
    fn test_go_type_alias() {
        let src = "package main\n\ntype ID = string\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "ID").unwrap();
        assert_eq!(sym.kind, SymbolKind::TypeAlias);
    }

    // Test 9: type definition (no =, wrapping type)
    #[test]
    fn test_go_type_definition() {
        let src = "package main\n\ntype ID string\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "ID").unwrap();
        assert_eq!(sym.kind, SymbolKind::TypeAlias);
    }

    // Test 10: single const
    #[test]
    fn test_go_const_single() {
        let src = "package main\n\nconst MaxSize = 100\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "MaxSize").unwrap();
        assert_eq!(sym.kind, SymbolKind::Const);
        assert_eq!(sym.visibility, SymbolVisibility::Pub);
        assert!(sym.is_exported);
    }

    // Test 11: const block with multiple names
    #[test]
    fn test_go_const_block_multiple() {
        let src = "package main\n\nconst (\n\tA = 1\n\tB = 2\n)\n";
        let syms = extract(src);
        let a = syms.iter().find(|(s, _)| s.name == "A");
        let b = syms.iter().find(|(s, _)| s.name == "B");
        assert!(a.is_some(), "Should have const A");
        assert!(b.is_some(), "Should have const B");
        assert_eq!(a.unwrap().0.kind, SymbolKind::Const);
        assert_eq!(b.unwrap().0.kind, SymbolKind::Const);
    }

    // Test 12: single var
    #[test]
    fn test_go_var_single() {
        let src = "package main\n\nvar count int\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "count").unwrap();
        assert_eq!(sym.kind, SymbolKind::Variable);
        assert_eq!(sym.visibility, SymbolVisibility::Private);
        assert!(!sym.is_exported);
    }

    // Test 13: var block with multiple names
    #[test]
    fn test_go_var_block_multiple() {
        let src = "package main\n\nvar (\n\tx int\n\ty string\n)\n";
        let syms = extract(src);
        let x = syms.iter().find(|(s, _)| s.name == "x");
        let y = syms.iter().find(|(s, _)| s.name == "y");
        assert!(x.is_some(), "Should have var x");
        assert!(y.is_some(), "Should have var y");
        assert_eq!(x.unwrap().0.kind, SymbolKind::Variable);
        assert_eq!(y.unwrap().0.kind, SymbolKind::Variable);
    }

    // Test 14: init() function is indexed
    #[test]
    fn test_go_init_function() {
        let src = "package main\n\nfunc init() {}\n";
        let syms = extract(src);
        let found = syms.iter().find(|(s, _)| s.name == "init");
        assert!(found.is_some(), "init() should be indexed");
        let (sym, _) = found.unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, SymbolVisibility::Private);
        assert!(!sym.is_exported);
    }

    // Test 15: TestXxx functions are indexed
    #[test]
    fn test_go_test_function() {
        let src = "package main\n\nimport \"testing\"\n\nfunc TestMyFeature(t *testing.T) {}\n";
        let syms = extract(src);
        let found = syms.iter().find(|(s, _)| s.name == "TestMyFeature");
        assert!(found.is_some(), "TestMyFeature should be indexed");
        let (sym, _) = found.unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.is_exported);
    }

    // Test 16: struct tags → DecoratorInfo
    #[test]
    fn test_go_struct_tags() {
        let src = r#"package main

type User struct {
    ID   int    `json:"id" gorm:"primaryKey"`
    Name string `json:"name"`
}
"#;
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "User").unwrap();
        assert_eq!(sym.kind, SymbolKind::Struct);
        let dec_names: Vec<_> = sym
            .decorators
            .iter()
            .map(|d| d.name.as_str())
            .filter(|n| !n.starts_with("__"))
            .collect();
        assert!(dec_names.contains(&"json"), "Should have json decorator");
        assert!(dec_names.contains(&"gorm"), "Should have gorm decorator");
    }

    // Test 17: go:generate directive → DecoratorInfo
    #[test]
    fn test_go_compiler_directive() {
        let src = "package main\n\n//go:generate stringer -type=Weekday\nfunc weekday() {}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "weekday").unwrap();
        let directive = sym.decorators.iter().find(|d| d.name == "go:generate");
        assert!(directive.is_some(), "Should have go:generate decorator");
        assert!(
            directive
                .unwrap()
                .args_raw
                .as_deref()
                .unwrap_or("")
                .contains("stringer"),
            "args_raw should contain 'stringer'"
        );
    }

    // Test 18: multi-line function → line_end > line
    #[test]
    fn test_go_line_end() {
        let src = "package main\n\nfunc multiLine() {\n    x := 1\n    _ = x\n}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "multiLine").unwrap();
        assert!(
            sym.line_end > sym.line,
            "line_end ({}) should be > line ({})",
            sym.line_end,
            sym.line
        );
    }

    // Test 19: embedded field detection
    #[test]
    fn test_go_embedded_field() {
        let src = "package main\n\ntype Server struct {\n    http.Handler\n    port int\n}\n";
        let syms = extract(src);
        let (sym, _) = syms.iter().find(|(s, _)| s.name == "Server").unwrap();
        let embedded = sym.decorators.iter().find(|d| d.name == "__embedded__");
        assert!(embedded.is_some(), "Should have __embedded__ decorator");
        let args = embedded.unwrap().args_raw.as_deref().unwrap_or("");
        assert!(
            args.contains("Handler"),
            "Embedded field should be 'Handler'"
        );
    }

    // Test 20: visibility rules
    #[test]
    fn test_go_visibility() {
        let src = "package main\n\nfunc Exported() {}\nfunc unexported() {}\n";
        let syms = extract(src);
        let exported = syms.iter().find(|(s, _)| s.name == "Exported").unwrap();
        let unexported = syms.iter().find(|(s, _)| s.name == "unexported").unwrap();
        assert_eq!(exported.0.visibility, SymbolVisibility::Pub);
        assert!(exported.0.is_exported);
        assert_eq!(unexported.0.visibility, SymbolVisibility::Private);
        assert!(!unexported.0.is_exported);
    }

    // Test 21: interface methods as child symbols
    #[test]
    fn test_go_interface_methods() {
        let src =
            "package main\n\ntype Writer interface {\n    Write() int\n    Close() error\n}\n";
        let syms = extract(src);
        let (sym, children) = syms.iter().find(|(s, _)| s.name == "Writer").unwrap();
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert_eq!(
            children.len(),
            2,
            "Interface should have 2 method children, got {:?}",
            children.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Write"));
        assert!(names.contains(&"Close"));
    }
}

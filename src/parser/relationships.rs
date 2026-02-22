use std::sync::OnceLock;

use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// The kind of symbol-level relationship.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RelationshipKind {
    /// Direct function call: `foo()`
    Calls,
    /// Method call: `obj.method()` — stores the method name, not the object
    MethodCall,
    /// Class extends class: `class Foo extends Bar`
    Extends,
    /// Class implements interface: `class Foo implements IBar`
    Implements,
    /// Interface extends interface: `interface IFoo extends IBar`
    InterfaceExtends,
    /// Type reference in annotation: `const x: SomeType`, `param: SomeType`
    TypeReference,
}

/// A single symbol-level relationship extracted from a source file.
#[derive(Debug, Clone)]
pub struct RelationshipInfo {
    /// The name of the source symbol (caller, child class, etc.).
    /// `None` for top-level calls not inside a named function (context-free extraction).
    pub from_name: Option<String>,
    /// The name of the target symbol (callee, parent class, interface, type).
    pub to_name: String,
    /// The kind of relationship.
    pub kind: RelationshipKind,
    /// 1-based line number of the relationship site.
    pub line: usize,
}

// ---------------------------------------------------------------------------
// Query strings
// ---------------------------------------------------------------------------

/// Query for direct function calls and method calls.
///
/// Pattern 1: `foo(...)` — direct call to an identifier.
/// Pattern 2: `obj.method(...)` — method call on any object.
const CALLS_QUERY: &str = r#"
    ; Direct call: foo(...)
    (call_expression
      function: (identifier) @callee_name
      arguments: (arguments))

    ; Method call: obj.method(...)
    (call_expression
      function: (member_expression
        property: (property_identifier) @method_name)
      arguments: (arguments))
"#;

/// Query for class/interface inheritance relationships.
///
/// Pattern 1: `class Foo extends Bar` — class-to-class inheritance.
/// Pattern 2: `class Foo implements IBar` — class-to-interface implementation.
/// Pattern 3: `interface IFoo extends IBar` — interface-to-interface inheritance.
///
/// Note: In the TypeScript tree-sitter grammar (0.23), interface extends uses
/// `extends_type_clause` (not `extends_clause` which is for class extends).
/// This was validated against the actual grammar node kinds.
const INHERITANCE_QUERY: &str = r#"
    ; class Foo extends Bar
    (class_declaration
      name: (type_identifier) @class_name
      (class_heritage
        (extends_clause
          value: (identifier) @extends_name)))

    ; class Foo implements IBar
    (class_declaration
      name: (type_identifier) @class_name
      (class_heritage
        (implements_clause
          (type_identifier) @implements_name)))

    ; interface IFoo extends IBar
    (interface_declaration
      name: (type_identifier) @iface_name
      (extends_type_clause
        (type_identifier) @parent_iface_name))
"#;

/// Query for type annotation references.
///
/// Captures type identifiers used in type positions: `const x: SomeType`.
const TYPE_REF_QUERY: &str = r#"
    ; Type annotation: const x: SomeType, param: SomeType
    (type_annotation
      (type_identifier) @type_ref)
"#;

// ---------------------------------------------------------------------------
// Query cache — one set of statics per grammar (TS / TSX / JS).
//
// Queries compiled for one grammar cannot be used with another grammar's tree.
// Following the exact pattern established in imports.rs and symbols.rs.
// ---------------------------------------------------------------------------

// TypeScript (.ts)
static TS_CALLS_QUERY: OnceLock<Query> = OnceLock::new();
static TS_INHERITANCE_QUERY: OnceLock<Query> = OnceLock::new();
static TS_TYPE_REF_QUERY: OnceLock<Query> = OnceLock::new();

// TypeScript-TSX (.tsx / .jsx)
static TSX_CALLS_QUERY: OnceLock<Query> = OnceLock::new();
static TSX_INHERITANCE_QUERY: OnceLock<Query> = OnceLock::new();
static TSX_TYPE_REF_QUERY: OnceLock<Query> = OnceLock::new();

// JavaScript (.js)
static JS_CALLS_QUERY: OnceLock<Query> = OnceLock::new();
static JS_INHERITANCE_QUERY: OnceLock<Query> = OnceLock::new();
// Note: JS has no type annotations, so JS_TYPE_REF_QUERY is intentionally absent.

/// Language group for query dispatch.
///
/// Note: `Language::name()` returns `None` for TypeScript/TSX grammars in
/// tree-sitter 0.26. We use `is_tsx` (derived from file extension) for TS vs
/// TSX discrimination, and `language.name() == Some("javascript")` for JS.
/// This mirrors the pattern established in imports.rs.
enum LangGroup {
    TypeScript,
    Tsx,
    JavaScript,
}

fn lang_group(language: &Language, is_tsx: bool) -> LangGroup {
    match language.name().unwrap_or("") {
        "javascript" => LangGroup::JavaScript,
        _ => {
            if is_tsx {
                LangGroup::Tsx
            } else {
                LangGroup::TypeScript
            }
        }
    }
}

fn calls_query(language: &Language, is_tsx: bool) -> &'static Query {
    match lang_group(language, is_tsx) {
        LangGroup::TypeScript => TS_CALLS_QUERY.get_or_init(|| {
            Query::new(language, CALLS_QUERY).expect("invalid TS calls query")
        }),
        LangGroup::Tsx => TSX_CALLS_QUERY.get_or_init(|| {
            Query::new(language, CALLS_QUERY).expect("invalid TSX calls query")
        }),
        LangGroup::JavaScript => JS_CALLS_QUERY.get_or_init(|| {
            Query::new(language, CALLS_QUERY).expect("invalid JS calls query")
        }),
    }
}

fn inheritance_query(language: &Language, is_tsx: bool) -> Option<&'static Query> {
    // JavaScript does not have interface declarations or TypeScript-style implements.
    // The INHERITANCE_QUERY contains TS-specific nodes; compile only for TS/TSX.
    match lang_group(language, is_tsx) {
        LangGroup::TypeScript => Some(TS_INHERITANCE_QUERY.get_or_init(|| {
            Query::new(language, INHERITANCE_QUERY).expect("invalid TS inheritance query")
        })),
        LangGroup::Tsx => Some(TSX_INHERITANCE_QUERY.get_or_init(|| {
            Query::new(language, INHERITANCE_QUERY).expect("invalid TSX inheritance query")
        })),
        LangGroup::JavaScript => {
            // JavaScript grammar supports class extends but not implements/interface.
            // In the JS grammar (tree-sitter-javascript 0.25), class_heritage contains
            // `extends` keyword + `identifier` directly — there is no `extends_clause` node.
            Some(JS_INHERITANCE_QUERY.get_or_init(|| {
                const JS_INHERITANCE_QUERY: &str = r#"
                    ; class Foo extends Bar (JS class_heritage layout differs from TS)
                    (class_declaration
                      name: (identifier) @class_name
                      (class_heritage
                        (identifier) @extends_name))
                "#;
                Query::new(language, JS_INHERITANCE_QUERY).expect("invalid JS inheritance query")
            }))
        }
    }
}

fn type_ref_query(language: &Language, is_tsx: bool) -> Option<&'static Query> {
    // Type annotations are TypeScript-only. Skip for JavaScript.
    match lang_group(language, is_tsx) {
        LangGroup::TypeScript => Some(TS_TYPE_REF_QUERY.get_or_init(|| {
            Query::new(language, TYPE_REF_QUERY).expect("invalid TS type_ref query")
        })),
        LangGroup::Tsx => Some(TSX_TYPE_REF_QUERY.get_or_init(|| {
            Query::new(language, TYPE_REF_QUERY).expect("invalid TSX type_ref query")
        })),
        LangGroup::JavaScript => None, // JS has no type annotations
    }
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extract all symbol-level relationships from a parsed syntax tree.
///
/// This is a context-free extraction pass: `from_name` is `None` for calls
/// and type references because determining the enclosing function/symbol would
/// require a separate scope-resolution pass (done in Plan 03 during graph
/// wiring). For inheritance relationships, `from_name` is the class or
/// interface name (which is always directly available in the grammar).
///
/// # Parameters
/// - `tree`: the tree-sitter syntax tree
/// - `source`: the raw UTF-8 source bytes
/// - `language`: the grammar used to parse `source`
/// - `is_tsx`: `true` for `.tsx`/`.jsx` files — used for grammar-specific query cache selection
///
/// # Returns
/// A deduplicated `Vec<RelationshipInfo>` with all extracted relationships.
/// Deduplication key: `(to_name, line, kind)` — matches the `(name, row)` strategy in symbols.rs.
pub fn extract_relationships(
    tree: &Tree,
    source: &[u8],
    language: &Language,
    is_tsx: bool,
) -> Vec<RelationshipInfo> {
    let mut results: Vec<RelationshipInfo> = Vec::new();
    let mut seen: std::collections::HashSet<(String, usize, String)> = std::collections::HashSet::new();

    // Helper to deduplicate and push
    macro_rules! push_rel {
        ($info:expr) => {{
            let info: RelationshipInfo = $info;
            let key = (
                info.to_name.clone(),
                info.line,
                format!("{:?}", info.kind),
            );
            if seen.insert(key) {
                results.push(info);
            }
        }};
    }

    // --- Calls (direct function calls and method calls) ---
    {
        let query = calls_query(language, is_tsx);
        let callee_idx = query
            .capture_index_for_name("callee_name")
            .expect("calls query must have @callee_name");
        let method_idx = query
            .capture_index_for_name("method_name")
            .expect("calls query must have @method_name");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let text = node_text(capture.node, source);
                let line = capture.node.start_position().row + 1;

                if capture.index == callee_idx {
                    push_rel!(RelationshipInfo {
                        from_name: None,
                        to_name: text.to_owned(),
                        kind: RelationshipKind::Calls,
                        line,
                    });
                } else if capture.index == method_idx {
                    push_rel!(RelationshipInfo {
                        from_name: None,
                        to_name: text.to_owned(),
                        kind: RelationshipKind::MethodCall,
                        line,
                    });
                }
            }
        }
    }

    // --- Inheritance (extends and implements) ---
    if let Some(query) = inheritance_query(language, is_tsx) {
        let class_name_idx = query.capture_index_for_name("class_name");
        let extends_idx = query.capture_index_for_name("extends_name");
        let implements_idx = query.capture_index_for_name("implements_name");
        let iface_name_idx = query.capture_index_for_name("iface_name");
        let parent_iface_idx = query.capture_index_for_name("parent_iface_name");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            // Collect all captures in this match to build from_name and to_name pairs.
            let mut class_name: Option<String> = None;
            let mut extends_name: Option<(String, usize)> = None;
            let mut implements_name: Option<(String, usize)> = None;
            let mut iface_name: Option<String> = None;
            let mut parent_iface: Option<(String, usize)> = None;

            for capture in m.captures {
                let text = node_text(capture.node, source).to_owned();
                let line = capture.node.start_position().row + 1;

                if class_name_idx == Some(capture.index) {
                    class_name = Some(text);
                } else if extends_idx == Some(capture.index) {
                    extends_name = Some((text, line));
                } else if implements_idx == Some(capture.index) {
                    implements_name = Some((text, line));
                } else if iface_name_idx == Some(capture.index) {
                    iface_name = Some(text);
                } else if parent_iface_idx == Some(capture.index) {
                    parent_iface = Some((text, line));
                }
            }

            // Emit Extends relationship
            if let (Some(from), Some((to, line))) = (&class_name, &extends_name) {
                push_rel!(RelationshipInfo {
                    from_name: Some(from.clone()),
                    to_name: to.clone(),
                    kind: RelationshipKind::Extends,
                    line: *line,
                });
            }

            // Emit Implements relationship
            if let (Some(from), Some((to, line))) = (&class_name, &implements_name) {
                push_rel!(RelationshipInfo {
                    from_name: Some(from.clone()),
                    to_name: to.clone(),
                    kind: RelationshipKind::Implements,
                    line: *line,
                });
            }

            // Emit InterfaceExtends relationship
            if let (Some(from), Some((to, line))) = (&iface_name, &parent_iface) {
                push_rel!(RelationshipInfo {
                    from_name: Some(from.clone()),
                    to_name: to.clone(),
                    kind: RelationshipKind::InterfaceExtends,
                    line: *line,
                });
            }
        }
    }

    // --- Type references ---
    if let Some(query) = type_ref_query(language, is_tsx) {
        let type_ref_idx = query
            .capture_index_for_name("type_ref")
            .expect("type_ref query must have @type_ref");

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                if capture.index == type_ref_idx {
                    let text = node_text(capture.node, source);
                    let line = capture.node.start_position().row + 1;
                    push_rel!(RelationshipInfo {
                        from_name: None,
                        to_name: text.to_owned(),
                        kind: RelationshipKind::TypeReference,
                        line,
                    });
                }
            }
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

    fn parse_tsx(source: &str) -> (tree_sitter::Tree, Language) {
        let lang = language_for_extension("tsx").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        (tree, lang)
    }

    // Test 1: Direct function call extraction
    #[test]
    fn test_direct_call_extraction() {
        let src = "foo(); bar();";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let calls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Calls).collect();
        assert_eq!(calls.len(), 2, "expected 2 Calls relationships, got {}", calls.len());

        let names: Vec<&str> = calls.iter().map(|r| r.to_name.as_str()).collect();
        assert!(names.contains(&"foo"), "missing 'foo' call");
        assert!(names.contains(&"bar"), "missing 'bar' call");
        assert!(calls.iter().all(|r| r.from_name.is_none()), "from_name should be None for context-free extraction");
    }

    // Test 2: Method call extraction
    #[test]
    fn test_method_call_extraction() {
        let src = "obj.method(); this.render();";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let method_calls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::MethodCall).collect();
        assert_eq!(method_calls.len(), 2, "expected 2 MethodCall relationships");

        let names: Vec<&str> = method_calls.iter().map(|r| r.to_name.as_str()).collect();
        assert!(names.contains(&"method"), "missing 'method' call");
        assert!(names.contains(&"render"), "missing 'render' call");
    }

    // Test 3: Class extends extraction
    #[test]
    fn test_class_extends_extraction() {
        let src = "class Dog extends Animal {}";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Extends).collect();
        assert_eq!(extends.len(), 1, "expected 1 Extends relationship");
        let rel = &extends[0];
        assert_eq!(rel.from_name.as_deref(), Some("Dog"), "from_name should be 'Dog'");
        assert_eq!(rel.to_name, "Animal", "to_name should be 'Animal'");
    }

    // Test 4: Class implements extraction
    #[test]
    fn test_class_implements_extraction() {
        let src = "class UserService implements IService {}";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let impls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Implements).collect();
        assert_eq!(impls.len(), 1, "expected 1 Implements relationship");
        let rel = &impls[0];
        assert_eq!(rel.from_name.as_deref(), Some("UserService"), "from_name should be 'UserService'");
        assert_eq!(rel.to_name, "IService", "to_name should be 'IService'");
    }

    // Test 5: Interface extends extraction
    #[test]
    fn test_interface_extends_extraction() {
        let src = "interface Admin extends User {}";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let iface_extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::InterfaceExtends).collect();
        assert_eq!(iface_extends.len(), 1, "expected 1 InterfaceExtends relationship");
        let rel = &iface_extends[0];
        assert_eq!(rel.from_name.as_deref(), Some("Admin"), "from_name should be 'Admin'");
        assert_eq!(rel.to_name, "User", "to_name should be 'User'");
    }

    // Test 6: Type reference extraction
    #[test]
    fn test_type_reference_extraction() {
        let src = "const x: MyType = {};";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let type_refs: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::TypeReference).collect();
        assert_eq!(type_refs.len(), 1, "expected 1 TypeReference relationship");
        assert_eq!(type_refs[0].to_name, "MyType", "to_name should be 'MyType'");
        assert!(type_refs[0].from_name.is_none(), "from_name should be None");
    }

    // Test 7: Combined multiple relationship types
    #[test]
    fn test_combined_relationship_extraction() {
        let src = r#"
class Dog extends Animal implements IPet {
    bark() {
        console.log("Woof");
        this.move();
    }
}
interface IPet extends IAnimal {}
const owner: Person = {};
"#;
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let calls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Calls).collect();
        let method_calls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::MethodCall).collect();
        let extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Extends).collect();
        let impls: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Implements).collect();
        let iface_extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::InterfaceExtends).collect();
        let type_refs: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::TypeReference).collect();

        assert!(!calls.is_empty() || !method_calls.is_empty(), "should find some calls");
        assert_eq!(extends.len(), 1, "should find class extends Animal");
        assert_eq!(impls.len(), 1, "should find class implements IPet");
        assert_eq!(iface_extends.len(), 1, "should find interface extends IAnimal");
        assert!(!type_refs.is_empty(), "should find type reference to Person");

        let extends_rel = &extends[0];
        assert_eq!(extends_rel.from_name.as_deref(), Some("Dog"));
        assert_eq!(extends_rel.to_name, "Animal");

        let impl_rel = &impls[0];
        assert_eq!(impl_rel.from_name.as_deref(), Some("Dog"));
        assert_eq!(impl_rel.to_name, "IPet");

        let iface_rel = &iface_extends[0];
        assert_eq!(iface_rel.from_name.as_deref(), Some("IPet"));
        assert_eq!(iface_rel.to_name, "IAnimal");
    }

    // Test: Empty file produces no relationships
    #[test]
    fn test_empty_file_no_relationships() {
        let src = "";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);
        assert!(rels.is_empty(), "empty file should produce no relationships");
    }

    // Test: File with no relationship-forming constructs
    #[test]
    fn test_no_relationships_in_plain_file() {
        let src = "const x = 42;\nconst y = 'hello';";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);
        // There should be no calls, extends, implements, or type refs
        let significant: Vec<_> = rels.iter().filter(|r| {
            matches!(r.kind, RelationshipKind::Extends | RelationshipKind::Implements | RelationshipKind::InterfaceExtends)
        }).collect();
        assert!(significant.is_empty(), "plain variable declarations should not produce inheritance relationships");
    }

    // Test: Deduplication — same call on same line does not produce duplicates
    #[test]
    fn test_deduplication() {
        // This tree-sitter query may match a call twice if two query patterns fire
        // (though with the current query design they shouldn't).
        // We verify the deduplication logic works for explicit duplicate inputs.
        let src = "foo();";
        let (tree, lang) = parse_ts(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);
        let foo_calls: Vec<_> = rels.iter().filter(|r| r.to_name == "foo" && r.kind == RelationshipKind::Calls).collect();
        assert_eq!(foo_calls.len(), 1, "foo() on one line should produce exactly 1 Calls entry");
    }

    // Test: TSX processing works correctly (no contamination from TS statics)
    #[test]
    fn test_tsx_relationships() {
        let src = "class Button extends Component { render() { this.setState(); } }";
        let (tree, lang) = parse_tsx(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, true);

        let extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Extends).collect();
        assert!(!extends.is_empty(), "TSX should find class extends relationship");
        assert_eq!(extends[0].to_name, "Component");
    }

    // Test: JavaScript class extends (JS grammar supports class extends but not implements)
    #[test]
    fn test_js_class_extends() {
        let src = "class Foo extends Bar {}";
        let (tree, lang) = parse_js(src);
        let rels = extract_relationships(&tree, src.as_bytes(), &lang, false);

        let extends: Vec<_> = rels.iter().filter(|r| r.kind == RelationshipKind::Extends).collect();
        assert_eq!(extends.len(), 1, "JS class extends should be extracted");
        assert_eq!(extends[0].from_name.as_deref(), Some("Foo"));
        assert_eq!(extends[0].to_name, "Bar");
    }
}

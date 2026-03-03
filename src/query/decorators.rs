use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::node::SymbolKind;

// ---------------------------------------------------------------------------
// DecoratorMatch — result type
// ---------------------------------------------------------------------------

/// A symbol that has a decorator matching the query pattern.
#[derive(Debug, Clone)]
pub struct DecoratorMatch {
    pub symbol_name: String,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub line: usize,
    /// End line of the decorated symbol — populated for API completeness; not yet
    /// consumed by the current text formatter but available for rich MCP clients.
    #[allow(dead_code)]
    pub line_end: usize,
    pub decorator_name: String,
    pub decorator_args: Option<String>,
    pub framework: Option<String>,
    /// Source language of the decorated symbol — available for filtering extensions.
    #[allow(dead_code)]
    pub language: String,
}

// ---------------------------------------------------------------------------
// Framework registry
// ---------------------------------------------------------------------------

struct FrameworkEntry {
    name: &'static str,
    languages: &'static [&'static str],
}

static FRAMEWORK_REGISTRY: OnceLock<HashMap<&'static str, Vec<FrameworkEntry>>> = OnceLock::new();

fn framework_registry() -> &'static HashMap<&'static str, Vec<FrameworkEntry>> {
    FRAMEWORK_REGISTRY.get_or_init(|| {
        let mut m: HashMap<&str, Vec<FrameworkEntry>> = HashMap::new();

        let mut add =
            |decorator: &'static str, framework: &'static str, langs: &'static [&'static str]| {
                m.entry(decorator).or_default().push(FrameworkEntry {
                    name: framework,
                    languages: langs,
                });
            };

        // NestJS — TypeScript/JavaScript only
        let ts_js: &[&str] = &["typescript", "tsx", "javascript"];
        add("Controller", "nestjs", ts_js);
        add("Injectable", "nestjs", ts_js);
        add("Module", "nestjs", ts_js);
        add("Get", "nestjs", ts_js);
        add("Post", "nestjs", ts_js);
        add("Put", "nestjs", ts_js);
        add("Delete", "nestjs", ts_js);
        add("Patch", "nestjs", ts_js);
        add("Guard", "nestjs", ts_js);
        add("UseGuards", "nestjs", ts_js);
        add("Interceptor", "nestjs", ts_js);
        add("UseInterceptors", "nestjs", ts_js);
        add("Pipe", "nestjs", ts_js);
        add("UsePipes", "nestjs", ts_js);
        add("Middleware", "nestjs", ts_js);
        add("Body", "nestjs", ts_js);
        add("Param", "nestjs", ts_js);
        add("Query", "nestjs", ts_js);
        add("Headers", "nestjs", ts_js);
        add("Inject", "nestjs", ts_js);

        // Angular — TypeScript only
        add("Component", "angular", ts_js);
        add("NgModule", "angular", ts_js);
        add("Directive", "angular", ts_js);
        add("Input", "angular", ts_js);
        add("Output", "angular", ts_js);
        add("ViewChild", "angular", ts_js);
        add("HostListener", "angular", ts_js);

        // Python — Flask
        let py: &[&str] = &["python"];
        add("app.route", "flask", py);
        add("bp.route", "flask", py);
        add("app.before_request", "flask", py);
        add("app.after_request", "flask", py);

        // Python — FastAPI
        add("router.get", "fastapi", py);
        add("router.post", "fastapi", py);
        add("router.put", "fastapi", py);
        add("router.delete", "fastapi", py);
        add("router.patch", "fastapi", py);
        add("app.get", "fastapi", py);
        add("app.post", "fastapi", py);
        add("app.put", "fastapi", py);
        add("app.delete", "fastapi", py);

        // Python — Django
        add("login_required", "django", py);
        add("permission_required", "django", py);
        add("csrf_exempt", "django", py);
        add("require_http_methods", "django", py);

        // Python — pytest
        add("pytest.fixture", "pytest", py);
        add("pytest.mark.parametrize", "pytest", py);
        add("pytest.mark.skip", "pytest", py);
        add("pytest.mark.skipif", "pytest", py);

        // Rust — Actix
        let rs: &[&str] = &["rust"];
        add("get", "actix", rs);
        add("post", "actix", rs);
        add("put", "actix", rs);
        add("delete", "actix", rs);
        add("patch", "actix", rs);
        add("head", "actix", rs);

        // Rust — Rocket
        add("rocket::get", "rocket", rs);
        add("rocket::post", "rocket", rs);

        // Rust — std / derive macros
        add("derive", "std", rs);
        add("cfg", "std", rs);
        add("test", "std", rs);
        add("tokio::test", "tokio", rs);
        add("async_trait", "async-trait", rs);

        // Go — struct tag keys
        let go: &[&str] = &["go"];
        add("json", "encoding/json", go);
        add("xml", "encoding/xml", go);
        add("yaml", "gopkg.in/yaml", go);
        add("gorm", "gorm", go);
        add("validate", "go-playground/validator", go);
        add("mapstructure", "mapstructure", go);
        add("bson", "mongo-driver", go);

        // Go — directives
        add("go:generate", "go-tools", go);
        add("go:embed", "go-embed", go);
        add("go:build", "go-build", go);

        m
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Look up the framework for a decorator name given the file's language.
///
/// Returns Some("nestjs") for `("Controller", "typescript")`, None for unknown decorators.
/// Language-aware: `"get"` in Rust → `"actix"`, but `"Get"` in TypeScript → `"nestjs"`.
pub fn lookup_framework(decorator_name: &str, file_language: &str) -> Option<&'static str> {
    let registry = framework_registry();
    if let Some(entries) = registry.get(decorator_name) {
        for entry in entries {
            if entry.languages.contains(&file_language) {
                return Some(entry.name);
            }
        }
        // Fallback: if no language-specific match, return first entry's framework.
        if let Some(first) = entries.first() {
            return Some(first.name);
        }
    }
    None
}

/// Enrich all symbols' decorators with framework labels from the static registry.
///
/// Called after graph build and before returning the graph to callers. Sets
/// `DecoratorInfo.framework` for any decorator whose name appears in the registry.
pub fn enrich_decorator_frameworks(graph: &mut CodeGraph) {
    use crate::graph::edge::EdgeKind;
    use crate::graph::node::GraphNode;
    use petgraph::Direction;

    // Collect (node_idx, file_language) for all symbol nodes that have decorators.
    let enrichments: Vec<(petgraph::stable_graph::NodeIndex, String)> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::Symbol(ref s) = graph.graph[idx] {
                if s.decorators.is_empty() {
                    return None;
                }
                // Find file language via a Contains edge from a File node.
                let lang = graph
                    .graph
                    .edges_directed(idx, Direction::Incoming)
                    .find_map(|e| {
                        if let EdgeKind::Contains = e.weight()
                            && let GraphNode::File(ref f) = graph.graph[e.source()]
                        {
                            return Some(f.language.clone());
                        }
                        None
                    })
                    // Child symbol: ChildOf → parent → Contains → file
                    .or_else(|| {
                        graph
                            .graph
                            .edges_directed(idx, Direction::Outgoing)
                            .find_map(|e| {
                                if let EdgeKind::ChildOf = e.weight() {
                                    graph
                                        .graph
                                        .edges_directed(e.target(), Direction::Incoming)
                                        .find_map(|pe| {
                                            if let EdgeKind::Contains = pe.weight()
                                                && let GraphNode::File(ref f) =
                                                    graph.graph[pe.source()]
                                            {
                                                return Some(f.language.clone());
                                            }
                                            None
                                        })
                                } else {
                                    None
                                }
                            })
                    })?;
                Some((idx, lang))
            } else {
                None
            }
        })
        .collect();

    for (idx, lang) in enrichments {
        if let GraphNode::Symbol(ref mut s) = graph.graph[idx] {
            for dec in &mut s.decorators {
                if dec.framework.is_none() {
                    dec.framework = lookup_framework(&dec.name, &lang).map(|fw| fw.to_owned());
                }
            }
        }
    }
}

/// Add `HasDecorator` self-edges for all symbols that have decorators.
///
/// Enables graph-level traversal queries ("find all symbols with this decorator").
/// Each edge is a self-loop: symbol → symbol with `EdgeKind::HasDecorator { name }`.
pub fn add_has_decorator_edges(graph: &mut CodeGraph) {
    use crate::graph::edge::EdgeKind;
    use crate::graph::node::GraphNode;

    let edges_to_add: Vec<(petgraph::stable_graph::NodeIndex, String)> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::Symbol(ref s) = graph.graph[idx] {
                if s.decorators.is_empty() {
                    return None;
                }
                Some(
                    s.decorators
                        .iter()
                        .map(move |d| (idx, d.name.clone()))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect();

    for (idx, name) in edges_to_add {
        graph
            .graph
            .add_edge(idx, idx, EdgeKind::HasDecorator { name });
    }
}

/// Find all symbols in the graph that have a decorator matching `pattern`.
///
/// - `pattern`: regex pattern matched against decorator names (case-insensitive).
/// - `language_filter`: optional language string ("typescript", "ts", "rust", "python", "go").
/// - `framework_filter`: optional framework name ("nestjs", "fastapi", etc.).
/// - `limit`: maximum number of results to return.
///
/// Returns `Err` if `pattern` is not a valid regex.
pub fn find_by_decorator(
    graph: &CodeGraph,
    pattern: &str,
    language_filter: Option<&str>,
    framework_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<DecoratorMatch>> {
    use crate::graph::edge::EdgeKind;
    use crate::graph::node::GraphNode;
    use petgraph::Direction;
    use regex::RegexBuilder;

    let re = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| anyhow::anyhow!("invalid regex: {}", e))?;

    let mut results = Vec::new();

    for idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(ref sym) = graph.graph[idx] {
            if sym.decorators.is_empty() {
                continue;
            }

            // Find containing file for language / path info.
            let file_info = graph
                .graph
                .edges_directed(idx, Direction::Incoming)
                .find_map(|e| {
                    if let EdgeKind::Contains = e.weight()
                        && let GraphNode::File(ref f) = graph.graph[e.source()]
                    {
                        return Some(f.clone());
                    }
                    None
                })
                .or_else(|| {
                    // Child symbol: ChildOf → parent → Contains → file
                    graph
                        .graph
                        .edges_directed(idx, Direction::Outgoing)
                        .find_map(|e| {
                            if let EdgeKind::ChildOf = e.weight() {
                                graph
                                    .graph
                                    .edges_directed(e.target(), Direction::Incoming)
                                    .find_map(|pe| {
                                        if let EdgeKind::Contains = pe.weight()
                                            && let GraphNode::File(ref f) = graph.graph[pe.source()]
                                        {
                                            return Some(f.clone());
                                        }
                                        None
                                    })
                            } else {
                                None
                            }
                        })
                });

            let file_info = match file_info {
                Some(fi) => fi,
                None => continue, // orphaned symbol — skip
            };

            // Language filter
            if let Some(lang) = language_filter {
                let file_lang = &file_info.language;
                let matches = match lang {
                    "ts" | "typescript" => file_lang == "typescript" || file_lang == "tsx",
                    "js" | "javascript" => file_lang == "javascript",
                    "rust" | "rs" => file_lang == "rust",
                    "python" | "py" => file_lang == "python",
                    "go" | "golang" => file_lang == "go",
                    _ => file_lang.as_str() == lang,
                };
                if !matches {
                    continue;
                }
            }

            for decorator in &sym.decorators {
                // Skip internal sentinel decorators
                if decorator.name.starts_with("__") {
                    continue;
                }

                if !re.is_match(&decorator.name) {
                    continue;
                }

                // Framework filter
                if let Some(fw) = framework_filter
                    && decorator.framework.as_deref() != Some(fw)
                {
                    continue;
                }

                results.push(DecoratorMatch {
                    symbol_name: sym.name.clone(),
                    kind: sym.kind.clone(),
                    file_path: file_info.path.clone(),
                    line: sym.line,
                    line_end: sym.line_end,
                    decorator_name: decorator.name.clone(),
                    decorator_args: decorator.args_raw.clone(),
                    framework: decorator.framework.clone(),
                    language: file_info.language.clone(),
                });

                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{DecoratorInfo, SymbolInfo, SymbolKind};
    use std::path::PathBuf;

    fn make_graph_with_decorated_symbol(
        lang: &str,
        ext: &str,
        decorator_name: &str,
        decorator_args: Option<&str>,
    ) -> (CodeGraph, petgraph::stable_graph::NodeIndex) {
        let mut graph = CodeGraph::new();
        let path = PathBuf::from(format!("src/example.{}", ext));
        let file_idx = graph.add_file(path, lang);
        let sym_idx = graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "MyController".into(),
                kind: SymbolKind::Class,
                line: 5,
                line_end: 20,
                is_exported: true,
                decorators: vec![DecoratorInfo {
                    name: decorator_name.to_owned(),
                    object: None,
                    attribute: None,
                    args_raw: decorator_args.map(|s| s.to_owned()),
                    framework: None,
                }],
                ..Default::default()
            },
        );
        (graph, sym_idx)
    }

    // ---- Framework registry tests ----

    #[test]
    fn test_framework_registry_nestjs_ts() {
        assert_eq!(lookup_framework("Controller", "typescript"), Some("nestjs"));
    }

    #[test]
    fn test_framework_registry_actix_rust() {
        assert_eq!(lookup_framework("get", "rust"), Some("actix"));
    }

    #[test]
    fn test_framework_registry_flask_python() {
        assert_eq!(lookup_framework("app.route", "python"), Some("flask"));
    }

    #[test]
    fn test_framework_registry_unknown() {
        assert_eq!(lookup_framework("unknown_decorator", "typescript"), None);
    }

    #[test]
    fn test_framework_disambiguation() {
        // "Get" is NestJS in TypeScript, but "get" (lowercase) in Rust is actix
        assert_eq!(lookup_framework("Get", "typescript"), Some("nestjs"));
        assert_eq!(lookup_framework("get", "rust"), Some("actix"));
    }

    #[test]
    fn test_go_struct_tag_framework() {
        assert_eq!(lookup_framework("json", "go"), Some("encoding/json"));
    }

    // ---- find_by_decorator tests ----

    #[test]
    fn test_find_by_decorator_basic() {
        let (graph, _) = make_graph_with_decorated_symbol("typescript", "ts", "Controller", None);
        let results = find_by_decorator(&graph, "Controller", None, None, 50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, "MyController");
        assert_eq!(results[0].decorator_name, "Controller");
    }

    #[test]
    fn test_find_by_decorator_regex() {
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(PathBuf::from("src/app.ts"), "typescript");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "UserController".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 10,
                decorators: vec![DecoratorInfo {
                    name: "Controller".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "UserService".into(),
                kind: SymbolKind::Class,
                line: 20,
                line_end: 30,
                decorators: vec![DecoratorInfo {
                    name: "Injectable".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        let results = find_by_decorator(&graph, "Controller|Injectable", None, None, 50).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_find_by_decorator_language_filter() {
        let mut graph = CodeGraph::new();
        let ts_file = graph.add_file(PathBuf::from("src/app.ts"), "typescript");
        let py_file = graph.add_file(PathBuf::from("src/app.py"), "python");

        graph.add_symbol(
            ts_file,
            SymbolInfo {
                name: "TSClass".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 5,
                decorators: vec![DecoratorInfo {
                    name: "Controller".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        graph.add_symbol(
            py_file,
            SymbolInfo {
                name: "PyClass".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 5,
                decorators: vec![DecoratorInfo {
                    name: "Controller".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        // Only Python results
        let py_results = find_by_decorator(&graph, "Controller", Some("python"), None, 50).unwrap();
        assert_eq!(py_results.len(), 1);
        assert_eq!(py_results[0].symbol_name, "PyClass");

        // Only TypeScript results
        let ts_results =
            find_by_decorator(&graph, "Controller", Some("typescript"), None, 50).unwrap();
        assert_eq!(ts_results.len(), 1);
        assert_eq!(ts_results[0].symbol_name, "TSClass");
    }

    #[test]
    fn test_find_by_decorator_framework_filter() {
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(PathBuf::from("src/app.ts"), "typescript");

        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "AppController".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 10,
                decorators: vec![DecoratorInfo {
                    name: "Controller".into(),
                    framework: Some("nestjs".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "AppComponent".into(),
                kind: SymbolKind::Class,
                line: 20,
                line_end: 30,
                decorators: vec![DecoratorInfo {
                    name: "Component".into(),
                    framework: Some("angular".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        let nestjs_results = find_by_decorator(&graph, ".*", None, Some("nestjs"), 50).unwrap();
        assert_eq!(nestjs_results.len(), 1);
        assert_eq!(nestjs_results[0].symbol_name, "AppController");

        let angular_results = find_by_decorator(&graph, ".*", None, Some("angular"), 50).unwrap();
        assert_eq!(angular_results.len(), 1);
        assert_eq!(angular_results[0].symbol_name, "AppComponent");
    }

    #[test]
    fn test_find_by_decorator_limit() {
        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(PathBuf::from("src/app.ts"), "typescript");

        for i in 0..10 {
            graph.add_symbol(
                file_idx,
                SymbolInfo {
                    name: format!("Class{}", i),
                    kind: SymbolKind::Class,
                    line: i + 1,
                    line_end: i + 2,
                    decorators: vec![DecoratorInfo {
                        name: "Injectable".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
        }

        let results = find_by_decorator(&graph, "Injectable", None, None, 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_enrich_decorator_frameworks() {
        let (mut graph, _sym_idx) =
            make_graph_with_decorated_symbol("typescript", "ts", "Controller", None);

        enrich_decorator_frameworks(&mut graph);

        // After enrichment, the decorator should have framework "nestjs"
        use crate::graph::node::GraphNode;
        for idx in graph.graph.node_indices() {
            if let GraphNode::Symbol(ref s) = graph.graph[idx]
                && s.name == "MyController"
            {
                assert!(!s.decorators.is_empty());
                assert_eq!(
                    s.decorators[0].framework.as_deref(),
                    Some("nestjs"),
                    "Controller in TypeScript should have framework 'nestjs'"
                );
            }
        }
    }

    #[test]
    fn test_add_has_decorator_edges() {
        let (mut graph, sym_idx) =
            make_graph_with_decorated_symbol("typescript", "ts", "Controller", None);

        add_has_decorator_edges(&mut graph);

        use crate::graph::edge::EdgeKind;
        // There should be a self-loop on sym_idx
        let has_self_loop = graph.graph.edges(sym_idx).any(|e| {
            e.target() == sym_idx
                && matches!(e.weight(), EdgeKind::HasDecorator { name } if name == "Controller")
        });
        assert!(
            has_self_loop,
            "HasDecorator self-edge should exist on the symbol"
        );
    }

    #[test]
    fn test_find_by_decorator_cross_language() {
        let mut graph = CodeGraph::new();
        let ts_file = graph.add_file(PathBuf::from("src/app.ts"), "typescript");
        let py_file = graph.add_file(PathBuf::from("src/app.py"), "python");

        graph.add_symbol(
            ts_file,
            SymbolInfo {
                name: "TSClass".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 5,
                decorators: vec![DecoratorInfo {
                    name: "Controller".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        graph.add_symbol(
            py_file,
            SymbolInfo {
                name: "PyView".into(),
                kind: SymbolKind::Class,
                line: 1,
                line_end: 5,
                decorators: vec![DecoratorInfo {
                    name: "login_required".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );

        // No language filter → results from both languages
        let results = find_by_decorator(&graph, ".*", None, None, 50).unwrap();
        assert_eq!(results.len(), 2);
        let names: Vec<_> = results.iter().map(|r| r.symbol_name.as_str()).collect();
        assert!(names.contains(&"TSClass"));
        assert!(names.contains(&"PyView"));
    }

    #[test]
    fn test_find_by_decorator_go_struct_tag_framework() {
        // Go is supported in the registry
        assert_eq!(lookup_framework("json", "go"), Some("encoding/json"));
        assert_eq!(lookup_framework("gorm", "go"), Some("gorm"));
        assert_eq!(
            lookup_framework("validate", "go"),
            Some("go-playground/validator")
        );
    }
}

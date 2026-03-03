/// Go import resolver — Step 8 of `resolve_all`.
///
/// Handles:
/// - go.mod parsing (module path + replace directives)
/// - go.work parsing (multi-module workspace support)
/// - stdlib detection via no-dot-in-first-segment heuristic
/// - vendor/ directory priority
/// - Local import resolution via module_map
/// - Blank import (import _ "pkg") -> SideEffectImport edges
/// - Dot import (import . "pkg") -> DotImport edges
/// - Method receiver -> ChildOf edges (LANG-07)
/// - Struct embedding -> Embeds edges
/// - Implicit interface satisfaction -> Implements edges (with 1000-type cap)
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::{GraphNode, SymbolKind};
use crate::parser::ParseResult;
use crate::parser::imports::ImportKind;

/// Statistics from Go import resolution.
#[derive(Debug, Default)]
pub struct GoResolveStats {
    pub resolved: usize,
    pub stdlib: usize,
    pub external: usize,
    pub unresolved: usize,
    pub method_edges: usize,
    pub embed_edges: usize,
    pub implements_edges: usize,
}

// ---------------------------------------------------------------------------
// go.mod parser
// ---------------------------------------------------------------------------

struct GoMod {
    module_path: String,
    replaces: Vec<(String, String)>, // (old_path, new_path_or_local)
}

fn parse_gomod(content: &str) -> Option<GoMod> {
    let mut module_path = None;
    let mut replaces = Vec::new();
    let mut in_replace_block = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(rest) = line.strip_prefix("module ") {
            module_path = Some(rest.trim().to_owned());
        } else if line == "replace (" {
            in_replace_block = true;
        } else if in_replace_block && line == ")" {
            in_replace_block = false;
        } else if in_replace_block || line.starts_with("replace ") {
            let replace_line = if let Some(rest) = line.strip_prefix("replace ") {
                rest
            } else {
                line
            };
            if let Some(arrow_pos) = replace_line.find("=>") {
                let old = replace_line[..arrow_pos]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let new = replace_line[arrow_pos + 2..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                if !old.is_empty() && !new.is_empty() {
                    replaces.push((old, new));
                }
            }
        }
    }

    module_path.map(|mp| GoMod {
        module_path: mp,
        replaces,
    })
}

// ---------------------------------------------------------------------------
// go.work parser
// ---------------------------------------------------------------------------

struct GoWork {
    use_dirs: Vec<String>,
}

fn parse_gowork(content: &str) -> GoWork {
    let mut use_dirs = Vec::new();
    let mut in_use_block = false;

    for line in content.lines() {
        let line = line.trim();
        if line == "use (" {
            in_use_block = true;
        } else if in_use_block && line == ")" {
            in_use_block = false;
        } else if in_use_block {
            let dir = line.trim();
            if !dir.is_empty() && !dir.starts_with("//") {
                use_dirs.push(dir.to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("use ")
            && !rest.contains('(')
        {
            let dir = rest.trim().to_owned();
            if !dir.is_empty() {
                use_dirs.push(dir);
            }
        }
    }

    GoWork { use_dirs }
}

// ---------------------------------------------------------------------------
// Stdlib detection
// ---------------------------------------------------------------------------

/// Returns true if `import_path` is a Go standard library package.
///
/// Heuristic: if the first path segment contains no dot, it's stdlib.
/// e.g. "fmt" -> true, "net/http" -> true, "github.com/pkg/errors" -> false.
fn is_go_stdlib(import_path: &str) -> bool {
    let first_segment = import_path.split('/').next().unwrap_or("");
    !first_segment.contains('.') && !first_segment.is_empty()
}

// ---------------------------------------------------------------------------
// Local resolution
// ---------------------------------------------------------------------------

fn resolve_go_import_path(
    import_path: &str,
    module_map: &HashMap<String, PathBuf>,
    project_root: &Path,
    has_vendor: bool,
) -> Option<PathBuf> {
    // Check vendor first if present
    if has_vendor {
        let vendor_path = project_root.join("vendor").join(import_path);
        if vendor_path.is_dir() {
            return Some(vendor_path);
        }
    }

    // Try matching against module paths (longest prefix match)
    let mut best_match: Option<(&str, &PathBuf)> = None;
    for (mod_path, mod_dir) in module_map {
        if import_path.starts_with(mod_path.as_str()) {
            let remainder = &import_path[mod_path.len()..];
            if (remainder.is_empty() || remainder.starts_with('/'))
                && (best_match.is_none() || mod_path.len() > best_match.unwrap().0.len())
            {
                best_match = Some((mod_path.as_str(), mod_dir));
            }
        }
    }

    if let Some((mod_path, mod_dir)) = best_match {
        let remainder = &import_path[mod_path.len()..];
        let sub_path = remainder.trim_start_matches('/');
        let target_dir = if sub_path.is_empty() {
            mod_dir.clone()
        } else {
            mod_dir.join(sub_path)
        };
        if target_dir.is_dir() {
            return Some(target_dir);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Method receiver -> ChildOf edge wiring (LANG-07)
// ---------------------------------------------------------------------------

fn wire_method_receiver_edges(graph: &mut CodeGraph, stats: &mut GoResolveStats) {
    // Collect (method_idx, receiver_type_name) for Go methods
    let method_infos: Vec<(NodeIndex, String)> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::Symbol(ref s) = graph.graph[idx]
                && s.kind == SymbolKind::Method
                && let Some(ref receiver_name) = s.trait_impl
            {
                // Check this is a Go file symbol (not Rust impl method)
                let in_go_file = graph
                    .graph
                    .edges_directed(idx, Direction::Incoming)
                    .any(|e| {
                        if let EdgeKind::Contains = e.weight()
                            && let GraphNode::File(ref f) = graph.graph[e.source()]
                        {
                            return f.language == "go";
                        }
                        false
                    });
                if in_go_file {
                    return Some((idx, receiver_name.clone()));
                }
            }
            None
        })
        .collect();

    for (method_idx, receiver_name) in method_infos {
        // Find the struct symbol with matching name in the same file or package
        if let Some(struct_indices) = graph.symbol_index.get(&receiver_name).cloned() {
            // Find the file containing this method
            let containing_file = graph
                .graph
                .edges_directed(method_idx, Direction::Incoming)
                .find_map(|e| {
                    if let EdgeKind::Contains = e.weight() {
                        Some(e.source())
                    } else {
                        None
                    }
                });

            let mut edge_added = false;
            for &struct_idx in &struct_indices {
                if edge_added {
                    break;
                }
                let struct_file = graph
                    .graph
                    .edges_directed(struct_idx, Direction::Incoming)
                    .find_map(|e| {
                        if let EdgeKind::Contains = e.weight() {
                            Some(e.source())
                        } else {
                            None
                        }
                    });

                // Same file or same Go package directory
                let same_file = containing_file == struct_file;
                let same_package = containing_file.is_some() && struct_file.is_some() && {
                    let cf = containing_file.unwrap();
                    let sf = struct_file.unwrap();
                    if let (GraphNode::File(f1), GraphNode::File(f2)) =
                        (&graph.graph[cf], &graph.graph[sf])
                    {
                        f1.path.parent() == f2.path.parent()
                            && f1.language == "go"
                            && f2.language == "go"
                    } else {
                        false
                    }
                };

                if same_file || same_package {
                    graph
                        .graph
                        .add_edge(method_idx, struct_idx, EdgeKind::ChildOf);
                    stats.method_edges += 1;
                    edge_added = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Struct embed -> Embeds edge wiring
// ---------------------------------------------------------------------------

fn wire_embed_edges(graph: &mut CodeGraph, stats: &mut GoResolveStats) {
    // Find structs with __embedded__ decorator (sentinel from Plan 01)
    let embed_infos: Vec<(NodeIndex, Vec<String>)> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::Symbol(ref s) = graph.graph[idx]
                && s.kind == SymbolKind::Struct
            {
                let embedded: Vec<String> = s
                    .decorators
                    .iter()
                    .filter(|d| d.name == "__embedded__")
                    .filter_map(|d| d.args_raw.clone())
                    .flat_map(|args| {
                        args.split(',')
                            .map(|s| s.trim().to_owned())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if !embedded.is_empty() {
                    return Some((idx, embedded));
                }
            }
            None
        })
        .collect();

    for (struct_idx, embedded_types) in embed_infos {
        for type_name in embedded_types {
            // Resolve the embedded type name (may be qualified: "http.Handler" -> "Handler")
            let simple_name = type_name.split('.').next_back().unwrap_or(&type_name);
            if let Some(target_indices) = graph.symbol_index.get(simple_name).cloned() {
                for &target_idx in &target_indices {
                    if target_idx != struct_idx {
                        graph
                            .graph
                            .add_edge(struct_idx, target_idx, EdgeKind::Embeds);
                        stats.embed_edges += 1;
                        break; // First match
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Implicit interface satisfaction -> Implements edge wiring
// ---------------------------------------------------------------------------

fn wire_implicit_interfaces(graph: &mut CodeGraph, stats: &mut GoResolveStats, verbose: bool) {
    // Only run for Go files. Cap at 1000 types to avoid O(n^2) on large generated codebases.
    let go_file_indices: HashSet<NodeIndex> = graph
        .graph
        .node_indices()
        .filter(|&idx| {
            if let GraphNode::File(ref f) = graph.graph[idx] {
                f.language == "go"
            } else {
                false
            }
        })
        .collect();

    if go_file_indices.is_empty() {
        return;
    }

    // Collect interfaces with their method sets
    let mut interface_methods: HashMap<NodeIndex, HashSet<String>> = HashMap::new();
    // Collect structs with their method sets
    let mut struct_methods: HashMap<NodeIndex, HashSet<String>> = HashMap::new();

    for idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(ref s) = graph.graph[idx] {
            // Check if in Go file
            let in_go = graph
                .graph
                .edges_directed(idx, Direction::Incoming)
                .any(|e| {
                    if let EdgeKind::Contains = e.weight() {
                        go_file_indices.contains(&e.source())
                    } else {
                        false
                    }
                });
            if !in_go {
                continue;
            }

            match s.kind {
                SymbolKind::Interface => {
                    // Collect method names from ChildOf edges (interface methods point TO the interface)
                    let methods: HashSet<String> = graph
                        .graph
                        .edges_directed(idx, Direction::Incoming)
                        .filter_map(|e| {
                            if let EdgeKind::ChildOf = e.weight()
                                && let GraphNode::Symbol(ref cs) = graph.graph[e.source()]
                            {
                                return Some(cs.name.clone());
                            }
                            None
                        })
                        .collect();
                    if !methods.is_empty() {
                        interface_methods.insert(idx, methods);
                    }
                }
                SymbolKind::Struct => {
                    // Collect method names from ChildOf edges wired by wire_method_receiver_edges
                    let methods: HashSet<String> = graph
                        .graph
                        .edges_directed(idx, Direction::Incoming)
                        .filter_map(|e| {
                            if let EdgeKind::ChildOf = e.weight()
                                && let GraphNode::Symbol(ref cs) = graph.graph[e.source()]
                                && cs.kind == SymbolKind::Method
                            {
                                return Some(cs.name.clone());
                            }
                            None
                        })
                        .collect();
                    struct_methods.insert(idx, methods);
                }
                _ => {}
            }
        }
    }

    // Performance cap: if too many types, skip implicit interface satisfaction
    let total_types = interface_methods.len() + struct_methods.len();
    if total_types > 1000 {
        if verbose {
            eprintln!(
                "  Go implicit interfaces: skipping ({} types exceeds 1000 cap)",
                total_types
            );
        }
        return;
    }

    // For each struct, check if its method set satisfies any interface
    let iface_list: Vec<(NodeIndex, HashSet<String>)> = interface_methods.into_iter().collect();
    for (&struct_idx, struct_meths) in &struct_methods {
        for (iface_idx, iface_meths) in &iface_list {
            if iface_meths.is_subset(struct_meths) {
                graph.add_implements_edge(struct_idx, *iface_idx);
                stats.implements_edges += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Resolve Go imports and wire structural edges for all Go files in the graph.
///
/// Called as Step 8 in `resolve_all` when Go files are detected.
pub fn resolve_go_imports(
    graph: &mut CodeGraph,
    parse_results: &HashMap<PathBuf, ParseResult>,
    project_root: &Path,
    verbose: bool,
) -> GoResolveStats {
    let mut stats = GoResolveStats::default();

    // 1. Discover go.mod files and build module path -> local directory mapping
    let mut module_map: HashMap<String, PathBuf> = HashMap::new();

    // Check for go.work first (multi-module workspace)
    let gowork_path = project_root.join("go.work");
    if gowork_path.exists()
        && let Ok(content) = std::fs::read_to_string(&gowork_path)
    {
        let gowork = parse_gowork(&content);
        for use_dir in &gowork.use_dirs {
            let mod_dir = project_root.join(use_dir);
            let gomod_path = mod_dir.join("go.mod");
            if let Ok(gomod_content) = std::fs::read_to_string(&gomod_path)
                && let Some(gomod) = parse_gomod(&gomod_content)
            {
                module_map.insert(gomod.module_path, mod_dir.clone());
                for (old, new) in &gomod.replaces {
                    if new.starts_with('.') || new.starts_with('/') {
                        let new_path = mod_dir.join(new);
                        module_map.insert(old.clone(), new_path);
                    }
                    // External replace (not a local path) -- skip
                }
            }
        }
    }

    // Also check top-level go.mod
    let gomod_path = project_root.join("go.mod");
    if gomod_path.exists()
        && let Ok(content) = std::fs::read_to_string(&gomod_path)
        && let Some(gomod) = parse_gomod(&content)
    {
        module_map
            .entry(gomod.module_path)
            .or_insert_with(|| project_root.to_owned());
        for (old, new) in &gomod.replaces {
            if new.starts_with('.') || new.starts_with('/') {
                let new_path = project_root.join(new);
                module_map.insert(old.clone(), new_path);
            }
        }
    }

    // 2. Check for vendor/ directory
    let has_vendor = project_root.join("vendor").is_dir();

    // 3. Collect Go file imports (avoid borrow conflict with graph)
    let go_file_imports: Vec<(PathBuf, Vec<crate::parser::imports::ImportInfo>)> = parse_results
        .iter()
        .filter(|(path, _)| path.extension().and_then(|e| e.to_str()) == Some("go"))
        .map(|(path, result)| (path.clone(), result.imports.clone()))
        .collect();

    // 4. Resolve Go imports
    for (file_path, imports) in &go_file_imports {
        let from_idx = match graph.file_index.get(file_path).copied() {
            Some(idx) => idx,
            None => continue,
        };

        for import in imports {
            let import_path = &import.module_path;

            // Stdlib detection
            if is_go_stdlib(import_path) {
                graph.add_external_package(from_idx, import_path, import_path);
                stats.stdlib += 1;
                continue;
            }

            // Try local resolution via module_map
            let resolved =
                resolve_go_import_path(import_path, &module_map, project_root, has_vendor);

            match resolved {
                Some(target_dir) => {
                    // Find .go files in this directory that are in the graph
                    let target_files: Vec<_> = graph
                        .file_index
                        .iter()
                        .filter(|(p, _)| {
                            p.parent() == Some(target_dir.as_path())
                                && p.extension().and_then(|e| e.to_str()) == Some("go")
                        })
                        .map(|(_, &idx)| idx)
                        .collect();

                    if target_files.is_empty() {
                        // Directory exists but no Go files indexed -- treat as external
                        graph.add_external_package(from_idx, import_path, import_path);
                        stats.external += 1;
                    } else {
                        // Create edge to each Go file in the target package directory
                        for target_idx in target_files {
                            match import.kind {
                                ImportKind::GoBlank => {
                                    graph.graph.add_edge(
                                        from_idx,
                                        target_idx,
                                        EdgeKind::SideEffectImport {
                                            specifier: import_path.clone(),
                                        },
                                    );
                                }
                                ImportKind::GoDot => {
                                    graph.graph.add_edge(
                                        from_idx,
                                        target_idx,
                                        EdgeKind::DotImport {
                                            specifier: import_path.clone(),
                                        },
                                    );
                                }
                                _ => {
                                    graph.add_resolved_import(from_idx, target_idx, import_path);
                                }
                            }
                        }
                        stats.resolved += 1;
                    }
                }
                None => {
                    // External package (has a domain but not in our module_map)
                    let pkg_name = import_path.split('/').take(3).collect::<Vec<_>>().join("/");
                    graph.add_external_package(from_idx, &pkg_name, import_path);
                    stats.external += 1;
                }
            }
        }
    }

    // 5. Method receiver -> ChildOf edges (LANG-07)
    wire_method_receiver_edges(graph, &mut stats);

    // 6. Embed edges
    wire_embed_edges(graph, &mut stats);

    // 7. Implicit interface satisfaction
    wire_implicit_interfaces(graph, &mut stats, verbose);

    stats
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::graph::CodeGraph;
    use crate::graph::node::{DecoratorInfo, SymbolInfo, SymbolKind, SymbolVisibility};
    use crate::parser::ParseResult;
    use crate::parser::imports::{ImportInfo, ImportKind};

    // -----------------------------------------------------------------------
    // go.mod parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_gomod() {
        let content = r#"
module github.com/test/project

go 1.21

require github.com/pkg/errors v0.9.1

replace github.com/old/pkg => ./local/pkg
replace (
    github.com/another/old => ../sibling
)
"#;
        let gomod = parse_gomod(content).expect("should parse");
        assert_eq!(gomod.module_path, "github.com/test/project");
        assert_eq!(gomod.replaces.len(), 2);
        assert_eq!(gomod.replaces[0].0, "github.com/old/pkg");
        assert_eq!(gomod.replaces[0].1, "./local/pkg");
        assert_eq!(gomod.replaces[1].0, "github.com/another/old");
        assert_eq!(gomod.replaces[1].1, "../sibling");
    }

    #[test]
    fn test_parse_gomod_simple() {
        let content = "module example.com/myapp\n\ngo 1.21\n";
        let gomod = parse_gomod(content).expect("should parse");
        assert_eq!(gomod.module_path, "example.com/myapp");
        assert!(gomod.replaces.is_empty());
    }

    #[test]
    fn test_parse_gomod_missing_module() {
        let content = "go 1.21\n";
        assert!(parse_gomod(content).is_none());
    }

    // -----------------------------------------------------------------------
    // go.work parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_gowork() {
        let content = r#"
go 1.21

use (
    ./module-a
    ./module-b
)

use ./module-c
"#;
        let gowork = parse_gowork(content);
        assert_eq!(
            gowork.use_dirs,
            vec!["./module-a", "./module-b", "./module-c"]
        );
    }

    #[test]
    fn test_parse_gowork_empty() {
        let content = "go 1.21\n";
        let gowork = parse_gowork(content);
        assert!(gowork.use_dirs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Stdlib detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_go_stdlib() {
        assert!(is_go_stdlib("fmt"));
        assert!(is_go_stdlib("os"));
        assert!(is_go_stdlib("net/http"));
        assert!(is_go_stdlib("encoding/json"));
        assert!(!is_go_stdlib("github.com/pkg/errors"));
        assert!(!is_go_stdlib("golang.org/x/tools"));
        assert!(!is_go_stdlib("example.com/myapp/pkg"));
    }

    // -----------------------------------------------------------------------
    // resolve_go_import_path
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_go_import_local() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create sub-directory simulating a local package
        let pkg_dir = root.join("handlers");
        fs::create_dir_all(&pkg_dir).unwrap();

        let mut module_map = HashMap::new();
        module_map.insert("github.com/test/project".to_string(), root.to_path_buf());

        let result =
            resolve_go_import_path("github.com/test/project/handlers", &module_map, root, false);
        assert_eq!(result, Some(pkg_dir));
    }

    #[test]
    fn test_resolve_go_import_replace() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create local replacement directory
        let local_pkg = root.join("local").join("replaced");
        fs::create_dir_all(&local_pkg).unwrap();

        let mut module_map = HashMap::new();
        module_map.insert("github.com/old/pkg".to_string(), local_pkg.clone());

        let result = resolve_go_import_path("github.com/old/pkg", &module_map, root, false);
        assert_eq!(result, Some(local_pkg));
    }

    #[test]
    fn test_resolve_go_import_not_found() {
        let module_map = HashMap::new();
        let root = std::path::Path::new("/tmp");
        let result = resolve_go_import_path("github.com/notexist/pkg", &module_map, root, false);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Stdlib imports -> external package edge
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_go_import_stdlib() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let gomod_content = "module github.com/test/project\n\ngo 1.21\n";
        std::fs::write(root.join("go.mod"), gomod_content).unwrap();

        let go_file = root.join("main.go");
        std::fs::write(&go_file, "package main\n").unwrap();

        let mut graph = CodeGraph::new();
        let _file_idx = graph.add_file(go_file.clone(), "go");

        let fmt_import = ImportInfo {
            module_path: "fmt".to_string(),
            kind: ImportKind::GoAbsolute,
            specifiers: vec![],
            line: 3,
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(
            go_file,
            ParseResult {
                symbols: vec![],
                imports: vec![fmt_import],
                exports: vec![],
                relationships: vec![],
                rust_uses: vec![],
            },
        );

        let stats = resolve_go_imports(&mut graph, &parse_results, root, false);
        assert_eq!(stats.stdlib, 1);
        assert_eq!(stats.resolved, 0);

        // Should have added ExternalPackage node for "fmt"
        let has_external = graph.graph.node_indices().any(|idx| {
            if let crate::graph::node::GraphNode::ExternalPackage(ref ep) = graph.graph[idx] {
                ep.name == "fmt"
            } else {
                false
            }
        });
        assert!(has_external, "fmt should be marked as external package");
    }

    // -----------------------------------------------------------------------
    // Method receiver -> ChildOf edge
    // -----------------------------------------------------------------------

    fn make_symbol(name: &str, kind: SymbolKind, receiver: Option<&str>) -> SymbolInfo {
        SymbolInfo {
            name: name.to_string(),
            kind,
            line: 1,
            col: 0,
            line_end: 5,
            is_exported: true,
            is_default: false,
            visibility: SymbolVisibility::Pub,
            trait_impl: receiver.map(|s| s.to_string()),
            decorators: vec![],
        }
    }

    #[test]
    fn test_method_receiver_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let go_file = root.join("handler.go");
        std::fs::write(&go_file, "package main\n").unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(go_file.clone(), "go");

        // Add Router struct
        let router_sym = make_symbol("Router", SymbolKind::Struct, None);
        let _router_idx = graph.add_symbol(file_idx, router_sym);

        // Add Handle method on *Router
        let handle_sym = make_symbol("Handle", SymbolKind::Method, Some("Router"));
        let _handle_idx = graph.add_symbol(file_idx, handle_sym);

        let parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

        // Write a minimal go.mod
        std::fs::write(root.join("go.mod"), "module example.com/test\n\ngo 1.21\n").unwrap();

        let stats = resolve_go_imports(&mut graph, &parse_results, root, false);
        assert!(
            stats.method_edges >= 1,
            "expected ChildOf edge for Handle->Router"
        );
    }

    // -----------------------------------------------------------------------
    // Struct embed -> Embeds edge
    // -----------------------------------------------------------------------

    #[test]
    fn test_embed_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let go_file = root.join("embed.go");
        std::fs::write(&go_file, "package main\n").unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(go_file.clone(), "go");

        // Add Router struct (the embedded type)
        let router_sym = make_symbol("Router", SymbolKind::Struct, None);
        let _router_idx = graph.add_symbol(file_idx, router_sym);

        // Add Server struct with __embedded__ decorator listing Router
        let mut server_sym = make_symbol("Server", SymbolKind::Struct, None);
        server_sym.decorators.push(DecoratorInfo {
            name: "__embedded__".to_string(),
            object: None,
            attribute: None,
            args_raw: Some("Router".to_string()),
            framework: None,
        });
        let _server_idx = graph.add_symbol(file_idx, server_sym);

        let parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        std::fs::write(root.join("go.mod"), "module example.com/test\n\ngo 1.21\n").unwrap();

        let stats = resolve_go_imports(&mut graph, &parse_results, root, false);
        assert!(
            stats.embed_edges >= 1,
            "expected Embeds edge for Server->Router"
        );
    }

    // -----------------------------------------------------------------------
    // Implicit interface satisfaction -> Implements edge
    // -----------------------------------------------------------------------

    #[test]
    fn test_implicit_interface_satisfaction() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let go_file = root.join("iface.go");
        std::fs::write(&go_file, "package main\n").unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(go_file.clone(), "go");

        // Add Handler interface
        let iface_sym = make_symbol("Handler", SymbolKind::Interface, None);
        let iface_idx = graph.add_symbol(file_idx, iface_sym);

        // Add Handle method as child of Handler interface
        let iface_method = make_symbol("Handle", SymbolKind::Method, None);
        graph.add_child_symbol(iface_idx, iface_method);

        // Add Router struct
        let router_sym = make_symbol("Router", SymbolKind::Struct, None);
        let router_idx = graph.add_symbol(file_idx, router_sym);

        // Add Handle method on Router (with receiver)
        let method_sym = make_symbol("Handle", SymbolKind::Method, Some("Router"));
        let method_idx = graph.add_symbol(file_idx, method_sym);

        // Manually wire ChildOf edge from method to struct (as the resolver would)
        graph
            .graph
            .add_edge(method_idx, router_idx, EdgeKind::ChildOf);

        let parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        std::fs::write(root.join("go.mod"), "module example.com/test\n\ngo 1.21\n").unwrap();

        let stats = resolve_go_imports(&mut graph, &parse_results, root, false);
        assert!(
            stats.implements_edges >= 1,
            "expected Implements edge for Router->Handler"
        );
    }

    #[test]
    fn test_implicit_interface_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let go_file = root.join("nomatch.go");
        std::fs::write(&go_file, "package main\n").unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(go_file.clone(), "go");

        // Interface with two methods
        let iface_sym = make_symbol("Handler", SymbolKind::Interface, None);
        let iface_idx = graph.add_symbol(file_idx, iface_sym);
        graph.add_child_symbol(iface_idx, make_symbol("Handle", SymbolKind::Method, None));
        graph.add_child_symbol(iface_idx, make_symbol("Name", SymbolKind::Method, None));

        // Struct with only ONE method -- does NOT satisfy the interface
        let router_sym = make_symbol("Router", SymbolKind::Struct, None);
        let router_idx = graph.add_symbol(file_idx, router_sym);
        let method_sym = make_symbol("Handle", SymbolKind::Method, Some("Router"));
        let method_idx = graph.add_symbol(file_idx, method_sym);
        graph
            .graph
            .add_edge(method_idx, router_idx, EdgeKind::ChildOf);

        let parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        std::fs::write(root.join("go.mod"), "module example.com/test\n\ngo 1.21\n").unwrap();

        let stats = resolve_go_imports(&mut graph, &parse_results, root, false);
        assert_eq!(
            stats.implements_edges, 0,
            "Router should NOT satisfy Handler (missing Name method)"
        );
    }

    #[test]
    fn test_implicit_interface_cap() {
        use std::path::PathBuf;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("go.mod"), "module example.com/test\n\ngo 1.21\n").unwrap();

        let mut graph = CodeGraph::new();

        // Create more than 1000 types (interfaces + structs) to trigger the cap
        for i in 0..600 {
            let go_file = root.join(format!("iface{i}.go"));
            std::fs::write(&go_file, "package main\n").unwrap();
            let file_idx = graph.add_file(go_file.clone(), "go");
            graph.add_symbol(
                file_idx,
                make_symbol(&format!("Iface{i}"), SymbolKind::Interface, None),
            );
            graph.add_symbol(
                file_idx,
                make_symbol(&format!("Struct{i}"), SymbolKind::Struct, None),
            );
        }

        let parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

        // Should not panic; cap kicks in and skips the O(n^2) comparison
        let stats = resolve_go_imports(&mut graph, &parse_results, root, true);
        // With cap, implements_edges should be 0 (skipped)
        assert_eq!(stats.implements_edges, 0, "should be 0 when cap kicks in");
    }
}

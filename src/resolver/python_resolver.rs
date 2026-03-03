//! Python import resolver.
//!
//! Resolves Python import statements (absolute, relative, wildcard, conditional)
//! to graph edges. Integrates into `resolve_all` as Step 7.
//!
//! Resolution algorithm:
//! - **Absolute** (`import os`, `from pkg.sub import name`): walk project_root / package path
//! - **Relative** (`from . import X`, `from ..pkg import Y`): walk relative to importer dir
//! - **Wildcard** (`from module import *`): expand via `__all__` or all-public-names fallback
//! - **Conditional** (try/except imports): same resolution but produces ConditionalImport edges
//! - **Stdlib** (`import os`, `import sys`): creates ExternalPackage nodes

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::parser::ParseResult;
use crate::parser::imports::{ImportInfo, ImportKind};

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Statistics collected by the Python resolver.
#[derive(Debug, Default)]
pub struct PythonResolveStats {
    pub resolved: usize,
    pub unresolved: usize,
    pub conditional: usize,
}

// ---------------------------------------------------------------------------
// Known stdlib modules
// ---------------------------------------------------------------------------

/// A set of well-known Python standard library module names.
///
/// When an import matches one of these, an ExternalPackage node is created
/// instead of an UnresolvedImport node (these will never resolve to project files).
static STDLIB_MODULES: &[&str] = &[
    "abc",
    "argparse",
    "ast",
    "asyncio",
    "base64",
    "binascii",
    "builtins",
    "collections",
    "concurrent",
    "contextlib",
    "copy",
    "csv",
    "dataclasses",
    "datetime",
    "decimal",
    "difflib",
    "email",
    "enum",
    "errno",
    "fileinput",
    "fnmatch",
    "fractions",
    "functools",
    "gc",
    "getpass",
    "glob",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "importlib",
    "inspect",
    "io",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "logging",
    "math",
    "mimetypes",
    "multiprocessing",
    "operator",
    "os",
    "pathlib",
    "pickle",
    "platform",
    "pprint",
    "queue",
    "random",
    "re",
    "shlex",
    "shutil",
    "signal",
    "socket",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "struct",
    "subprocess",
    "sys",
    "tempfile",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "traceback",
    "types",
    "typing",
    "unittest",
    "urllib",
    "uuid",
    "warnings",
    "weakref",
    "xml",
    "xmlrpc",
    "zipfile",
    "zipimport",
    "zlib",
];

fn is_stdlib(module_name: &str) -> bool {
    // Check the top-level component (e.g., "os.path" -> "os")
    let top = module_name.split('.').next().unwrap_or(module_name);
    STDLIB_MODULES.contains(&top)
}

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Try to resolve a Python module path to a filesystem path.
///
/// Given a `base_dir` and a dotted module path (e.g., `"pkg.sub.module"`),
/// this function tries:
/// 1. `base_dir/pkg/sub/module.py` (file)
/// 2. `base_dir/pkg/sub/module/__init__.py` (package)
///
/// Returns `None` if neither exists.
fn resolve_module_path(base_dir: &Path, module_path: &str) -> Option<PathBuf> {
    if module_path.is_empty() {
        // Empty specifier means the base_dir itself — return __init__.py if it exists.
        let init = base_dir.join("__init__.py");
        if init.exists() {
            return Some(init);
        }
        return None;
    }

    // Convert dotted.module.path -> dir/components
    let parts: Vec<&str> = module_path.split('.').collect();

    // Build the path by joining all components
    let mut path = base_dir.to_path_buf();
    for part in &parts {
        path = path.join(part);
    }

    // Try as .py file
    let as_file = path.with_extension("py");
    if as_file.exists() {
        return Some(as_file);
    }

    // Try as package (directory with __init__.py)
    let as_package = path.join("__init__.py");
    if as_package.exists() {
        return Some(as_package);
    }

    None
}

/// Compute the base directory for a relative import.
///
/// `level` is the number of leading dots:
/// - `level=1` means `from . import X` -> current package dir (importer's parent)
/// - `level=2` means `from .. import X` -> parent package dir (go up one more)
/// - `level=3` means `from ... import X` -> grandparent package dir (go up two more)
///
/// Returns `None` if the level goes above the filesystem root.
fn relative_import_base(importer_path: &Path, level: usize) -> Option<PathBuf> {
    // Start from the directory containing the importer file.
    let mut base = importer_path.parent()?.to_path_buf();

    // For level > 1, go up (level - 1) additional directories.
    // level=1 -> stay in current dir (no extra ascent)
    // level=2 -> go up 1
    // level=3 -> go up 2
    for _ in 1..level {
        base = base.parent()?.to_path_buf();
    }

    Some(base)
}

// ---------------------------------------------------------------------------
// __init__.py re-export following
// ---------------------------------------------------------------------------

/// Follow `__init__.py` re-export chains to find the actual defining file.
///
/// When resolving `from pkg import Foo` where `pkg/__init__.py` re-exports `Foo`
/// from a sub-module (`from .sub import Foo`), we want to point the edge at
/// `pkg/sub.py` instead of `pkg/__init__.py`.
///
/// Returns the transitive source file, or the original `init_path` if
/// no better target is found. `depth_limit` prevents infinite loops in
/// circular re-export chains.
fn follow_init_reexport(
    init_path: &Path,
    symbol_name: &str,
    parse_results: &HashMap<PathBuf, ParseResult>,
    depth_limit: usize,
) -> PathBuf {
    if depth_limit == 0 {
        return init_path.to_path_buf();
    }

    let parse_result = match parse_results.get(init_path) {
        Some(pr) => pr,
        None => return init_path.to_path_buf(),
    };

    let _init_dir = match init_path.parent() {
        Some(d) => d,
        None => return init_path.to_path_buf(),
    };

    // Look for a relative import in __init__.py that re-exports `symbol_name`.
    // e.g., `from .sub import Foo` or `from .sub import Foo, Bar`
    for import_info in &parse_result.imports {
        let is_relative = matches!(import_info.kind, ImportKind::PythonRelative { .. });
        if !is_relative {
            continue;
        }

        // Check if this import re-exports the symbol we're looking for.
        let imports_symbol = import_info
            .specifiers
            .iter()
            .any(|spec| spec.name == symbol_name || spec.alias.as_deref() == Some(symbol_name));

        if !imports_symbol {
            continue;
        }

        // Found the re-export import -- resolve its source module.
        let level = match import_info.kind {
            ImportKind::PythonRelative { level } => level,
            _ => continue,
        };
        let base = match relative_import_base(init_path, level) {
            Some(b) => b,
            None => continue,
        };

        let target = resolve_module_path(&base, &import_info.module_path);
        if let Some(target_path) = target {
            // Recurse: follow further re-export chains.
            return follow_init_reexport(&target_path, symbol_name, parse_results, depth_limit - 1);
        }
    }

    // No re-export chain found -- return __init__.py itself.
    init_path.to_path_buf()
}

// ---------------------------------------------------------------------------
// Wildcard import expansion
// ---------------------------------------------------------------------------

/// Expand a wildcard import `from module import *` by collecting target symbols.
///
/// If the target module has `__all__` (detected via `is_exported` on symbols from
/// the extract pass), return only the exported names.
/// Otherwise, return all public names (not starting with `_`).
fn expand_wildcard(
    target_path: &Path,
    parse_results: &HashMap<PathBuf, ParseResult>,
) -> Vec<String> {
    let pr = match parse_results.get(target_path) {
        Some(pr) => pr,
        None => return Vec::new(),
    };

    // Collect exported names from the target's symbols.
    pr.symbols
        .iter()
        .filter(|(sym, _)| sym.is_exported)
        .map(|(sym, _)| sym.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Main resolution function
// ---------------------------------------------------------------------------

/// Resolve all Python imports in `parse_results` and add graph edges.
///
/// Called from `resolve_all` as Step 7 -- after TypeScript/Rust resolution.
///
/// For each file with a `.py` extension:
///   - Absolute imports -> try project_root lookup -> ExternalPackage (stdlib) or UnresolvedImport
///   - Relative imports -> compute base dir from level -> try local path
///   - Wildcard imports -> expand via `__all__` or all-public fallback
///   - Conditional imports -> same resolution but ConditionalImport edge kind
pub fn resolve_python_imports(
    graph: &mut CodeGraph,
    parse_results: &HashMap<PathBuf, ParseResult>,
    project_root: &Path,
) -> PythonResolveStats {
    let mut stats = PythonResolveStats::default();

    // Collect Python file imports to avoid borrow conflicts.
    let python_imports: Vec<(PathBuf, Vec<ImportInfo>)> = parse_results
        .iter()
        .filter(|(path, _)| path.extension().and_then(|e| e.to_str()) == Some("py"))
        .map(|(path, result)| (path.clone(), result.imports.clone()))
        .collect();

    for (file_path, imports) in &python_imports {
        let from_idx = match graph.file_index.get(file_path).copied() {
            Some(idx) => idx,
            None => continue,
        };

        for import_info in imports {
            let is_conditional = matches!(
                import_info.kind,
                ImportKind::PythonConditionalAbsolute
                    | ImportKind::PythonConditionalRelative { .. }
            );

            match &import_info.kind {
                ImportKind::PythonAbsolute | ImportKind::PythonConditionalAbsolute => {
                    resolve_absolute_import(
                        graph,
                        from_idx,
                        file_path,
                        import_info,
                        parse_results,
                        project_root,
                        is_conditional,
                        &mut stats,
                    );
                }
                ImportKind::PythonRelative { level }
                | ImportKind::PythonConditionalRelative { level } => {
                    let level = *level;
                    resolve_relative_import(
                        graph,
                        from_idx,
                        file_path,
                        import_info,
                        parse_results,
                        level,
                        is_conditional,
                        &mut stats,
                    );
                }
                // Non-Python imports are handled by earlier steps.
                _ => {}
            }
        }
    }

    stats
}

/// Resolve a single absolute Python import.
#[allow(clippy::too_many_arguments)]
fn resolve_absolute_import(
    graph: &mut CodeGraph,
    from_idx: petgraph::stable_graph::NodeIndex,
    _file_path: &Path,
    import_info: &ImportInfo,
    parse_results: &HashMap<PathBuf, ParseResult>,
    project_root: &Path,
    is_conditional: bool,
    stats: &mut PythonResolveStats,
) {
    let module_path = &import_info.module_path;

    // Check stdlib first -- these will never resolve to project files.
    if is_stdlib(module_path) {
        // Extract top-level package name for the ExternalPackage node.
        let pkg_name = module_path.split('.').next().unwrap_or(module_path);
        graph.add_external_package(from_idx, pkg_name, module_path);
        return;
    }

    // Check for wildcard import `from module import *`.
    let is_wildcard = import_info
        .specifiers
        .iter()
        .any(|s| s.name == "*" && s.is_namespace);

    if is_wildcard {
        // Resolve the target module first.
        if let Some(target_path) = resolve_module_path(project_root, module_path)
            && let Some(&target_idx) = graph.file_index.get(&target_path)
        {
            let names = expand_wildcard(&target_path, parse_results);
            if names.is_empty() {
                // Create a single edge for the whole wildcard import.
                add_import_edge(graph, from_idx, target_idx, module_path, is_conditional);
                stats.resolved += 1;
            } else {
                for name in &names {
                    add_import_edge(graph, from_idx, target_idx, name, is_conditional);
                }
                stats.resolved += names.len().max(1);
            }
            return;
        }
        // Target not found.
        graph.add_unresolved_import(from_idx, module_path, "Python module not found");
        stats.unresolved += 1;
        return;
    }

    // Try to resolve the module path.
    if let Some(target_path) = resolve_module_path(project_root, module_path) {
        // If target is an __init__.py, follow re-export chains for named imports.
        if target_path.file_name().and_then(|n| n.to_str()) == Some("__init__.py") {
            if import_info.specifiers.is_empty() {
                // `import pkg` with no named specifiers -- point at __init__.py.
                if let Some(&target_idx) = graph.file_index.get(&target_path) {
                    add_import_edge(graph, from_idx, target_idx, module_path, is_conditional);
                    stats.resolved += 1;
                }
            } else {
                for spec in &import_info.specifiers {
                    let resolved_target =
                        follow_init_reexport(&target_path, &spec.name, parse_results, 10);

                    if let Some(&target_idx) = graph.file_index.get(&resolved_target) {
                        add_import_edge(graph, from_idx, target_idx, &spec.name, is_conditional);
                        stats.resolved += 1;
                    } else if let Some(&init_idx) = graph.file_index.get(&target_path) {
                        // Fallback: point at __init__.py if the resolved target isn't indexed.
                        add_import_edge(graph, from_idx, init_idx, &spec.name, is_conditional);
                        stats.resolved += 1;
                    } else {
                        graph.add_unresolved_import(
                            from_idx,
                            &spec.name,
                            "Python module not found",
                        );
                        stats.unresolved += 1;
                    }
                }
            }
        } else {
            // Direct file resolution.
            if let Some(&target_idx) = graph.file_index.get(&target_path) {
                if import_info.specifiers.is_empty() {
                    // `import module` with no specifiers -- single edge.
                    add_import_edge(graph, from_idx, target_idx, module_path, is_conditional);
                    stats.resolved += 1;
                } else {
                    for spec in &import_info.specifiers {
                        add_import_edge(graph, from_idx, target_idx, &spec.name, is_conditional);
                        stats.resolved += 1;
                    }
                }
            } else {
                // File exists on disk but not in graph -- treat as external.
                let pkg_name = module_path.split('.').next().unwrap_or(module_path);
                graph.add_external_package(from_idx, pkg_name, module_path);
            }
        }
    } else {
        // Module path not found on disk.
        graph.add_unresolved_import(from_idx, module_path, "Python module not found");
        stats.unresolved += 1;
    }

    if is_conditional {
        stats.conditional += 1;
    }
}

/// Resolve a single relative Python import.
#[allow(clippy::too_many_arguments)]
fn resolve_relative_import(
    graph: &mut CodeGraph,
    from_idx: petgraph::stable_graph::NodeIndex,
    file_path: &Path,
    import_info: &ImportInfo,
    parse_results: &HashMap<PathBuf, ParseResult>,
    level: usize,
    is_conditional: bool,
    stats: &mut PythonResolveStats,
) {
    let base = match relative_import_base(file_path, level) {
        Some(b) => b,
        None => {
            graph.add_unresolved_import(
                from_idx,
                &import_info.module_path,
                "relative import level too high",
            );
            stats.unresolved += 1;
            return;
        }
    };

    let module_path = &import_info.module_path;

    // Check for wildcard import.
    let is_wildcard = import_info
        .specifiers
        .iter()
        .any(|s| s.name == "*" && s.is_namespace);

    if is_wildcard {
        if let Some(target_path) = resolve_module_path(&base, module_path)
            && let Some(&target_idx) = graph.file_index.get(&target_path)
        {
            let names = expand_wildcard(&target_path, parse_results);
            if names.is_empty() {
                add_import_edge(graph, from_idx, target_idx, module_path, is_conditional);
                stats.resolved += 1;
            } else {
                for name in &names {
                    add_import_edge(graph, from_idx, target_idx, name, is_conditional);
                }
                stats.resolved += names.len().max(1);
            }
            return;
        }
        graph.add_unresolved_import(from_idx, module_path, "Python relative module not found");
        stats.unresolved += 1;
        return;
    }

    // Resolve the module to a path.
    if let Some(target_path) = resolve_module_path(&base, module_path) {
        let is_init = target_path.file_name().and_then(|n| n.to_str()) == Some("__init__.py");

        if is_init && !import_info.specifiers.is_empty() {
            // Follow re-export chains for named imports from __init__.py.
            for spec in &import_info.specifiers {
                let resolved_target =
                    follow_init_reexport(&target_path, &spec.name, parse_results, 10);
                if let Some(&target_idx) = graph.file_index.get(&resolved_target) {
                    add_import_edge(graph, from_idx, target_idx, &spec.name, is_conditional);
                    stats.resolved += 1;
                } else if let Some(&init_idx) = graph.file_index.get(&target_path) {
                    add_import_edge(graph, from_idx, init_idx, &spec.name, is_conditional);
                    stats.resolved += 1;
                } else {
                    graph.add_unresolved_import(from_idx, &spec.name, "Python module not found");
                    stats.unresolved += 1;
                }
            }
        } else if let Some(&target_idx) = graph.file_index.get(&target_path) {
            if import_info.specifiers.is_empty() {
                // `from . import module` -- single edge to the target.
                let label = if module_path.is_empty() {
                    "."
                } else {
                    module_path.as_str()
                };
                add_import_edge(graph, from_idx, target_idx, label, is_conditional);
                stats.resolved += 1;
            } else {
                for spec in &import_info.specifiers {
                    add_import_edge(graph, from_idx, target_idx, &spec.name, is_conditional);
                    stats.resolved += 1;
                }
            }
        } else {
            // Target exists on disk but not indexed.
            graph.add_unresolved_import(from_idx, module_path, "Python relative module not found");
            stats.unresolved += 1;
        }
    } else {
        // Module not found.
        graph.add_unresolved_import(from_idx, module_path, "Python relative module not found");
        stats.unresolved += 1;
    }

    if is_conditional {
        stats.conditional += 1;
    }
}

/// Add a resolved import edge (normal or conditional) from `from_idx` to `to_idx`.
fn add_import_edge(
    graph: &mut CodeGraph,
    from_idx: petgraph::stable_graph::NodeIndex,
    to_idx: petgraph::stable_graph::NodeIndex,
    specifier: &str,
    is_conditional: bool,
) {
    if is_conditional {
        graph.graph.add_edge(
            from_idx,
            to_idx,
            EdgeKind::ConditionalImport {
                specifier: specifier.to_owned(),
            },
        );
    } else {
        graph.add_resolved_import(from_idx, to_idx, specifier);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::graph::node::GraphNode;
    use crate::parser::ParseResult;
    use crate::parser::imports::{ImportInfo, ImportKind, ImportSpecifier};
    use petgraph::visit::EdgeRef;

    /// Create an empty ParseResult for a file with no symbols/imports/exports.
    fn empty_parse_result() -> ParseResult {
        ParseResult {
            symbols: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses: Vec::new(),
        }
    }

    /// Create a ParseResult with just the given imports.
    fn parse_result_with_imports(imports: Vec<ImportInfo>) -> ParseResult {
        ParseResult {
            symbols: Vec::new(),
            imports,
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses: Vec::new(),
        }
    }

    /// Create an ImportInfo for a Python absolute import.
    fn abs_import(module_path: &str, specifiers: Vec<(&str, Option<&str>)>) -> ImportInfo {
        ImportInfo {
            kind: ImportKind::PythonAbsolute,
            module_path: module_path.to_owned(),
            specifiers: specifiers
                .into_iter()
                .map(|(name, alias)| ImportSpecifier {
                    name: name.to_owned(),
                    alias: alias.map(|a| a.to_owned()),
                    is_default: false,
                    is_namespace: false,
                })
                .collect(),
            line: 1,
        }
    }

    /// Create an ImportInfo for a Python relative import.
    fn rel_import(
        level: usize,
        module_path: &str,
        specifiers: Vec<(&str, Option<&str>)>,
    ) -> ImportInfo {
        ImportInfo {
            kind: ImportKind::PythonRelative { level },
            module_path: module_path.to_owned(),
            specifiers: specifiers
                .into_iter()
                .map(|(name, alias)| ImportSpecifier {
                    name: name.to_owned(),
                    alias: alias.map(|a| a.to_owned()),
                    is_default: false,
                    is_namespace: false,
                })
                .collect(),
            line: 1,
        }
    }

    /// Create an ImportInfo for a wildcard import.
    fn wildcard_import(kind: ImportKind, module_path: &str) -> ImportInfo {
        ImportInfo {
            kind,
            module_path: module_path.to_owned(),
            specifiers: vec![ImportSpecifier {
                name: "*".to_owned(),
                alias: None,
                is_default: false,
                is_namespace: true,
            }],
            line: 1,
        }
    }

    /// Create an ImportInfo for a conditional import.
    fn cond_import(module_path: &str, specifiers: Vec<(&str, Option<&str>)>) -> ImportInfo {
        ImportInfo {
            kind: ImportKind::PythonConditionalAbsolute,
            module_path: module_path.to_owned(),
            specifiers: specifiers
                .into_iter()
                .map(|(name, alias)| ImportSpecifier {
                    name: name.to_owned(),
                    alias: alias.map(|a| a.to_owned()),
                    is_default: false,
                    is_namespace: false,
                })
                .collect(),
            line: 1,
        }
    }

    /// Count the number of ResolvedImport edges in the graph.
    fn count_resolved_edges(graph: &CodeGraph) -> usize {
        graph
            .graph
            .edge_indices()
            .filter(|&e| matches!(graph.graph[e], EdgeKind::ResolvedImport { .. }))
            .count()
    }

    /// Count the number of UnresolvedImport nodes in the graph.
    fn count_unresolved_nodes(graph: &CodeGraph) -> usize {
        graph
            .graph
            .node_indices()
            .filter(|&n| matches!(graph.graph[n], GraphNode::UnresolvedImport { .. }))
            .count()
    }

    /// Count the number of ExternalPackage nodes in the graph.
    fn count_external_packages(graph: &CodeGraph) -> usize {
        graph
            .graph
            .node_indices()
            .filter(|&n| matches!(graph.graph[n], GraphNode::ExternalPackage(_)))
            .count()
    }

    /// Count the number of ConditionalImport edges in the graph.
    fn count_conditional_edges(graph: &CodeGraph) -> usize {
        graph
            .graph
            .edge_indices()
            .filter(|&e| matches!(graph.graph[e], EdgeKind::ConditionalImport { .. }))
            .count()
    }

    // Test 1: absolute import resolves to project file
    #[test]
    fn test_python_absolute_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create mypackage/module.py on disk.
        let pkg_dir = root.join("mypackage");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let module_path = pkg_dir.join("module.py");
        std::fs::write(&module_path, "x = 1").unwrap();

        let mut graph = CodeGraph::new();
        let importer = root.join("main.py");
        graph.add_file(importer.clone(), "python");
        graph.add_file(module_path.clone(), "python");

        let mut parse_results = HashMap::new();
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![abs_import("mypackage.module", vec![("x", None)])]),
        );
        parse_results.insert(module_path.clone(), empty_parse_result());

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        assert_eq!(stats.resolved, 1, "should resolve 1 absolute import");
        assert_eq!(count_resolved_edges(&graph), 1);
        assert_eq!(count_unresolved_nodes(&graph), 0);
    }

    // Test 2: relative import from . -> ./sibling.py
    #[test]
    fn test_python_relative_import_dot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let pkg_dir = root.join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let importer = pkg_dir.join("importer.py");
        let sibling = pkg_dir.join("sibling.py");
        std::fs::write(&importer, "").unwrap();
        std::fs::write(&sibling, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(sibling.clone(), "python");

        let mut parse_results = HashMap::new();
        // from . import sibling
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![rel_import(1, "sibling", vec![("sibling", None)])]),
        );
        parse_results.insert(sibling.clone(), empty_parse_result());

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        assert_eq!(stats.resolved, 1, "should resolve relative import");
        assert_eq!(count_resolved_edges(&graph), 1);
    }

    // Test 3: relative import from .. -> parent package
    #[test]
    fn test_python_relative_import_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let pkg_dir = root.join("pkg");
        let sub_dir = pkg_dir.join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();

        let importer = sub_dir.join("importer.py");
        let util = pkg_dir.join("util.py");
        std::fs::write(&importer, "").unwrap();
        std::fs::write(&util, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(util.clone(), "python");

        let mut parse_results = HashMap::new();
        // from .. import util -> go up 2 dirs (pkg), then resolve util
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![rel_import(2, "util", vec![("util", None)])]),
        );
        parse_results.insert(util.clone(), empty_parse_result());

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        assert_eq!(stats.resolved, 1, "should resolve dotdot relative import");
        assert_eq!(count_resolved_edges(&graph), 1);
    }

    // Test 4: stdlib import creates ExternalPackage node
    #[test]
    fn test_python_stdlib_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");

        let mut parse_results = HashMap::new();
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![abs_import("os", vec![])]),
        );

        let _stats = resolve_python_imports(&mut graph, &parse_results, root);

        // stdlib creates an ExternalPackage node, not an UnresolvedImport.
        assert_eq!(count_external_packages(&graph), 1);
        assert_eq!(count_unresolved_nodes(&graph), 0);
    }

    // Test 5: unresolvable import creates UnresolvedImport node
    #[test]
    fn test_python_unresolved_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");

        let mut parse_results = HashMap::new();
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![abs_import("nonexistent_pkg", vec![("X", None)])]),
        );

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        assert_eq!(count_unresolved_nodes(&graph), 1);
        assert_eq!(stats.unresolved, 1);
    }

    // Test 6: __init__.py recognized as package root
    #[test]
    fn test_python_init_package_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let pkg_dir = root.join("mypkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let init_path = pkg_dir.join("__init__.py");
        std::fs::write(&init_path, "").unwrap();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(init_path.clone(), "python");

        let mut parse_results = HashMap::new();
        // `import mypkg` -> resolves to mypkg/__init__.py
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![abs_import("mypkg", vec![])]),
        );
        parse_results.insert(init_path.clone(), empty_parse_result());

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        assert_eq!(stats.resolved, 1);
        assert_eq!(count_resolved_edges(&graph), 1);
    }

    // Test 7: transitive re-export through __init__.py
    #[test]
    fn test_python_init_reexport_transitive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let pkg_dir = root.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let init_path = pkg_dir.join("__init__.py");
        let sub_path = pkg_dir.join("sub.py");
        std::fs::write(&init_path, "from .sub import Foo").unwrap();
        std::fs::write(&sub_path, "class Foo: pass").unwrap();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(init_path.clone(), "python");
        graph.add_file(sub_path.clone(), "python");

        // __init__.py has `from .sub import Foo` (relative import re-exporting Foo)
        let init_import = rel_import(1, "sub", vec![("Foo", None)]);

        let mut parse_results = HashMap::new();
        // main.py: `from pkg import Foo`
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![abs_import("pkg", vec![("Foo", None)])]),
        );
        // pkg/__init__.py re-exports Foo from .sub
        parse_results.insert(
            init_path.clone(),
            parse_result_with_imports(vec![init_import]),
        );
        parse_results.insert(sub_path.clone(), empty_parse_result());

        let _stats = resolve_python_imports(&mut graph, &parse_results, root);

        // Should resolve to pkg/sub.py (transitive) instead of pkg/__init__.py
        let sub_idx = graph.file_index.get(&sub_path).copied().unwrap();
        let from_idx = graph.file_index.get(&importer).copied().unwrap();
        let edge_to_sub = graph.graph.edges(from_idx).any(|e| e.target() == sub_idx);
        assert!(
            edge_to_sub,
            "should have edge from main.py to pkg/sub.py (transitive)"
        );
    }

    // Test 8: wildcard import with __all__ (only exported symbols)
    #[test]
    fn test_python_wildcard_import_with_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let target_path = root.join("mymodule.py");
        std::fs::write(&target_path, "").unwrap();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(target_path.clone(), "python");

        // Simulate __all__ = ["X", "Y"] by setting is_exported=true for X and Y.
        use crate::graph::node::{SymbolInfo, SymbolKind, SymbolVisibility};
        let x_sym = SymbolInfo {
            name: "X".to_owned(),
            kind: SymbolKind::Variable,
            is_exported: true,
            ..Default::default()
        };
        let y_sym = SymbolInfo {
            name: "Y".to_owned(),
            kind: SymbolKind::Variable,
            is_exported: true,
            ..Default::default()
        };
        let hidden_sym = SymbolInfo {
            name: "_internal".to_owned(),
            kind: SymbolKind::Variable,
            is_exported: false,
            visibility: SymbolVisibility::Private,
            ..Default::default()
        };

        let target_parse = ParseResult {
            symbols: vec![
                (x_sym, Vec::new()),
                (y_sym, Vec::new()),
                (hidden_sym, Vec::new()),
            ],
            imports: Vec::new(),
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses: Vec::new(),
        };

        let mut parse_results = HashMap::new();
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![wildcard_import(
                ImportKind::PythonAbsolute,
                "mymodule",
            )]),
        );
        parse_results.insert(target_path.clone(), target_parse);

        let _stats = resolve_python_imports(&mut graph, &parse_results, root);

        // Should create 2 edges (X, Y -- not _internal)
        assert_eq!(count_resolved_edges(&graph), 2, "should have X and Y edges");
    }

    // Test 9: wildcard import without __all__ (all public names)
    #[test]
    fn test_python_wildcard_import_no_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let target_path = root.join("mymodule.py");
        std::fs::write(&target_path, "").unwrap();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(target_path.clone(), "python");

        // Without __all__: all non-underscore symbols are exported.
        use crate::graph::node::{SymbolInfo, SymbolKind, SymbolVisibility};
        let pub_sym = SymbolInfo {
            name: "PublicFunc".to_owned(),
            kind: SymbolKind::Function,
            is_exported: true,
            ..Default::default()
        };
        let priv_sym = SymbolInfo {
            name: "_private".to_owned(),
            kind: SymbolKind::Function,
            is_exported: false,
            visibility: SymbolVisibility::Private,
            ..Default::default()
        };

        let target_parse = ParseResult {
            symbols: vec![(pub_sym, Vec::new()), (priv_sym, Vec::new())],
            imports: Vec::new(),
            exports: Vec::new(),
            relationships: Vec::new(),
            rust_uses: Vec::new(),
        };

        let mut parse_results = HashMap::new();
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![wildcard_import(
                ImportKind::PythonAbsolute,
                "mymodule",
            )]),
        );
        parse_results.insert(target_path.clone(), target_parse);

        let _stats = resolve_python_imports(&mut graph, &parse_results, root);

        // Should create 1 edge (PublicFunc, not _private)
        assert_eq!(
            count_resolved_edges(&graph),
            1,
            "should only have edge for public symbol"
        );
    }

    // Test 10: conditional import creates ConditionalImport edge kind
    #[test]
    fn test_python_conditional_import_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let fast_path = root.join("fast_impl.py");
        std::fs::write(&fast_path, "def accel(): pass").unwrap();

        let importer = root.join("main.py");
        std::fs::write(&importer, "").unwrap();

        let mut graph = CodeGraph::new();
        graph.add_file(importer.clone(), "python");
        graph.add_file(fast_path.clone(), "python");

        let mut parse_results = HashMap::new();
        // try: from fast_impl import accel  -> PythonConditionalAbsolute
        parse_results.insert(
            importer.clone(),
            parse_result_with_imports(vec![cond_import("fast_impl", vec![("accel", None)])]),
        );
        parse_results.insert(fast_path.clone(), empty_parse_result());

        let stats = resolve_python_imports(&mut graph, &parse_results, root);

        // Should create a ConditionalImport edge (not a ResolvedImport).
        assert_eq!(
            count_conditional_edges(&graph),
            1,
            "should have 1 ConditionalImport edge"
        );
        assert_eq!(
            count_resolved_edges(&graph),
            0,
            "should have no ResolvedImport edges"
        );
        assert_eq!(stats.conditional, 1);
    }
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{FileInfo, FileKind, GraphNode, SymbolInfo, SymbolKind, SymbolVisibility},
};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single unreferenced symbol within a file.
#[derive(Debug, Clone)]
pub struct DeadSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}

/// Result of dead code analysis.
#[derive(Debug, Clone)]
pub struct DeadCodeResult {
    /// Files with zero incoming import edges that are not entry points.
    pub unreachable_files: Vec<PathBuf>,
    /// Symbols with zero incoming Calls edges, grouped by file path.
    /// Each entry is (file_path, vec_of_dead_symbols).
    pub unreferenced_symbols: Vec<(PathBuf, Vec<DeadSymbol>)>,
}

// ---------------------------------------------------------------------------
// Entry-point detection helpers
// ---------------------------------------------------------------------------

/// Returns true if the symbol should be excluded from dead code results.
///
/// Exclusion rules (ANALYSIS-02):
/// - Functions named "main"
/// - Trait implementations (`trait_impl.is_some()`)
/// - Pub/PubCrate Rust symbols
/// - Exported TS/JS symbols (`is_exported`)
/// - Symbols in test files or with "test_" prefix
fn is_entry_point_symbol(sym: &SymbolInfo, file_info: &FileInfo) -> bool {
    // main function
    if sym.name == "main" && matches!(sym.kind, SymbolKind::Function) {
        return true;
    }

    // Trait implementations
    if sym.trait_impl.is_some() {
        return true;
    }

    // Rust: pub or pub(crate) symbols
    if file_info.language == "rust" {
        if sym.visibility == SymbolVisibility::Pub || sym.visibility == SymbolVisibility::PubCrate {
            return true;
        }
    } else {
        // TS/JS: exported symbols
        if sym.is_exported {
            return true;
        }
    }

    // Symbols with test_ prefix
    if sym.name.starts_with("test_") {
        return true;
    }

    // Symbols in test files (checked at the file level, but also here for completeness)
    let path_str = file_info.path.to_string_lossy();
    if path_str.contains("/tests/")
        || path_str.contains("/_tests_/")
        || path_str.contains("/__tests__/")
        || path_str.ends_with("_test.rs")
        || path_str.ends_with("_test.ts")
        || path_str.ends_with(".test.ts")
        || path_str.ends_with(".spec.ts")
    {
        return true;
    }

    false
}

/// Returns true if the file should be excluded from dead code results.
///
/// Exclusion rules (ANALYSIS-02):
/// - Files named main.rs, lib.rs
/// - Files named index.ts, index.js, index.tsx, index.jsx (barrel entry points)
/// - Files inside test directories
fn is_entry_point_file(file_info: &FileInfo) -> bool {
    let file_name = file_info
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Common entry point file names
    let entry_names = [
        "main.rs", "lib.rs",
        "index.ts", "index.js", "index.tsx", "index.jsx",
    ];
    if entry_names.contains(&file_name) {
        return true;
    }

    // Test directories
    let path_str = file_info.path.to_string_lossy();
    if path_str.contains("/tests/")
        || path_str.contains("/_tests_/")
        || path_str.contains("/__tests__/")
        || file_name.ends_with("_test.rs")
        || file_name.ends_with("_test.ts")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
    {
        return true;
    }

    // Non-source files (doc, config, ci, asset, other) are not dead code candidates
    if !matches!(file_info.kind, FileKind::Source) {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Main query function
// ---------------------------------------------------------------------------

/// Detect dead code: unreachable files and unreferenced symbols.
///
/// - `graph`: the code graph to analyze
/// - `root`: the project root path (used for relative path computation)
/// - `scope`: optional path scope; if provided, only analyze files under this path
///
/// Returns a `DeadCodeResult` with unreachable files and unreferenced symbols.
pub fn find_dead_code(graph: &CodeGraph, root: &Path, scope: Option<&Path>) -> DeadCodeResult {
    // Compute absolute scope path if provided
    let abs_scope: Option<PathBuf> = scope.map(|s| {
        if s.is_absolute() {
            s.to_path_buf()
        } else {
            root.join(s)
        }
    });

    // Helper: check if a path is under the scope
    let in_scope = |path: &Path| -> bool {
        match &abs_scope {
            None => true,
            Some(scope_path) => path.starts_with(scope_path),
        }
    };

    // --- Unreachable files ---
    // A file is unreachable if it has zero incoming ResolvedImport or BarrelReExportAll edges
    // AND it is not an entry point file.
    let mut unreachable_files: Vec<PathBuf> = Vec::new();

    for (file_path, &file_idx) in &graph.file_index {
        if !in_scope(file_path) {
            continue;
        }

        let file_info = match &graph.graph[file_idx] {
            GraphNode::File(fi) => fi,
            _ => continue,
        };

        // Skip entry point files
        if is_entry_point_file(file_info) {
            continue;
        }

        // Count incoming import edges
        let importer_count = graph
            .graph
            .edges_directed(file_idx, Direction::Incoming)
            .filter(|e| {
                matches!(
                    e.weight(),
                    EdgeKind::ResolvedImport { .. } | EdgeKind::BarrelReExportAll
                )
            })
            .count();

        if importer_count == 0 {
            unreachable_files.push(file_path.clone());
        }
    }

    // Sort for deterministic output
    unreachable_files.sort();

    // --- Unreferenced symbols ---
    // A symbol is unreferenced if it has zero incoming Calls edges
    // AND it is not excluded by entry-point rules.

    // We need to find, for each symbol node, which file contains it.
    // The Contains edge goes: File -> Symbol.
    // So for a symbol node, we look for incoming Contains edges.

    // Build a map: symbol NodeIndex -> FileInfo (for exclusion checks)
    // We iterate all node indices, for Symbol nodes check incoming Contains edge
    let mut sym_to_file: HashMap<petgraph::stable_graph::NodeIndex, FileInfo> = HashMap::new();

    for node_idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(_) = &graph.graph[node_idx] {
            // Find the file that contains this symbol via incoming Contains edge
            for edge in graph.graph.edges_directed(node_idx, Direction::Incoming) {
                if matches!(edge.weight(), EdgeKind::Contains) {
                    if let GraphNode::File(fi) = &graph.graph[edge.source()] {
                        sym_to_file.insert(node_idx, fi.clone());
                        break;
                    }
                }
            }
        }
    }

    // Group dead symbols by file path
    let mut dead_by_file: HashMap<PathBuf, Vec<DeadSymbol>> = HashMap::new();

    for node_idx in graph.graph.node_indices() {
        let sym = match &graph.graph[node_idx] {
            GraphNode::Symbol(s) => s.clone(),
            _ => continue,
        };

        // Get file info for this symbol
        let file_info = match sym_to_file.get(&node_idx) {
            Some(fi) => fi,
            None => continue, // orphan symbol, skip
        };

        // Check scope
        if !in_scope(&file_info.path) {
            continue;
        }

        // Skip entry-point symbols
        if is_entry_point_symbol(&sym, file_info) {
            continue;
        }

        // Count incoming Calls edges
        let call_count = graph
            .graph
            .edges_directed(node_idx, Direction::Incoming)
            .filter(|e| matches!(e.weight(), EdgeKind::Calls))
            .count();

        if call_count == 0 {
            let dead_sym = DeadSymbol {
                name: sym.name.clone(),
                kind: crate::query::find::kind_to_str(&sym.kind).to_string(),
                line: sym.line,
            };
            dead_by_file
                .entry(file_info.path.clone())
                .or_default()
                .push(dead_sym);
        }
    }

    // Convert map to sorted vec of (path, symbols)
    let mut unreferenced_symbols: Vec<(PathBuf, Vec<DeadSymbol>)> = dead_by_file.into_iter().collect();
    unreferenced_symbols.sort_by(|a, b| a.0.cmp(&b.0));
    // Sort symbols within each file by line number
    for (_, syms) in &mut unreferenced_symbols {
        syms.sort_by_key(|s| s.line);
    }

    DeadCodeResult {
        unreachable_files,
        unreferenced_symbols,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::graph::{
        CodeGraph,
        edge::EdgeKind,
        node::{SymbolInfo, SymbolKind, SymbolVisibility},
    };

    fn make_symbol(
        name: &str,
        kind: SymbolKind,
        vis: SymbolVisibility,
        exported: bool,
        trait_impl: Option<String>,
        line: usize,
    ) -> SymbolInfo {
        SymbolInfo {
            name: name.into(),
            kind,
            line,
            col: 0,
            is_exported: exported,
            is_default: false,
            visibility: vis,
            trait_impl,
        }
    }

    #[test]
    fn test_unreachable_file() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/unused_module.rs");
        graph.add_file(file_path.clone(), "rust");

        let result = find_dead_code(&graph, &root, None);
        assert!(
            result.unreachable_files.contains(&file_path),
            "File with zero importers should be unreachable"
        );
    }

    #[test]
    fn test_referenced_file_not_dead() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_a = root.join("src/utils.rs");
        let file_b = root.join("src/main.rs");
        let a_idx = graph.add_file(file_a.clone(), "rust");
        let b_idx = graph.add_file(file_b.clone(), "rust");

        // b imports a
        graph.graph.add_edge(
            b_idx,
            a_idx,
            EdgeKind::ResolvedImport {
                specifier: "./utils".into(),
            },
        );

        let result = find_dead_code(&graph, &root, None);
        assert!(
            !result.unreachable_files.contains(&file_a),
            "File with an importer should NOT be unreachable"
        );
    }

    #[test]
    fn test_unreferenced_symbol() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/utils.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        graph.add_symbol(
            file_idx,
            make_symbol("unused_helper", SymbolKind::Function, SymbolVisibility::Private, false, None, 10),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            all_dead_names.contains(&"unused_helper"),
            "Private symbol with zero Calls edges should be dead"
        );
    }

    #[test]
    fn test_main_function_excluded() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/helpers.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        graph.add_symbol(
            file_idx,
            make_symbol("main", SymbolKind::Function, SymbolVisibility::Private, false, None, 1),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            !all_dead_names.contains(&"main"),
            "main function should be excluded from dead code"
        );
    }

    #[test]
    fn test_pub_symbol_excluded() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/lib.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        graph.add_symbol(
            file_idx,
            make_symbol("public_api", SymbolKind::Function, SymbolVisibility::Pub, false, None, 5),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            !all_dead_names.contains(&"public_api"),
            "pub Rust symbol should be excluded from dead code"
        );
    }

    #[test]
    fn test_exported_ts_symbol_excluded() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/utils.ts");
        let file_idx = graph.add_file(file_path.clone(), "typescript");

        graph.add_symbol(
            file_idx,
            make_symbol("exportedFn", SymbolKind::Function, SymbolVisibility::Private, true, None, 3),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            !all_dead_names.contains(&"exportedFn"),
            "is_exported TS symbol should be excluded from dead code"
        );
    }

    #[test]
    fn test_trait_impl_excluded() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_path = root.join("src/display.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        graph.add_symbol(
            file_idx,
            make_symbol(
                "fmt",
                SymbolKind::ImplMethod,
                SymbolVisibility::Private,
                false,
                Some("Display".into()),
                8,
            ),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            !all_dead_names.contains(&"fmt"),
            "trait impl symbol should be excluded from dead code"
        );
    }

    #[test]
    fn test_test_function_excluded() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");

        // test_ prefix
        let file_path = root.join("src/helpers.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");
        graph.add_symbol(
            file_idx,
            make_symbol("test_something", SymbolKind::Function, SymbolVisibility::Private, false, None, 20),
        );

        // file in tests/ directory
        let test_file_path = root.join("tests/integration.rs");
        let test_file_idx = graph.add_file(test_file_path.clone(), "rust");
        graph.add_symbol(
            test_file_idx,
            make_symbol("run_test", SymbolKind::Function, SymbolVisibility::Private, false, None, 5),
        );

        let result = find_dead_code(&graph, &root, None);
        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            !all_dead_names.contains(&"test_something"),
            "Symbol with test_ prefix should be excluded"
        );
        assert!(
            !all_dead_names.contains(&"run_test"),
            "Symbol in tests/ directory should be excluded"
        );
    }

    #[test]
    fn test_scope_filter() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");

        // File inside scope
        let in_scope_file = root.join("src/module/helper.rs");
        let in_scope_idx = graph.add_file(in_scope_file.clone(), "rust");
        graph.add_symbol(
            in_scope_idx,
            make_symbol("in_scope_fn", SymbolKind::Function, SymbolVisibility::Private, false, None, 1),
        );

        // File outside scope
        let out_of_scope_file = root.join("other/unrelated.rs");
        let out_of_scope_idx = graph.add_file(out_of_scope_file.clone(), "rust");
        graph.add_symbol(
            out_of_scope_idx,
            make_symbol("out_of_scope_fn", SymbolKind::Function, SymbolVisibility::Private, false, None, 1),
        );

        // Run with scope = "src/module"
        let scope_path = PathBuf::from("src/module");
        let result = find_dead_code(&graph, &root, Some(&scope_path));

        let all_dead_names: Vec<&str> = result
            .unreferenced_symbols
            .iter()
            .flat_map(|(_, syms)| syms.iter().map(|s| s.name.as_str()))
            .collect();

        assert!(
            all_dead_names.contains(&"in_scope_fn"),
            "Symbol inside scope should be analyzed"
        );
        assert!(
            !all_dead_names.contains(&"out_of_scope_fn"),
            "Symbol outside scope should NOT be analyzed"
        );

        // Check file scope filtering
        assert!(
            result.unreachable_files.contains(&in_scope_file),
            "File inside scope with no importers should be unreachable"
        );
        assert!(
            !result.unreachable_files.contains(&out_of_scope_file),
            "File outside scope should NOT be in unreachable list"
        );
    }
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{FileKind, GraphNode, SymbolKind, SymbolVisibility},
};
use crate::query::find::kind_to_str;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Dependency role of a file in the project.
#[derive(Debug, Clone, PartialEq)]
pub enum FileRole {
    EntryPoint,
    LibraryRoot,
    Test,
    Config,
    Types,
    Utility,
}

/// Graph topology label for a file.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphLabel {
    Hub,    // >= 5 importers
    Leaf,   // 0 importers
    Bridge, // 2+ importers AND 3+ imports
}

/// An exported symbol from a file.
#[derive(Debug, Clone)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: String, // "fn", "struct", etc.
}

/// Summary information for a single file.
#[derive(Debug, Clone)]
pub struct FileSummary {
    pub relative_path: String,
    pub role: FileRole,
    pub line_count: usize,
    pub symbol_count: usize,
    /// Breakdown of all symbols by kind string (e.g. "fn" -> 3, "struct" -> 1).
    pub symbol_kinds: HashMap<String, usize>,
    pub exports: Vec<ExportedSymbol>,
    pub import_count: usize,     // outgoing import edges
    pub importer_count: usize,   // incoming import edges
    pub graph_label: Option<GraphLabel>,
}

// ---------------------------------------------------------------------------
// Role detection helpers
// ---------------------------------------------------------------------------

/// Detect the dependency role of a file based on its name, path, and symbols.
fn detect_role(
    file_info: &crate::graph::node::FileInfo,
    root: &Path,
    outgoing_reexport_count: usize,
    symbols: &[crate::graph::node::SymbolInfo],
) -> FileRole {
    let path = &file_info.path;

    // Config/Ci files
    match file_info.kind {
        FileKind::Config | FileKind::Ci => return FileRole::Config,
        _ => {}
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let path_str = path.to_string_lossy();

    // Test file detection
    let is_test = file_name.contains("test")
        || file_name.contains("spec")
        || path_str.contains("/tests/")
        || path_str.contains("/__tests__/")
        || file_name.ends_with("_test.rs")
        || path_str.contains("\\tests\\")
        || path_str.contains("\\_tests_\\");

    if is_test {
        return FileRole::Test;
    }

    // Entry point detection: common entry point file names near the root
    let entry_point_names = [
        "main.rs", "main.ts", "main.js", "index.ts", "index.js",
        "app.ts", "app.js",
    ];
    if entry_point_names.contains(&file_name) {
        // Check depth from root: count components between root and the file's parent dir
        if let Ok(rel) = path.strip_prefix(root) {
            let depth = rel.components().count();
            // depth=1 means directly in root, depth=2 means one level deep (e.g. src/main.rs)
            if depth <= 2 {
                return FileRole::EntryPoint;
            }
        }
    }

    // Library root detection
    let lib_root_names = ["lib.rs", "mod.rs"];
    if lib_root_names.contains(&file_name) {
        return FileRole::LibraryRoot;
    }

    // Many re-exports => library root
    if outgoing_reexport_count >= 3 {
        return FileRole::LibraryRoot;
    }

    // Types file detection: >= 60% of symbols are type-defining kinds
    if !symbols.is_empty() {
        let type_kinds = [
            SymbolKind::TypeAlias,
            SymbolKind::Interface,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
        ];
        let type_count = symbols
            .iter()
            .filter(|s| type_kinds.contains(&s.kind))
            .count();
        let fn_kinds = [SymbolKind::Function, SymbolKind::ImplMethod, SymbolKind::Method];
        let fn_count = symbols
            .iter()
            .filter(|s| fn_kinds.contains(&s.kind))
            .count();
        if type_count > 0
            && fn_count == 0
            && type_count * 100 / symbols.len() >= 60
        {
            return FileRole::Types;
        }
    }

    FileRole::Utility
}

/// Determine graph label based on importer and import counts.
fn compute_graph_label(importer_count: usize, import_count: usize) -> Option<GraphLabel> {
    if importer_count >= 5 {
        Some(GraphLabel::Hub)
    } else if importer_count == 0 {
        Some(GraphLabel::Leaf)
    } else if importer_count >= 2 && import_count >= 3 {
        Some(GraphLabel::Bridge)
    } else {
        None
    }
}

/// Count lines in a file by counting `\n` bytes.
fn count_lines(path: &Path) -> usize {
    match std::fs::read(path) {
        Ok(bytes) => bytes.iter().filter(|&&b| b == b'\n').count(),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Main query function
// ---------------------------------------------------------------------------

/// Build a summary for a single file in the graph.
///
/// Returns `Err` if the file path is not found in the graph.
pub fn file_summary(
    graph: &CodeGraph,
    root: &Path,
    file_path: &Path,
) -> Result<FileSummary, String> {
    // Resolve path: relative paths are joined to root.
    let abs_path: PathBuf = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        root.join(file_path)
    };

    let file_idx = graph
        .file_index
        .get(&abs_path)
        .copied()
        .ok_or_else(|| format!("File not found: {}", file_path.display()))?;

    // Get FileInfo
    let file_info = match &graph.graph[file_idx] {
        GraphNode::File(fi) => fi.clone(),
        _ => return Err(format!("Node at path is not a File: {}", file_path.display())),
    };

    // Collect all symbols via Contains edges (top-level symbols only)
    let all_symbols: Vec<crate::graph::node::SymbolInfo> = graph
        .graph
        .edges(file_idx)
        .filter_map(|edge_ref| {
            if let EdgeKind::Contains = edge_ref.weight() {
                if let GraphNode::Symbol(ref sym) = graph.graph[edge_ref.target()] {
                    return Some(sym.clone());
                }
            }
            None
        })
        .collect();

    let symbol_count = all_symbols.len();

    // Build symbol kind breakdown map
    let mut symbol_kinds: HashMap<String, usize> = HashMap::new();
    for sym in &all_symbols {
        *symbol_kinds.entry(kind_to_str(&sym.kind).to_string()).or_insert(0) += 1;
    }

    // Filter exported symbols:
    // - For TS/JS: is_exported == true
    // - For Rust: visibility is Pub or PubCrate
    let is_rust = file_info.language == "rust";
    let exports: Vec<ExportedSymbol> = all_symbols
        .iter()
        .filter(|sym| {
            if is_rust {
                sym.visibility == SymbolVisibility::Pub || sym.visibility == SymbolVisibility::PubCrate
            } else {
                sym.is_exported
            }
        })
        .map(|sym| ExportedSymbol {
            name: sym.name.clone(),
            kind: kind_to_str(&sym.kind).to_string(),
        })
        .collect();

    // Count outgoing import edges (ResolvedImport, RustImport, ReExport, BarrelReExportAll)
    let mut import_count: usize = 0;
    let mut reexport_count: usize = 0;
    for edge_ref in graph.graph.edges(file_idx) {
        match edge_ref.weight() {
            EdgeKind::ResolvedImport { .. }
            | EdgeKind::RustImport { .. } => {
                import_count += 1;
            }
            EdgeKind::ReExport { .. } => {
                import_count += 1;
                reexport_count += 1;
            }
            EdgeKind::BarrelReExportAll => {
                import_count += 1;
                reexport_count += 1;
            }
            _ => {}
        }
    }

    // Count incoming import edges (files that import this file)
    let importer_count: usize = graph
        .graph
        .edges_directed(file_idx, Direction::Incoming)
        .filter(|edge_ref| {
            matches!(
                edge_ref.weight(),
                EdgeKind::ResolvedImport { .. } | EdgeKind::BarrelReExportAll
            )
        })
        .count();

    // Graph label
    let graph_label = compute_graph_label(importer_count, import_count);

    // Role detection
    let role = detect_role(&file_info, root, reexport_count, &all_symbols);

    // Line count
    let line_count = count_lines(&abs_path);

    // Compute relative path for display
    let relative_path = abs_path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned());

    Ok(FileSummary {
        relative_path,
        role,
        line_count,
        symbol_count,
        symbol_kinds,
        exports,
        import_count,
        importer_count,
        graph_label,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use petgraph::Direction;

    use super::*;
    use crate::graph::{
        CodeGraph,
        edge::EdgeKind,
        node::{FileInfo, FileKind, SymbolInfo, SymbolKind, SymbolVisibility},
    };

    fn make_symbol(name: &str, kind: SymbolKind, vis: SymbolVisibility, exported: bool) -> SymbolInfo {
        SymbolInfo {
            name: name.into(),
            kind,
            line: 1,
            col: 0,
            is_exported: exported,
            is_default: false,
            visibility: vis,
            trait_impl: None,
        }
    }

    #[test]
    fn test_entry_point_detection() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");
        // main.rs at depth 2 (src/main.rs)
        let file_path = root.join("src/main.rs");
        graph.add_file(file_path.clone(), "rust");

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::EntryPoint,
            "main.rs at src/ depth should be EntryPoint"
        );
    }

    #[test]
    fn test_test_file_detection() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        // File in tests/ directory
        let file_path = root.join("tests/integration_test.rs");
        graph.add_file(file_path.clone(), "rust");
        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::Test,
            "File in tests/ should be Test"
        );
    }

    #[test]
    fn test_test_file_detection_by_name() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        // File with "test" in name
        let file_path = root.join("src/test_utils.rs");
        graph.add_file(file_path.clone(), "rust");
        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::Test,
            "File with 'test' in name should be Test"
        );
    }

    #[test]
    fn test_library_root_detection() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_path = root.join("src/lib.rs");
        graph.add_file(file_path.clone(), "rust");
        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::LibraryRoot,
            "lib.rs should be LibraryRoot"
        );
    }

    #[test]
    fn test_types_file_detection() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_path = root.join("src/types.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        // Add 3 struct symbols (all type-defining) and 0 functions
        graph.add_symbol(file_idx, make_symbol("TypeA", SymbolKind::Struct, SymbolVisibility::Pub, false));
        graph.add_symbol(file_idx, make_symbol("TypeB", SymbolKind::Struct, SymbolVisibility::Pub, false));
        graph.add_symbol(file_idx, make_symbol("TypeC", SymbolKind::Enum, SymbolVisibility::Pub, false));

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::Types,
            "File with 100% type symbols and 0 functions should be Types"
        );
    }

    #[test]
    fn test_utility_default() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        // A regular file that doesn't match any specific role
        let file_path = root.join("src/helpers.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");
        // Add some function symbols (not all types)
        graph.add_symbol(file_idx, make_symbol("helper_fn", SymbolKind::Function, SymbolVisibility::Pub, false));

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.role,
            FileRole::Utility,
            "Regular file with function symbols should default to Utility"
        );
    }

    #[test]
    fn test_hub_label() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_path = root.join("src/central.rs");
        let hub_idx = graph.add_file(file_path.clone(), "rust");

        // Add 5 files that import from hub (incoming ResolvedImport edges)
        for i in 0..5 {
            let importer_path = root.join(format!("src/importer{}.rs", i));
            let importer_idx = graph.add_file(importer_path, "rust");
            graph.graph.add_edge(
                importer_idx,
                hub_idx,
                EdgeKind::ResolvedImport { specifier: "./central".into() },
            );
        }

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.graph_label,
            Some(GraphLabel::Hub),
            "File with 5+ importers should be Hub"
        );
    }

    #[test]
    fn test_leaf_label() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        // A file with no incoming imports
        let file_path = root.join("src/leaf.rs");
        graph.add_file(file_path.clone(), "rust");

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.graph_label,
            Some(GraphLabel::Leaf),
            "File with 0 importers should be Leaf"
        );
    }

    #[test]
    fn test_bridge_label() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_path = root.join("src/bridge.rs");
        let bridge_idx = graph.add_file(file_path.clone(), "rust");

        // 2 incoming importers
        for i in 0..2 {
            let importer_path = root.join(format!("src/importer{}.rs", i));
            let importer_idx = graph.add_file(importer_path, "rust");
            graph.graph.add_edge(
                importer_idx,
                bridge_idx,
                EdgeKind::ResolvedImport { specifier: "./bridge".into() },
            );
        }

        // 3 outgoing imports
        for i in 0..3 {
            let dep_path = root.join(format!("src/dep{}.rs", i));
            let dep_idx = graph.add_file(dep_path, "rust");
            graph.graph.add_edge(
                bridge_idx,
                dep_idx,
                EdgeKind::ResolvedImport { specifier: format!("./dep{}", i) },
            );
        }

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.graph_label,
            Some(GraphLabel::Bridge),
            "File with 2 importers and 3 imports should be Bridge"
        );
    }

    #[test]
    fn test_exports_not_truncated() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_path = root.join("src/big_exports.ts");
        let file_idx = graph.add_file(file_path.clone(), "typescript");

        // Add 20 exported symbols
        for i in 0..20 {
            graph.add_symbol(
                file_idx,
                make_symbol(
                    &format!("ExportedFn{}", i),
                    SymbolKind::Function,
                    SymbolVisibility::Private, // TS uses is_exported
                    true, // is_exported = true
                ),
            );
        }

        let summary = file_summary(&graph, &root, &file_path).unwrap();
        assert_eq!(
            summary.exports.len(),
            20,
            "All 20 exports should be listed (no truncation)"
        );
    }

    #[test]
    fn test_line_count() {
        // Create a temp file with known number of lines
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        // Write 10 lines
        for i in 0..10 {
            writeln!(tmp, "line {}", i).unwrap();
        }
        let tmp_path = tmp.path().to_path_buf();

        let mut graph = CodeGraph::new();
        // Use the temp dir as "root" — but we'll call file_summary with absolute path
        let root = PathBuf::from("/tmp");
        graph.add_file(tmp_path.clone(), "rust");

        let summary = file_summary(&graph, &root, &tmp_path).unwrap();
        assert_eq!(
            summary.line_count,
            10,
            "Should count 10 lines in the temp file"
        );
    }
}

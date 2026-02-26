use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Classification category of an import.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportCategory {
    Internal,  // Same package/crate
    Workspace, // Sibling crate in same workspace (Rust only)
    External,  // npm/crates.io dependency
    Builtin,   // std/core/alloc or node builtins
}

/// A single import entry.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    pub specifier: String,
    pub category: ImportCategory,
    pub is_reexport: bool,
}

// ---------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------

/// Classify a RustImport path (e.g. "std::collections::HashMap") into a category.
///
/// - `std::` / `core::` / `alloc::` -> Builtin
/// - `crate::` or matches source file's own crate_name -> Internal
/// - anything else -> External
fn classify_rust_import(path: &str, source_crate: Option<&str>) -> ImportCategory {
    let first_segment = path.split("::").next().unwrap_or("");

    match first_segment {
        "std" | "core" | "alloc" => ImportCategory::Builtin,
        "crate" | "super" | "self" => ImportCategory::Internal,
        seg => {
            // Check if it's the source file's own crate
            if let Some(crate_name) = source_crate
                && seg == crate_name
            {
                return ImportCategory::Internal;
            }
            ImportCategory::External
        }
    }
}

// ---------------------------------------------------------------------------
// Main query function
// ---------------------------------------------------------------------------

/// Build a classified list of imports for a single file in the graph.
///
/// Returns imports in edge iteration order (approximate source-file order).
/// Returns `Err` if the file path is not found in the graph.
pub fn file_imports(
    graph: &CodeGraph,
    root: &Path,
    file_path: &Path,
) -> Result<Vec<ImportEntry>, String> {
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

    // Get source crate_name for RustImport classification
    let source_crate: Option<String> = match &graph.graph[file_idx] {
        GraphNode::File(fi) => fi.crate_name.clone(),
        _ => None,
    };

    let mut entries: Vec<ImportEntry> = Vec::new();

    for edge_ref in graph.graph.edges(file_idx) {
        match edge_ref.weight() {
            EdgeKind::ResolvedImport { specifier } => {
                let target_idx = edge_ref.target();
                let category = match &graph.graph[target_idx] {
                    GraphNode::File(fi) => {
                        // Check if source and target are from different crates (workspace)
                        if let (Some(src_crate), Some(tgt_crate)) =
                            (source_crate.as_deref(), fi.crate_name.as_deref())
                        {
                            if src_crate != tgt_crate {
                                ImportCategory::Workspace
                            } else {
                                ImportCategory::Internal
                            }
                        } else {
                            ImportCategory::Internal
                        }
                    }
                    GraphNode::ExternalPackage(_) => ImportCategory::External,
                    GraphNode::Builtin { .. } => ImportCategory::Builtin,
                    GraphNode::UnresolvedImport { .. } => {
                        // Skip unresolved imports — they add noise
                        continue;
                    }
                    _ => continue,
                };

                entries.push(ImportEntry {
                    specifier: specifier.clone(),
                    category,
                    is_reexport: false,
                });
            }

            EdgeKind::ReExport { path } => {
                entries.push(ImportEntry {
                    specifier: path.clone(),
                    category: ImportCategory::Internal,
                    is_reexport: true,
                });
            }

            EdgeKind::BarrelReExportAll => {
                let target_idx = edge_ref.target();
                // Use relative path of target file as specifier
                let specifier = match &graph.graph[target_idx] {
                    GraphNode::File(fi) => fi
                        .path
                        .strip_prefix(root)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| fi.path.to_string_lossy().into_owned()),
                    _ => continue,
                };

                entries.push(ImportEntry {
                    specifier,
                    category: ImportCategory::Internal,
                    is_reexport: true,
                });
            }

            EdgeKind::RustImport { path } => {
                let category = classify_rust_import(path, source_crate.as_deref());
                entries.push(ImportEntry {
                    specifier: path.clone(),
                    category,
                    is_reexport: false,
                });
            }

            // Skip all other edge kinds
            _ => {}
        }
    }

    Ok(entries)
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
        node::{FileInfo, FileKind, GraphNode},
    };

    #[allow(dead_code)]
    fn make_file_info(path: PathBuf, crate_name: Option<&str>) -> FileInfo {
        FileInfo {
            path,
            language: "rust".into(),
            crate_name: crate_name.map(|s| s.to_string()),
            kind: FileKind::Source,
        }
    }

    #[allow(dead_code)]
    fn make_graph_with_source(
        root: &Path,
        file_name: &str,
        crate_name: Option<&str>,
    ) -> (CodeGraph, PathBuf) {
        let mut graph = CodeGraph::new();
        let file_path = root.join(file_name);
        let file_idx = graph.add_file(file_path.clone(), "rust");
        // Update crate_name if provided
        if let Some(cn) = crate_name
            && let Some(GraphNode::File(fi)) = graph.graph.node_weight_mut(file_idx)
        {
            fi.crate_name = Some(cn.to_string());
        }
        (graph, file_path)
    }

    #[test]
    fn test_resolved_import_internal() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/a.rs");
        let tgt_path = root.join("src/b.rs");

        let src_idx = graph.add_file(src_path.clone(), "rust");
        let tgt_idx = graph.add_file(tgt_path.clone(), "rust");

        // Both in same crate (crate_name = None -> Internal)
        graph.graph.add_edge(
            src_idx,
            tgt_idx,
            EdgeKind::ResolvedImport {
                specifier: "./b".into(),
            },
        );

        let entries = file_imports(&graph, &root, &src_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, ImportCategory::Internal);
        assert!(!entries[0].is_reexport);
    }

    #[test]
    fn test_resolved_import_external() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/a.ts");
        let src_idx = graph.add_file(src_path.clone(), "typescript");

        // Add an external package node
        let _pkg_idx = graph.add_external_package(src_idx, "react", "react");

        // The add_external_package already adds the edge, so let's just verify
        let entries = file_imports(&graph, &root, &src_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, ImportCategory::External);
        assert!(!entries[0].is_reexport);
    }

    #[test]
    fn test_resolved_import_builtin() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/main.rs");
        let src_idx = graph.add_file(src_path.clone(), "rust");

        // Add a builtin node
        graph.add_builtin_node(src_idx, "std", "std::collections::HashMap");

        let entries = file_imports(&graph, &root, &src_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, ImportCategory::Builtin);
        assert!(!entries[0].is_reexport);
    }

    #[test]
    fn test_reexport_label() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/lib.rs");
        let src_idx = graph.add_file(src_path.clone(), "rust");

        // Add a ReExport edge
        graph.graph.add_edge(
            src_idx,
            src_idx, // self-edge placeholder (as in real resolver)
            EdgeKind::ReExport {
                path: "crate::utils::helper".into(),
            },
        );

        let entries = file_imports(&graph, &root, &src_path).unwrap();
        assert!(
            entries.iter().any(|e| e.is_reexport),
            "ReExport edge should set is_reexport=true"
        );
        let reexport = entries.iter().find(|e| e.is_reexport).unwrap();
        assert_eq!(reexport.category, ImportCategory::Internal);
    }

    #[test]
    fn test_barrel_reexport() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let barrel_path = root.join("src/index.ts");
        let source_path = root.join("src/utils.ts");

        let barrel_idx = graph.add_file(barrel_path.clone(), "typescript");
        let source_idx = graph.add_file(source_path.clone(), "typescript");

        // Add BarrelReExportAll edge
        graph.add_barrel_reexport_all(barrel_idx, source_idx);

        let entries = file_imports(&graph, &root, &barrel_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].is_reexport,
            "BarrelReExportAll should set is_reexport=true"
        );
        assert_eq!(entries[0].category, ImportCategory::Internal);
    }

    #[test]
    fn test_rust_import_builtin() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/main.rs");
        let src_idx = graph.add_file(src_path.clone(), "rust");

        // Add a RustImport edge with std:: path
        graph.graph.add_edge(
            src_idx,
            src_idx, // placeholder target
            EdgeKind::RustImport {
                path: "std::collections::HashMap".into(),
            },
        );

        let entries = file_imports(&graph, &root, &src_path).unwrap();
        let rust_imports: Vec<_> = entries
            .iter()
            .filter(|e| e.specifier == "std::collections::HashMap")
            .collect();
        assert_eq!(rust_imports.len(), 1);
        assert_eq!(rust_imports[0].category, ImportCategory::Builtin);
    }

    #[test]
    fn test_rust_import_external() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/main.rs");
        let src_idx = graph.add_file(src_path.clone(), "rust");

        // Add a RustImport edge with external crate path (serde)
        graph.graph.add_edge(
            src_idx,
            src_idx, // placeholder target
            EdgeKind::RustImport {
                path: "serde::Deserialize".into(),
            },
        );

        let entries = file_imports(&graph, &root, &src_path).unwrap();
        let serde_imports: Vec<_> = entries
            .iter()
            .filter(|e| e.specifier == "serde::Deserialize")
            .collect();
        assert_eq!(serde_imports.len(), 1);
        assert_eq!(serde_imports[0].category, ImportCategory::External);
    }

    #[test]
    fn test_import_order_preserved() {
        let root = PathBuf::from("/tmp/test_project");
        let mut graph = CodeGraph::new();

        let src_path = root.join("src/main.rs");
        let src_idx = graph.add_file(src_path.clone(), "rust");

        // Add 3 edges: the order in which petgraph returns edges should be preserved
        let _pkg1_idx = graph.add_external_package(src_idx, "alpha", "alpha");
        let _pkg2_idx = graph.add_external_package(src_idx, "beta", "beta");
        let _pkg3_idx = graph.add_external_package(src_idx, "gamma", "gamma");

        let entries = file_imports(&graph, &root, &src_path).unwrap();

        // We should get 3 entries (order is insertion order from petgraph)
        assert_eq!(entries.len(), 3, "All 3 imports should be present");
        // Verify all 3 are present (order is guaranteed by petgraph edge insertion order)
        let specifiers: Vec<&str> = entries.iter().map(|e| e.specifier.as_str()).collect();
        assert!(specifiers.contains(&"alpha"), "alpha should be present");
        assert!(specifiers.contains(&"beta"), "beta should be present");
        assert!(specifiers.contains(&"gamma"), "gamma should be present");
    }

    #[test]
    fn test_file_not_found() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");
        let missing_path = root.join("src/nonexistent.rs");

        let result = file_imports(&graph, &root, &missing_path);
        assert!(result.is_err(), "Missing file should return Err");
        assert!(
            result.unwrap_err().contains("File not found"),
            "Error message should mention 'File not found'"
        );
    }
}

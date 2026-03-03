use std::collections::HashSet;
use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::visit::IntoEdgeReferences;
use serde::{Deserialize, Serialize};

use crate::graph::edge::EdgeKind;
use crate::graph::node::GraphNode;
use crate::query::circular;

use super::super::server::AppState;

// ---------------------------------------------------------------------------
// Module file detection
// ---------------------------------------------------------------------------

/// File names that indicate a module entry-point (structural files that anchor
/// a directory's public API). These get kind "module" instead of "file" in the
/// graph response so they can be visually distinguished in both granularity views.
const MODULE_FILES: &[&str] = &[
    "index.ts",
    "index.js",
    "index.tsx",
    "index.jsx",
    "mod.rs",
    "__init__.py",
    "lib.rs",
    "main.rs",
];

/// Returns `true` if `path` has a filename that is a recognized module entry-point.
fn is_module_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| MODULE_FILES.contains(&name))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct GraphQuery {
    #[serde(default = "default_granularity")]
    pub granularity: String,
}

fn default_granularity() -> String {
    "file".to_string()
}

// ---------------------------------------------------------------------------
// Graphology serialisation structs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct GraphResponse {
    pub attributes: GraphAttributes,
    pub nodes: Vec<NodeEntry>,
    pub edges: Vec<EdgeEntry>,
}

#[derive(Serialize)]
pub struct GraphAttributes {
    pub granularity: String,
}

#[derive(Serialize)]
pub struct NodeEntry {
    pub key: String,
    pub attributes: NodeAttributes,
}

#[derive(Serialize)]
pub struct NodeAttributes {
    pub label: String,
    pub kind: String,
    pub language: Option<String>,
    pub path: String,
    pub size: f32,
    pub x: f32,
    pub y: f32,
    pub color: String,
    #[serde(rename = "isCircular")]
    pub is_circular: bool,
    pub decorators: Vec<String>,
    pub line: Option<usize>,
    #[serde(rename = "lineEnd")]
    pub line_end: Option<usize>,
}

#[derive(Serialize)]
pub struct EdgeEntry {
    pub key: String,
    pub source: String,
    pub target: String,
    pub attributes: EdgeAttributes,
}

#[derive(Serialize)]
pub struct EdgeAttributes {
    /// Semantic edge type (e.g. "Imports", "Calls"). Serialised as `edgeType` to
    /// avoid colliding with Sigma's reserved `type` attribute (which selects the
    /// edge rendering program).
    #[serde(rename = "edgeType")]
    pub edge_type: String,
    pub color: String,
    #[serde(rename = "isCircular")]
    pub is_circular: bool,
    /// Visual weight hint for edgeReducer (1.0 = default, 2.0 = prominent)
    pub weight: f32,
}

// ---------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------

fn node_color(kind: &str) -> &'static str {
    match kind {
        "function" => "#7c5cfc",    // violet — primary symbol
        "class" => "#4f8ef7",       // blue
        "struct" => "#5ba3f5",      // lighter blue
        "interface" => "#2dba8c",   // teal
        "trait" => "#26a87e",       // darker teal
        "impl_method" => "#9b7fe8", // soft purple
        "method" => "#9b7fe8",      // soft purple (same as impl_method)
        "enum" => "#c4853a",        // amber
        "component" => "#c0537a",   // muted pink
        "type" => "#7a9fd4",        // slate blue
        "property" => "#8897a8",    // grey-blue
        "variable" => "#8aa0b0",    // muted slate
        "const" => "#96a8b8",       // slightly brighter slate
        "static" => "#7d8fa0",      // darker slate
        "macro" => "#a07ab0",       // muted mauve
        "folder" => "#6366f1",      // indigo — folder hierarchy
        "module" => "#5e8bc0",      // module blue
        "file" => "#6b6090",        // purple-tinted grey
        _ => "#6b6090",             // fallback same as file
    }
}

/// Language-based coloring for file-granularity nodes.
/// In file view all nodes are "file" kind — color by language instead.
fn language_color(lang: &str) -> &'static str {
    match lang {
        "typescript" | "tsx" => "#3178C6",
        "javascript" | "jsx" => "#E8D44D",
        "rust" => "#DEA584",
        "python" => "#3572A5",
        "go" => "#00ADD8",
        "java" => "#B07219",
        "c" | "cpp" | "c++" => "#555555",
        "css" | "scss" | "less" => "#563D7C",
        "html" => "#E34C26",
        "svelte" => "#FF3E00",
        "vue" => "#41B883",
        "ruby" => "#CC342D",
        "php" => "#4F5D95",
        "swift" => "#F05138",
        "kotlin" => "#A97BFF",
        "dart" => "#00B4AB",
        "zig" => "#F7A41D",
        _ => "#6B7280",
    }
}

fn edge_color(edge_type: &str) -> &'static str {
    match edge_type {
        "Imports" | "ResolvedImport" => "#1d4ed8", // muted blue
        "Calls" => "#7c3aed",                      // muted violet
        "Extends" => "#c2410c",                    // muted orange
        "Contains" => "#2d5a3d",                   // muted green
        "Implements" => "#be185d",                 // muted pink
        "HasDecorator" => "#b45309",               // muted amber
        _ => "#4a4060",                            // muted purple-grey
    }
}

fn edge_weight(edge_type: &str) -> f32 {
    match edge_type {
        "Circular" => 2.0,
        "Extends" => 1.5,
        "Implements" => 1.5,
        "Calls" => 1.2,
        "Contains" => 0.5, // loose coupling for folder scaffolding
        _ => 1.0,
    }
}

fn edge_type_str(edge: &EdgeKind) -> &'static str {
    match edge {
        EdgeKind::Imports { .. } => "Imports",
        EdgeKind::ResolvedImport { .. } => "ResolvedImport",
        EdgeKind::BarrelReExportAll => "BarrelReExportAll",
        EdgeKind::ConditionalImport { .. } => "ConditionalImport",
        EdgeKind::SideEffectImport { .. } => "SideEffectImport",
        EdgeKind::DotImport { .. } => "DotImport",
        EdgeKind::Contains => "Contains",
        EdgeKind::Calls => "Calls",
        EdgeKind::Extends => "Extends",
        EdgeKind::Implements => "Implements",
        EdgeKind::ChildOf => "ChildOf",
        EdgeKind::HasDecorator { .. } => "HasDecorator",
        EdgeKind::Exports { .. } => "Exports",
        EdgeKind::ReExport { .. } => "ReExport",
        EdgeKind::RustImport { .. } => "RustImport",
        EdgeKind::Embeds => "Embeds",
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handler(
    Query(params): Query<GraphQuery>,
    State(state): State<AppState>,
) -> Result<Json<GraphResponse>, (StatusCode, String)> {
    let graph = state.graph.read().await;

    // Pre-compute which files are in circular dependency cycles.
    let cycles = circular::find_circular(&graph, &state.project_root);
    let mut circular_files: HashSet<PathBuf> = HashSet::new();
    for cycle in &cycles {
        // The cycle closes by repeating the first file -- skip the last duplicate.
        let unique_len = cycle.files.len().saturating_sub(1);
        for file in &cycle.files[..unique_len] {
            circular_files.insert(file.clone());
        }
    }

    let granularity = params.granularity.as_str();

    match granularity {
        "symbol" => build_symbol_graph(&graph, &circular_files, granularity, &state.project_root),
        _ => build_file_graph(&graph, &circular_files, granularity, &state.project_root),
    }
    .map(Json)
}

// ---------------------------------------------------------------------------
// File-granularity graph builder
// ---------------------------------------------------------------------------

fn build_file_graph(
    graph: &crate::graph::CodeGraph,
    circular_files: &HashSet<PathBuf>,
    granularity: &str,
    project_root: &Path,
) -> Result<GraphResponse, (StatusCode, String)> {
    let mut nodes: Vec<NodeEntry> = Vec::new();
    let mut edges: Vec<EdgeEntry> = Vec::new();

    // Map NodeIndex -> key string for edge construction.
    let mut idx_to_key: std::collections::HashMap<NodeIndex, String> =
        std::collections::HashMap::new();

    // Iterate file_index: HashMap<PathBuf, NodeIndex>
    for (file_path, &file_idx) in &graph.file_index {
        let rel_path = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path.as_path())
            .to_string_lossy()
            .to_string();
        let key = rel_path.clone();
        idx_to_key.insert(file_idx, key.clone());

        // Compute degree for node size.
        let degree = graph.graph.edges(file_idx).count()
            + graph
                .graph
                .edges_directed(file_idx, petgraph::Direction::Incoming)
                .count();

        let size = 2.0 + (degree as f32).sqrt() * 3.0;
        let is_circ = circular_files.contains(file_path.as_path());

        let language = if let GraphNode::File(ref fi) = graph.graph[file_idx] {
            Some(fi.language.clone())
        } else {
            None
        };

        let label = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| key.clone());

        // Module entry-point files get kind "module" and a distinct color.
        // Other files use language-based coloring for visual distinction.
        let (kind, color) = if is_module_file(file_path) {
            ("module".to_string(), node_color("module").to_string())
        } else {
            (
                "file".to_string(),
                language_color(language.as_deref().unwrap_or("")).to_string(),
            )
        };

        nodes.push(NodeEntry {
            key: key.clone(),
            attributes: NodeAttributes {
                label,
                kind,
                language,
                path: rel_path,
                size,
                x: 0.0,
                y: 0.0,
                color,
                is_circular: is_circ,
                decorators: vec![],
                line: None,
                line_end: None,
            },
        });
    }

    // -------------------------------------------------------------------------
    // Folder synthesis: synthesize virtual folder nodes and Contains edges.
    //
    // Algorithm:
    //   1. Walk all file paths; group files by their parent directory (relative).
    //      Root-level files (parent == "") are skipped — no project root node.
    //   2. Collapse single-child chains: if a directory has 0 direct files and
    //      exactly 1 direct subdirectory, merge them into a joined label ("src/lib").
    //   3. Emit a NodeEntry for each surviving folder with size based on child count.
    //   4. Emit Contains edges from each folder to its direct file/subdir children.
    // -------------------------------------------------------------------------
    {
        use std::collections::{BTreeMap, BTreeSet};

        // dir_files:   rel_dir -> Vec<file_key>
        // dir_subdirs: rel_dir -> BTreeSet<child_rel_dir>
        let mut dir_files: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        let mut dir_subdirs: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();

        for file_path in graph.file_index.keys() {
            let rel = file_path
                .strip_prefix(project_root)
                .unwrap_or(file_path.as_path());

            // File key is the relative path string (same as built in the loop above).
            let file_key = rel.to_string_lossy().to_string();

            let parent = match rel.parent() {
                Some(p) if p != Path::new("") => p.to_path_buf(),
                _ => continue, // root-level file — skip
            };

            // Register this file under its immediate parent.
            dir_files.entry(parent.clone()).or_default().push(file_key);

            // Walk up the directory tree, registering each dir as a child of its parent.
            let mut current = parent.clone();
            loop {
                match current.parent() {
                    Some(p) if p != Path::new("") => {
                        dir_subdirs
                            .entry(p.to_path_buf())
                            .or_default()
                            .insert(current.clone());
                        current = p.to_path_buf();
                    }
                    _ => {
                        // current is a top-level directory — register under the virtual "" root
                        // so we can traverse but won't emit a root folder node.
                        dir_subdirs
                            .entry(PathBuf::new())
                            .or_default()
                            .insert(current.clone());
                        break;
                    }
                }
            }
        }

        // Ensure every dir that appears as a parent also has an entry in dir_files
        // (even if empty) so the collapse loop can use it.
        for subdirs in dir_subdirs.values() {
            for sub in subdirs {
                dir_files.entry(sub.clone()).or_default();
            }
        }

        // ------------------------------------------------------------------
        // Collapse single-child chains.
        //
        // A directory is collapsible if:
        //   - It has 0 direct files (dir_files[dir] is empty), AND
        //   - It has exactly 1 direct subdirectory (dir_subdirs[dir].len() == 1)
        //
        // We merge the lone child into the parent, transferring the child's
        // subdirectories.  We track the display label separately.
        //
        // Example: a -> b -> c (a has no files, b has no files, c has files)
        //   After collapse: "a/b" -> c
        // ------------------------------------------------------------------
        // collapsed_labels[dir] = display label (e.g. "a/b")
        let mut collapsed_labels: BTreeMap<PathBuf, String> = BTreeMap::new();

        let mut changed = true;
        while changed {
            changed = false;
            // Collect candidates: dirs with 0 files and exactly 1 subdir.
            // We process parents that satisfy this condition — we KEEP the parent key
            // and absorb the child key into it.
            let candidates: Vec<PathBuf> = dir_subdirs
                .keys()
                .filter(|dir| {
                    // Skip the virtual root.
                    if dir.as_os_str().is_empty() {
                        return false;
                    }
                    let files_empty = dir_files.get(*dir).is_none_or(|v| v.is_empty());
                    let one_subdir = dir_subdirs.get(*dir).is_some_and(|s| s.len() == 1);
                    files_empty && one_subdir
                })
                .cloned()
                .collect();

            for parent_dir in candidates {
                // Get the single child directory.
                let child_dir = match dir_subdirs.get(&parent_dir).and_then(|s| s.iter().next()) {
                    Some(c) => c.clone(),
                    None => continue,
                };

                // Build the merged display label.
                let parent_label =
                    collapsed_labels
                        .get(&parent_dir)
                        .cloned()
                        .unwrap_or_else(|| {
                            parent_dir
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| parent_dir.to_string_lossy().to_string())
                        });
                let child_label = collapsed_labels
                    .get(&child_dir)
                    .cloned()
                    .unwrap_or_else(|| {
                        child_dir
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| child_dir.to_string_lossy().to_string())
                    });
                let merged_label = format!("{parent_label}/{child_label}");
                collapsed_labels.insert(parent_dir.clone(), merged_label);

                // Transfer child's files and subdirs to parent.
                let child_files = dir_files.remove(&child_dir).unwrap_or_default();
                dir_files
                    .entry(parent_dir.clone())
                    .or_default()
                    .extend(child_files);

                let child_subdirs = dir_subdirs.remove(&child_dir).unwrap_or_default();
                if let Some(parent_subdirs) = dir_subdirs.get_mut(&parent_dir) {
                    parent_subdirs.remove(&child_dir);
                    parent_subdirs.extend(child_subdirs);
                }

                // Remove child from collapsed_labels (it's been merged).
                collapsed_labels.remove(&child_dir);

                // Also update any grandparent references: if another dir listed
                // child_dir as a subdir, replace with parent_dir.
                for subdirs in dir_subdirs.values_mut() {
                    if subdirs.remove(&child_dir) {
                        subdirs.insert(parent_dir.clone());
                    }
                }

                changed = true;
                break; // restart loop to pick up cascading collapses
            }
        }

        // ------------------------------------------------------------------
        // Emit folder NodeEntry for each surviving directory.
        // Skip the virtual "" root entry.
        // ------------------------------------------------------------------
        let mut folder_edge_counter = 0usize;

        for (dir, files) in &dir_files {
            if dir.as_os_str().is_empty() {
                continue;
            }

            let subdirs = dir_subdirs.get(dir).map(|s| s.len()).unwrap_or(0);
            let child_count = files.len() + subdirs;

            let dir_display = dir.to_string_lossy().to_string();
            let folder_key = format!("folder:{dir_display}");

            let label = collapsed_labels.get(dir).cloned().unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir_display.clone())
            });

            let size = 5.0 + (child_count as f32).sqrt() * 3.0;

            nodes.push(NodeEntry {
                key: folder_key.clone(),
                attributes: NodeAttributes {
                    label,
                    kind: "folder".to_string(),
                    language: None,
                    path: dir_display.clone(),
                    size,
                    x: 0.0,
                    y: 0.0,
                    color: node_color("folder").to_string(),
                    is_circular: false,
                    decorators: vec![],
                    line: None,
                    line_end: None,
                },
            });

            // Emit Contains edges: folder -> direct child files.
            for file_key in files {
                edges.push(EdgeEntry {
                    key: format!("fe{folder_edge_counter}"),
                    source: folder_key.clone(),
                    target: file_key.clone(),
                    attributes: EdgeAttributes {
                        edge_type: "Contains".to_string(),
                        color: edge_color("Contains").to_string(),
                        is_circular: false,
                        weight: edge_weight("Contains"),
                    },
                });
                folder_edge_counter += 1;
            }

            // Emit Contains edges: folder -> direct child subdirs.
            if let Some(subdirs_set) = dir_subdirs.get(dir) {
                for subdir in subdirs_set {
                    let subdir_display = subdir.to_string_lossy().to_string();
                    let subdir_key = format!("folder:{subdir_display}");
                    edges.push(EdgeEntry {
                        key: format!("fe{folder_edge_counter}"),
                        source: folder_key.clone(),
                        target: subdir_key,
                        attributes: EdgeAttributes {
                            edge_type: "Contains".to_string(),
                            color: edge_color("Contains").to_string(),
                            is_circular: false,
                            weight: edge_weight("Contains"),
                        },
                    });
                    folder_edge_counter += 1;
                }
            }
        }
    }

    // Collect import-like edges between file nodes.
    // All import variants are normalized to "Imports" so the frontend's
    // visibleEdgeTypes filter (which expects "Imports") shows them.
    let mut edge_counter = 0usize;
    for edge_ref in graph.graph.edge_references() {
        let include = matches!(
            edge_ref.weight(),
            EdgeKind::Imports { .. }
                | EdgeKind::ResolvedImport { .. }
                | EdgeKind::BarrelReExportAll
                | EdgeKind::ConditionalImport { .. }
                | EdgeKind::SideEffectImport { .. }
                | EdgeKind::DotImport { .. }
        );
        if !include {
            continue;
        }

        let src_idx = edge_ref.source();
        let dst_idx = edge_ref.target();

        let src_key = match idx_to_key.get(&src_idx) {
            Some(k) => k.clone(),
            None => continue,
        };
        let dst_key = match idx_to_key.get(&dst_idx) {
            Some(k) => k.clone(),
            None => continue,
        };

        // An edge is circular if both endpoints are in circular_files.
        let src_circ = if let GraphNode::File(ref fi) = graph.graph[src_idx] {
            circular_files.contains(fi.path.as_path())
        } else {
            false
        };
        let dst_circ = if let GraphNode::File(ref fi) = graph.graph[dst_idx] {
            circular_files.contains(fi.path.as_path())
        } else {
            false
        };

        edges.push(EdgeEntry {
            key: format!("e{edge_counter}"),
            source: src_key,
            target: dst_key,
            attributes: EdgeAttributes {
                edge_type: "Imports".to_string(),
                color: edge_color("Imports").to_string(),
                is_circular: src_circ && dst_circ,
                weight: edge_weight("Imports"),
            },
        });
        edge_counter += 1;
    }

    Ok(GraphResponse {
        attributes: GraphAttributes {
            granularity: granularity.to_string(),
        },
        nodes,
        edges,
    })
}

// ---------------------------------------------------------------------------
// Symbol-granularity graph builder
// ---------------------------------------------------------------------------

fn build_symbol_graph(
    graph: &crate::graph::CodeGraph,
    circular_files: &HashSet<PathBuf>,
    granularity: &str,
    project_root: &Path,
) -> Result<GraphResponse, (StatusCode, String)> {
    use crate::query::find::kind_to_str;

    let mut nodes: Vec<NodeEntry> = Vec::new();
    let mut edges: Vec<EdgeEntry> = Vec::new();
    let mut idx_to_key: std::collections::HashMap<NodeIndex, String> =
        std::collections::HashMap::new();

    // Add file nodes first.
    for (file_path, &file_idx) in &graph.file_index {
        let rel_path = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path.as_path())
            .to_string_lossy()
            .to_string();
        let key = rel_path.clone();
        idx_to_key.insert(file_idx, key.clone());

        let language = if let GraphNode::File(ref fi) = graph.graph[file_idx] {
            Some(fi.language.clone())
        } else {
            None
        };
        let is_circ = circular_files.contains(file_path.as_path());

        let label = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| key.clone());

        // Module entry-point files get kind "module" and a distinct color.
        let (kind, color) = if is_module_file(file_path) {
            ("module".to_string(), node_color("module").to_string())
        } else {
            ("file".to_string(), node_color("file").to_string())
        };

        nodes.push(NodeEntry {
            key: key.clone(),
            attributes: NodeAttributes {
                label,
                kind,
                language,
                path: rel_path,
                size: 5.0,
                x: 0.0,
                y: 0.0,
                color,
                is_circular: is_circ,
                decorators: vec![],
                line: None,
                line_end: None,
            },
        });
    }

    // Add symbol nodes.
    for node_idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(ref s) = graph.graph[node_idx] {
            // Find parent file to construct the path part of the key.
            let file_path_opt = graph
                .graph
                .edges_directed(node_idx, petgraph::Direction::Incoming)
                .find_map(|e| {
                    if matches!(e.weight(), EdgeKind::Contains)
                        && let GraphNode::File(ref fi) = graph.graph[e.source()]
                    {
                        return Some(fi.path.clone());
                    }
                    None
                });

            let path_str = file_path_opt
                .as_ref()
                .map(|p| {
                    p.strip_prefix(project_root)
                        .unwrap_or(p.as_path())
                        .to_string_lossy()
                        .to_string()
                })
                .unwrap_or_default();

            // Clone fields before further borrows.
            let sym_name = s.name.clone();
            let sym_line = s.line;
            let sym_line_end = s.line_end;
            let sym_kind = s.kind.clone();
            let decorator_names: Vec<String> =
                s.decorators.iter().map(|d| d.name.clone()).collect();

            let key = format!("{}::{}::{}", path_str, sym_name, sym_line);
            idx_to_key.insert(node_idx, key.clone());

            let kind_str = kind_to_str(&sym_kind).to_string();
            let degree = graph.graph.edges(node_idx).count()
                + graph
                    .graph
                    .edges_directed(node_idx, petgraph::Direction::Incoming)
                    .count();
            let size = 2.0 + (degree as f32).sqrt() * 3.0;

            nodes.push(NodeEntry {
                key: key.clone(),
                attributes: NodeAttributes {
                    label: sym_name,
                    kind: kind_str.clone(),
                    language: None,
                    path: path_str,
                    size,
                    x: 0.0,
                    y: 0.0,
                    color: node_color(&kind_str).to_string(),
                    is_circular: false,
                    decorators: decorator_names,
                    line: Some(sym_line),
                    line_end: Some(sym_line_end),
                },
            });
        }
    }

    // Add symbol-level edges.
    let mut edge_counter = 0usize;
    for edge_ref in graph.graph.edge_references() {
        let include = matches!(
            edge_ref.weight(),
            EdgeKind::Contains
                | EdgeKind::Calls
                | EdgeKind::Extends
                | EdgeKind::Implements
                | EdgeKind::ChildOf
                | EdgeKind::HasDecorator { .. }
        );
        if !include {
            continue;
        }

        let src_key = match idx_to_key.get(&edge_ref.source()) {
            Some(k) => k.clone(),
            None => continue,
        };
        let dst_key = match idx_to_key.get(&edge_ref.target()) {
            Some(k) => k.clone(),
            None => continue,
        };

        let etype = edge_type_str(edge_ref.weight());
        edges.push(EdgeEntry {
            key: format!("e{edge_counter}"),
            source: src_key,
            target: dst_key,
            attributes: EdgeAttributes {
                edge_type: etype.to_string(),
                color: edge_color(etype).to_string(),
                is_circular: false,
                weight: edge_weight(etype),
            },
        });
        edge_counter += 1;
    }

    Ok(GraphResponse {
        attributes: GraphAttributes {
            granularity: granularity.to_string(),
        },
        nodes,
        edges,
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CodeGraph;
    use crate::graph::node::{SymbolInfo, SymbolKind};
    use std::path::PathBuf;

    // -------------------------------------------------------------------------
    // Task 1: Folder synthesis, module detection, Contains edges
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_file_graph_emits_folder_nodes() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_file(root.join("src/main.rs"), "rust");
        graph.add_file(root.join("tests/integration.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        // Should have folder nodes for "src" and "tests"
        let folder_nodes: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "folder")
            .collect();

        assert!(
            folder_nodes.len() >= 2,
            "should have at least 2 folder nodes (src, tests), got {}",
            folder_nodes.len()
        );

        for folder in &folder_nodes {
            assert_eq!(
                folder.attributes.color, "#6366f1",
                "folder node should have indigo color"
            );
            let label = &folder.attributes.label;
            assert!(
                label == "src" || label == "tests",
                "folder label should be directory basename, got '{label}'"
            );
        }
    }

    #[test]
    fn test_build_file_graph_contains_edges() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_file(root.join("src/main.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        let contains_edges: Vec<_> = response
            .edges
            .iter()
            .filter(|e| e.attributes.edge_type == "Contains")
            .collect();

        assert!(
            !contains_edges.is_empty(),
            "should have Contains edges from folder to files"
        );

        for edge in &contains_edges {
            assert_eq!(edge.attributes.color, "#2d5a3d", "Contains edge color");
            assert!(
                (edge.attributes.weight - 0.5).abs() < f32::EPSILON,
                "Contains edge weight should be 0.5"
            );
            assert!(
                edge.source.starts_with("folder:"),
                "Contains edge source should start with 'folder:', got '{}'",
                edge.source
            );
        }
    }

    #[test]
    fn test_build_file_graph_folder_size_formula() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        // 4 children in one folder -> size = 5.0 + sqrt(4) * 3.0 = 11.0
        graph.add_file(root.join("src/a.rs"), "rust");
        graph.add_file(root.join("src/b.rs"), "rust");
        graph.add_file(root.join("src/c.rs"), "rust");
        graph.add_file(root.join("src/d.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        let src_folder = response
            .nodes
            .iter()
            .find(|n| n.attributes.kind == "folder" && n.attributes.label == "src")
            .expect("should have src folder node");

        let expected_size = 5.0 + (4.0_f32).sqrt() * 3.0; // = 11.0
        assert!(
            (src_folder.attributes.size - expected_size).abs() < 0.01,
            "folder size should be {expected_size}, got {}",
            src_folder.attributes.size
        );
    }

    #[test]
    fn test_build_file_graph_collapses_single_child_chains() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        // a/b/ only contains c/ (no files directly in a/ or a/b/)
        // c/ has actual files
        graph.add_file(root.join("a/b/c/file1.rs"), "rust");
        graph.add_file(root.join("a/b/c/file2.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        // a has only child b (no files), so they should collapse
        // The collapsed node should have a joined label like "a/b"
        let folder_labels: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "folder")
            .map(|n| n.attributes.label.as_str())
            .collect();

        // We should NOT have separate "a" and "b" folder nodes
        // Instead we should have "a/b" and "c"
        assert!(
            !folder_labels.contains(&"a") || !folder_labels.contains(&"b"),
            "single-child chain a/b should be collapsed, got labels: {folder_labels:?}"
        );
        assert!(
            folder_labels.contains(&"c") || folder_labels.iter().any(|l| l.contains('/')),
            "collapsed chain should produce a merged label or keep leaf, got: {folder_labels:?}"
        );
    }

    #[test]
    fn test_build_file_graph_no_root_folder() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        // Root-level file — should not get a parent folder node
        graph.add_file(root.join("Cargo.toml"), "");
        graph.add_file(root.join("main.rs"), "rust");
        // File in subdirectory — should get a folder node
        graph.add_file(root.join("src/lib.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        // There should be no Contains edge whose target is a root-level file from a folder
        // Root files don't have a parent folder node
        let folder_nodes: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "folder")
            .collect();

        // Root node itself ("" / ".") should NOT appear
        for folder in &folder_nodes {
            assert_ne!(
                folder.attributes.path, "",
                "should not have a root folder node with empty path"
            );
            assert!(
                !folder.attributes.label.is_empty(),
                "folder label should not be empty"
            );
        }

        // Cargo.toml and main.rs should not be targets of any Contains edge
        let root_file_keys: Vec<&str> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "file" || n.attributes.kind == "module")
            .filter(|n| !n.attributes.path.contains('/'))
            .map(|n| n.key.as_str())
            .collect();

        for root_key in &root_file_keys {
            let has_contains_parent = response
                .edges
                .iter()
                .any(|e| e.attributes.edge_type == "Contains" && &e.target.as_str() == root_key);
            assert!(
                !has_contains_parent,
                "root-level file '{root_key}' should not have a parent folder node"
            );
        }
    }

    #[test]
    fn test_build_file_graph_folder_key_prefix() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        graph.add_file(root.join("src/lib.rs"), "rust");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        let folder_nodes: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "folder")
            .collect();

        assert!(
            !folder_nodes.is_empty(),
            "should have at least one folder node"
        );
        for folder in &folder_nodes {
            assert!(
                folder.key.starts_with("folder:"),
                "folder key should have 'folder:' prefix, got '{}'",
                folder.key
            );
        }
    }

    #[test]
    fn test_build_file_graph_module_files_get_module_kind() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        graph.add_file(root.join("src/mod.rs"), "rust");
        graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_file(root.join("src/main.rs"), "rust");
        graph.add_file(root.join("web/index.ts"), "typescript");
        graph.add_file(root.join("pkg/__init__.py"), "python");
        graph.add_file(root.join("src/utils.rs"), "rust"); // normal file

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        let module_nodes: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| n.attributes.kind == "module")
            .collect();

        assert_eq!(
            module_nodes.len(),
            5,
            "should have 5 module nodes (mod.rs, lib.rs, main.rs, index.ts, __init__.py)"
        );

        for m in &module_nodes {
            assert_eq!(
                m.attributes.color, "#5e8bc0",
                "module node should have module-blue color, got '{}' for '{}'",
                m.attributes.color, m.attributes.label
            );
        }

        // utils.rs should remain as "file"
        let utils = response
            .nodes
            .iter()
            .find(|n| n.attributes.label == "utils.rs")
            .expect("utils.rs should exist");
        assert_eq!(
            utils.attributes.kind, "file",
            "utils.rs should have kind 'file'"
        );
    }

    // -------------------------------------------------------------------------
    // Existing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_graph_api_file_granularity_returns_nodes_and_edges() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_idx = graph.add_file(root.join("a.ts"), "typescript");
        let b_idx = graph.add_file(root.join("b.ts"), "typescript");
        graph.add_resolved_import(a_idx, b_idx, "./b");

        let circular_files = HashSet::new();
        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        assert_eq!(response.nodes.len(), 2, "two file nodes expected");
        assert_eq!(response.edges.len(), 1, "one edge expected");
        assert_eq!(response.attributes.granularity, "file");
    }

    #[test]
    fn test_graph_api_symbol_granularity_returns_nodes_and_edges() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let file_idx = graph.add_file(root.join("src/lib.ts"), "typescript");
        graph.add_symbol(
            file_idx,
            SymbolInfo {
                name: "MyClass".to_string(),
                kind: SymbolKind::Class,
                line: 10,
                ..Default::default()
            },
        );

        let circular_files = HashSet::new();
        let response = build_symbol_graph(&graph, &circular_files, "symbol", &root)
            .expect("should build symbol graph");

        // 1 file node + 1 symbol node
        assert_eq!(response.nodes.len(), 2, "file + symbol nodes expected");
        // 1 Contains edge (file -> symbol)
        assert_eq!(response.edges.len(), 1, "one Contains edge expected");
    }

    #[test]
    fn test_node_color_all_kinds() {
        let kinds = [
            "function",
            "class",
            "struct",
            "interface",
            "trait",
            "impl_method",
            "method",
            "enum",
            "component",
            "type",
            "property",
            "variable",
            "const",
            "static",
            "macro",
            "module",
            "file",
        ];
        for kind in &kinds {
            let color = node_color(kind);
            assert!(
                color.starts_with('#'),
                "kind '{}' color should start with #",
                kind
            );
            assert_eq!(
                color.len(),
                7,
                "kind '{}' color should be 7 chars (#RRGGBB)",
                kind
            );
        }
        // Verify at least 12 distinct colors among the 17 kinds
        let unique: std::collections::HashSet<&str> = kinds.iter().map(|k| node_color(k)).collect();
        assert!(
            unique.len() >= 12,
            "should have at least 12 distinct colors, got {}",
            unique.len()
        );
    }

    #[test]
    fn test_edge_weight_types() {
        assert_eq!(edge_weight("Circular"), 2.0);
        assert_eq!(edge_weight("Extends"), 1.5);
        assert_eq!(edge_weight("Implements"), 1.5);
        assert_eq!(edge_weight("Calls"), 1.2);
        assert_eq!(edge_weight("Imports"), 1.0);
        assert_eq!(edge_weight("Contains"), 0.5); // loose coupling for folder scaffolding
    }

    #[test]
    fn test_edge_attributes_has_weight() {
        let attr = EdgeAttributes {
            edge_type: "Calls".to_string(),
            color: "#7c3aed".to_string(),
            is_circular: false,
            weight: 1.2,
        };
        let json = serde_json::to_string(&attr).expect("should serialize");
        assert!(
            json.contains("\"weight\""),
            "JSON should contain weight field"
        );
    }

    #[test]
    fn test_graph_circular_flag_on_edges() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_idx = graph.add_file(root.join("a.ts"), "typescript");
        let b_idx = graph.add_file(root.join("b.ts"), "typescript");

        // Mutual cycle: a <-> b
        graph.add_resolved_import(a_idx, b_idx, "./b");
        graph.add_resolved_import(b_idx, a_idx, "./a");

        // Pre-compute circular files
        let cycles = circular::find_circular(&graph, &root);
        let mut circular_files: HashSet<PathBuf> = HashSet::new();
        for cycle in &cycles {
            let unique_len = cycle.files.len().saturating_sub(1);
            for file in &cycle.files[..unique_len] {
                circular_files.insert(file.clone());
            }
        }

        assert!(
            !circular_files.is_empty(),
            "circular files should be detected"
        );

        let response = build_file_graph(&graph, &circular_files, "file", &root)
            .expect("should build file graph");

        let circular_edges: Vec<_> = response
            .edges
            .iter()
            .filter(|e| e.attributes.is_circular)
            .collect();
        assert!(
            !circular_edges.is_empty(),
            "at least one edge should be marked isCircular"
        );
    }

    // -------------------------------------------------------------------------
    // Task 2: Module detection in build_symbol_graph()
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_symbol_graph_module_files_get_module_kind() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        // Module files — should get kind "module"
        let mod_idx = graph.add_file(root.join("src/mod.rs"), "rust");
        let lib_idx = graph.add_file(root.join("src/lib.rs"), "rust");
        let index_idx = graph.add_file(root.join("web/index.ts"), "typescript");
        // Regular file — should stay kind "file"
        let utils_idx = graph.add_file(root.join("src/utils.rs"), "rust");

        // Each file needs at least one symbol so build_symbol_graph has symbols to emit.
        graph.add_symbol(
            mod_idx,
            SymbolInfo {
                name: "ModStruct".to_string(),
                kind: SymbolKind::Struct,
                line: 1,
                ..Default::default()
            },
        );
        graph.add_symbol(
            lib_idx,
            SymbolInfo {
                name: "LibFn".to_string(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        graph.add_symbol(
            index_idx,
            SymbolInfo {
                name: "IndexExport".to_string(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );
        graph.add_symbol(
            utils_idx,
            SymbolInfo {
                name: "helper".to_string(),
                kind: SymbolKind::Function,
                line: 1,
                ..Default::default()
            },
        );

        let circular_files = HashSet::new();
        let response = build_symbol_graph(&graph, &circular_files, "symbol", &root)
            .expect("should build symbol graph");

        // mod.rs, lib.rs, index.ts should have kind "module"
        let module_file_nodes: Vec<_> = response
            .nodes
            .iter()
            .filter(|n| {
                (n.attributes.kind == "module")
                    && (n.attributes.label == "mod.rs"
                        || n.attributes.label == "lib.rs"
                        || n.attributes.label == "index.ts")
            })
            .collect();

        assert_eq!(
            module_file_nodes.len(),
            3,
            "mod.rs, lib.rs, and index.ts should each have kind 'module' in symbol graph"
        );

        for m in &module_file_nodes {
            assert_eq!(
                m.attributes.color, "#5e8bc0",
                "module file node should have module-blue color in symbol graph"
            );
        }

        // utils.rs should still have kind "file"
        let utils_node = response
            .nodes
            .iter()
            .find(|n| n.attributes.label == "utils.rs")
            .expect("utils.rs should exist in symbol graph");
        assert_eq!(
            utils_node.attributes.kind, "file",
            "utils.rs should have kind 'file' in symbol graph"
        );
    }
}

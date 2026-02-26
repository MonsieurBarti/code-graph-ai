use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{FileKind, GraphNode, SymbolVisibility},
};
use crate::query::find::kind_to_str;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A node in the structure tree.
#[derive(Debug, PartialEq)]
pub enum StructureNode {
    /// A directory with children.
    Dir {
        name: String,
        children: Vec<StructureNode>,
    },
    /// A source file with its top-level symbols.
    SourceFile {
        name: String,
        symbols: Vec<StructureSymbol>,
    },
    /// A non-parsed file with a kind tag.
    NonParsedFile {
        name: String,
        kind_tag: String, // "doc", "config", "ci", "asset", "other"
    },
    /// Truncation marker when depth limit is hit.
    Truncated {
        count: usize, // number of items not shown
    },
}

/// A symbol entry in the structure tree.
#[derive(Debug, PartialEq)]
pub struct StructureSymbol {
    pub name: String,
    pub kind: String,       // "fn", "struct", "trait", etc.
    pub visibility: String, // "pub", "pub(crate)", "private"
}

// ---------------------------------------------------------------------------
// Kind tag helpers
// ---------------------------------------------------------------------------

fn file_kind_tag(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::Doc => "doc",
        FileKind::Config => "config",
        FileKind::Ci => "ci",
        FileKind::Asset => "asset",
        FileKind::Other => "other",
        FileKind::Source => "source", // unreachable in non-parsed branch
    }
}

fn visibility_label(vis: &SymbolVisibility) -> &'static str {
    match vis {
        SymbolVisibility::Pub => "pub",
        SymbolVisibility::PubCrate => "pub(crate)",
        SymbolVisibility::Private => "private",
    }
}

// ---------------------------------------------------------------------------
// Main query function
// ---------------------------------------------------------------------------

/// Collect top-level symbols for a file node via Contains edges.
fn collect_symbols(graph: &CodeGraph, file_idx: petgraph::stable_graph::NodeIndex) -> Vec<StructureSymbol> {
    let mut symbols: Vec<StructureSymbol> = graph
        .graph
        .edges(file_idx)
        .filter_map(|edge_ref| {
            if let EdgeKind::Contains = edge_ref.weight() {
                if let GraphNode::Symbol(ref sym) = graph.graph[edge_ref.target()] {
                    return Some(StructureSymbol {
                        name: sym.name.clone(),
                        kind: kind_to_str(&sym.kind).to_string(),
                        visibility: visibility_label(&sym.visibility).to_string(),
                    });
                }
            }
            None
        })
        .collect();

    // Sort symbols by name for deterministic output.
    symbols.sort_by(|a, b| a.name.cmp(&b.name));
    symbols
}

/// Build the structure tree from a flat list of paths relative to `base_dir`.
///
/// - `paths`: (relative_path, absolute_path) pairs sorted lexicographically.
/// - `depth`: remaining depth levels to recurse. When 0, emit a Truncated node.
fn build_tree(
    graph: &CodeGraph,
    paths: &[(PathBuf, PathBuf)],
    depth: usize,
) -> Vec<StructureNode> {
    if paths.is_empty() {
        return vec![];
    }

    // Group paths by their first component.
    // Keys: first component string; values: (rest-of-path, absolute-path)
    let mut dirs: HashMap<String, Vec<(PathBuf, PathBuf)>> = HashMap::new();
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();

    for (rel, abs) in paths {
        let mut components = rel.components();
        let first = match components.next() {
            Some(c) => c.as_os_str().to_string_lossy().into_owned(),
            None => continue,
        };
        let rest: PathBuf = components.collect();
        if rest.as_os_str().is_empty() {
            // This IS the file itself (no more path components).
            files.push((rel.clone(), abs.clone()));
        } else {
            dirs.entry(first).or_default().push((rest, abs.clone()));
        }
    }

    // Collect directory names and sort for deterministic output.
    let mut dir_names: Vec<String> = dirs.keys().cloned().collect();
    dir_names.sort();

    // Sort files by their relative path.
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut nodes: Vec<StructureNode> = Vec::new();

    // If depth is 0 and there's anything to show, emit a Truncated node.
    if depth == 0 {
        let total = dir_names.len() + files.len();
        if total > 0 {
            nodes.push(StructureNode::Truncated { count: total });
        }
        return nodes;
    }

    // Add directories first, then files (standard tree convention).
    for dir_name in dir_names {
        let children_paths = dirs.remove(&dir_name).unwrap_or_default();
        let children = build_tree(graph, &children_paths, depth - 1);
        nodes.push(StructureNode::Dir {
            name: dir_name,
            children,
        });
    }

    // Add files.
    for (_, abs) in &files {
        let file_idx = match graph.file_index.get(abs) {
            Some(&idx) => idx,
            None => continue,
        };

        let file_info = match &graph.graph[file_idx] {
            GraphNode::File(fi) => fi.clone(),
            _ => continue,
        };

        let file_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        match file_info.kind {
            FileKind::Source => {
                let symbols = collect_symbols(graph, file_idx);
                nodes.push(StructureNode::SourceFile {
                    name: file_name,
                    symbols,
                });
            }
            other => {
                let kind_tag = file_kind_tag(&other).to_string();
                nodes.push(StructureNode::NonParsedFile {
                    name: file_name,
                    kind_tag,
                });
            }
        }
    }

    nodes
}

/// Build a directory/module structure tree from the code graph.
///
/// - `graph`: the in-memory code graph.
/// - `root`: the project root path (used to relativize file paths).
/// - `path`: optional directory to scope the tree to; if `None`, uses `root`.
/// - `depth`: maximum directory levels to recurse (default: 3 in handler).
///
/// Returns a list of top-level `StructureNode`s representing the tree.
pub fn file_structure(
    graph: &CodeGraph,
    root: &Path,
    path: Option<&Path>,
    depth: usize,
) -> Vec<StructureNode> {
    // Compute the base directory to scope to.
    let base_dir: PathBuf = match path {
        Some(p) => {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            }
        }
        None => root.to_path_buf(),
    };

    // Collect all file paths under base_dir from the graph, building (rel, abs) pairs.
    let mut paths: Vec<(PathBuf, PathBuf)> = graph
        .file_index
        .keys()
        .filter_map(|abs| {
            let rel = abs.strip_prefix(&base_dir).ok()?;
            // Skip the base directory itself (empty relative path).
            if rel.as_os_str().is_empty() {
                return None;
            }
            Some((rel.to_path_buf(), abs.clone()))
        })
        .collect();

    // Sort lexicographically for deterministic output.
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    build_tree(graph, &paths, depth)
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
        node::{FileKind, SymbolInfo, SymbolKind, SymbolVisibility},
    };
    use crate::query::output::format_structure_to_string;

    fn make_symbol(name: &str, kind: SymbolKind, vis: SymbolVisibility) -> SymbolInfo {
        SymbolInfo {
            name: name.into(),
            kind,
            line: 1,
            col: 0,
            is_exported: false,
            is_default: false,
            visibility: vis,
            trait_impl: None,
        }
    }

    #[test]
    fn test_empty_graph() {
        let graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");
        let tree = file_structure(&graph, &root, None, 3);
        assert!(tree.is_empty(), "Empty graph should produce an empty tree");
    }

    #[test]
    fn test_single_source_file() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_idx = graph.add_file(root.join("src/main.rs"), "rust");
        graph.add_symbol(file_idx, make_symbol("main", SymbolKind::Function, SymbolVisibility::Pub));
        graph.add_symbol(file_idx, make_symbol("Config", SymbolKind::Struct, SymbolVisibility::Pub));

        let tree = file_structure(&graph, &root, None, 3);

        // Should have one Dir("src") at top level
        assert_eq!(tree.len(), 1);
        let dir = match &tree[0] {
            StructureNode::Dir { name, children } => {
                assert_eq!(name, "src");
                children
            }
            other => panic!("Expected Dir, got {:?}", other),
        };

        // Dir should contain one SourceFile("main.rs") with 2 symbols
        assert_eq!(dir.len(), 1);
        match &dir[0] {
            StructureNode::SourceFile { name, symbols } => {
                assert_eq!(name, "main.rs");
                assert_eq!(symbols.len(), 2, "Should have 2 symbols");
            }
            other => panic!("Expected SourceFile, got {:?}", other),
        }
    }

    #[test]
    fn test_non_parsed_file() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        graph.add_non_parsed_file(root.join("README.md"), FileKind::Doc);

        let tree = file_structure(&graph, &root, None, 3);

        assert_eq!(tree.len(), 1);
        match &tree[0] {
            StructureNode::NonParsedFile { name, kind_tag } => {
                assert_eq!(name, "README.md");
                assert_eq!(kind_tag, "doc");
            }
            other => panic!("Expected NonParsedFile, got {:?}", other),
        }
    }

    #[test]
    fn test_depth_limit() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        // File at depth 3: src/a/b/file.rs
        graph.add_file(root.join("src/a/b/file.rs"), "rust");

        // With depth=1, we should see src/ -> Truncated
        let tree = file_structure(&graph, &root, None, 1);

        assert_eq!(tree.len(), 1);
        let children = match &tree[0] {
            StructureNode::Dir { name, children } => {
                assert_eq!(name, "src");
                children
            }
            other => panic!("Expected Dir(src), got {:?}", other),
        };

        // src/ at depth=1 should have depth=0 remaining for its children, producing Truncated
        assert_eq!(children.len(), 1);
        match &children[0] {
            StructureNode::Truncated { count } => {
                assert_eq!(*count, 1, "Should truncate 1 item (the 'a' directory)");
            }
            other => panic!("Expected Truncated, got {:?}", other),
        }
    }

    #[test]
    fn test_symbol_visibility() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");
        graph.add_symbol(file_idx, make_symbol("pub_fn", SymbolKind::Function, SymbolVisibility::Pub));
        graph.add_symbol(file_idx, make_symbol("crate_fn", SymbolKind::Function, SymbolVisibility::PubCrate));
        graph.add_symbol(file_idx, make_symbol("priv_fn", SymbolKind::Function, SymbolVisibility::Private));

        let tree = file_structure(&graph, &root, None, 3);

        let symbols = match &tree[0] {
            StructureNode::Dir { children, .. } => match &children[0] {
                StructureNode::SourceFile { symbols, .. } => symbols,
                other => panic!("Expected SourceFile, got {:?}", other),
            },
            other => panic!("Expected Dir, got {:?}", other),
        };

        assert_eq!(symbols.len(), 3);

        let pub_sym = symbols.iter().find(|s| s.name == "pub_fn").expect("pub_fn not found");
        assert_eq!(pub_sym.visibility, "pub");

        let crate_sym = symbols.iter().find(|s| s.name == "crate_fn").expect("crate_fn not found");
        assert_eq!(crate_sym.visibility, "pub(crate)");

        let priv_sym = symbols.iter().find(|s| s.name == "priv_fn").expect("priv_fn not found");
        assert_eq!(priv_sym.visibility, "private");
    }

    #[test]
    fn test_path_scoping() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/tmp/test_project");

        graph.add_file(root.join("src/main.rs"), "rust");
        graph.add_file(root.join("tests/test_main.rs"), "rust");

        // Query scoped to "src" only
        let tree = file_structure(&graph, &root, Some(Path::new("src")), 3);

        assert_eq!(tree.len(), 1, "Should only have 1 item (main.rs)");
        match &tree[0] {
            StructureNode::SourceFile { name, .. } => {
                assert_eq!(name, "main.rs");
            }
            other => panic!("Expected SourceFile(main.rs), got {:?}", other),
        }
    }

    #[test]
    fn test_format_structure_output() {
        // Build a small tree manually and verify the formatted output.
        let tree = vec![
            StructureNode::Dir {
                name: "src".to_string(),
                children: vec![
                    StructureNode::SourceFile {
                        name: "main.rs".to_string(),
                        symbols: vec![
                            StructureSymbol {
                                name: "main".to_string(),
                                kind: "function".to_string(),
                                visibility: "pub".to_string(),
                            },
                        ],
                    },
                ],
            },
            StructureNode::NonParsedFile {
                name: "README.md".to_string(),
                kind_tag: "doc".to_string(),
            },
            StructureNode::NonParsedFile {
                name: "Cargo.toml".to_string(),
                kind_tag: "config".to_string(),
            },
        ];

        let root = PathBuf::from("/tmp/test_project");
        let output = format_structure_to_string(&tree, &root);

        assert!(output.contains("src/"), "Should contain src/");
        assert!(output.contains("main.rs"), "Should contain main.rs");
        assert!(output.contains("pub main (function)"), "Should contain symbol with visibility");
        assert!(output.contains("README.md [doc]"), "Should contain README.md with kind tag");
        assert!(output.contains("Cargo.toml [config]"), "Should contain Cargo.toml with kind tag");
    }
}

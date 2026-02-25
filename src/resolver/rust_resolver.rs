//! Rust use-path classifier and resolver.
//!
//! Classifies every `use` and `pub use` statement into one of four categories and
//! replaces the Phase 8 self-edge placeholders with real graph edges.
//!
//! # Classification
//! - **Builtin**: `std::`, `core::`, `alloc::` (or bare `std`, `core`, `alloc`) → `GraphNode::Builtin`
//! - **IntraCrate**: `crate::`, `self::`, `super::` → resolved to a `FileInfo` node via `RustModTree`
//! - **CrossWorkspace**: first segment matches a workspace crate name → resolved to that crate's root file
//! - **External**: everything else → `GraphNode::ExternalPackage`

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::GraphNode;
use crate::parser::ParseResult;
use crate::resolver::cargo_workspace::discover_rust_workspace_members;
use crate::resolver::rust_mod_tree::{RustModTree, build_mod_tree};

// ---------------------------------------------------------------------------
// UsePathKind — classification result
// ---------------------------------------------------------------------------

/// Classification of a Rust use path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UsePathKind {
    /// `crate::`, `self::`, `super::` — belongs to the current crate.
    IntraCrate,
    /// First segment matches another workspace crate name.
    CrossWorkspace,
    /// `std::`, `core::`, `alloc::` (or bare identifier).
    Builtin,
    /// Everything else — an external crate from crates.io or the registry.
    External,
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Counts collected during the Rust use-path resolution pass.
#[derive(Debug, Default)]
pub struct RustResolveStats {
    /// Paths resolved to a file node in this workspace.
    pub resolved: usize,
    /// Paths resolved to an `ExternalPackage` node.
    pub external: usize,
    /// Paths resolved to a `Builtin` node (`std`, `core`, `alloc`).
    pub builtin: usize,
    /// Paths that could not be resolved — created `UnresolvedImport` nodes.
    pub unresolved: usize,
    /// Re-export edges that were resolved (counted within `resolved`).
    pub reexport_resolved: usize,
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a Rust use path string into a [`UsePathKind`].
///
/// Rules (checked in this order):
/// 1. Bare `std`, `core`, `alloc` → `Builtin`
/// 2. Starts with `std::`, `core::`, `alloc::` → `Builtin`
/// 3. Starts with `crate::`, `self::`, `super::` → `IntraCrate`
/// 4. First `::` segment (hyphen-normalised) matches a workspace crate name → `CrossWorkspace`
/// 5. Everything else → `External`
fn classify_use_path(
    path: &str,
    _current_crate: &str,
    workspace_crate_names: &HashSet<String>,
) -> UsePathKind {
    // 1 & 2: Builtin check first.
    let bare = matches!(path, "std" | "core" | "alloc");
    let prefixed = path.starts_with("std::")
        || path.starts_with("core::")
        || path.starts_with("alloc::");
    if bare || prefixed {
        return UsePathKind::Builtin;
    }

    // 3: Intra-crate.
    if path.starts_with("crate::") || path.starts_with("self::") || path.starts_with("super::") {
        return UsePathKind::IntraCrate;
    }

    // 4: Cross-workspace — check normalised first segment.
    let first_segment = path
        .split("::")
        .next()
        .unwrap_or("")
        .replace('-', "_");
    if workspace_crate_names.contains(&first_segment) {
        return UsePathKind::CrossWorkspace;
    }

    // 5: External.
    UsePathKind::External
}

// ---------------------------------------------------------------------------
// super:: and self:: path expansion
// ---------------------------------------------------------------------------

/// Convert a `super::…` path into an absolute `crate::…` path.
///
/// Returns `None` if the number of `super::` segments exceeds the module depth
/// (which would be invalid Rust — results in an `UnresolvedImport` node).
fn resolve_super_path(path: &str, current_file: &Path, mod_tree: &RustModTree) -> Option<String> {
    // Get the current file's module path (e.g. "crate::parser::imports").
    let module_path = mod_tree.file_to_module_path(current_file)?.as_str().to_owned();

    // Split module path into segments — skip the leading "crate" to get a Vec of inner segments.
    // "crate::parser::imports" → ["parser", "imports"]
    let module_segments: Vec<&str> = module_path
        .split("::")
        .skip(1) // drop "crate"
        .collect();

    // Count consecutive "super::" prefixes.
    let mut remaining = path;
    let mut super_count = 0usize;
    while let Some(rest) = remaining.strip_prefix("super::") {
        super_count += 1;
        remaining = rest;
    }

    // We need to strip `super_count` levels from the module path.
    // "crate::parser::imports" with 1 super → "crate::parser"
    // "crate::parser" with 1 super → "crate"
    if super_count > module_segments.len() {
        return None; // too many super:: levels
    }

    let kept = module_segments.len() - super_count;
    let mut result = String::from("crate");
    for seg in &module_segments[..kept] {
        result.push_str("::");
        result.push_str(seg);
    }
    if !remaining.is_empty() {
        result.push_str("::");
        result.push_str(remaining);
    }

    Some(result)
}

/// Convert a `self::…` path into an absolute `crate::…` path.
///
/// Returns `None` if the file's module path cannot be determined.
fn resolve_self_path(path: &str, current_file: &Path, mod_tree: &RustModTree) -> Option<String> {
    let module_path = mod_tree.file_to_module_path(current_file)?.as_str().to_owned();
    let rest = path.strip_prefix("self::").unwrap_or(path);
    if rest.is_empty() {
        return Some(module_path);
    }
    Some(format!("{module_path}::{rest}"))
}

// ---------------------------------------------------------------------------
// Main resolver
// ---------------------------------------------------------------------------

/// Run the Rust use-path resolution pass on the code graph.
///
/// Steps:
/// 1. Discover workspace members to build crate name → root file map.
/// 2. Build a `RustModTree` per crate.
/// 3. Build `file_to_crate` map (inverse of mod trees).
/// 4. Collect all Phase 8 self-edges (`RustImport` / `ReExport` where source == target).
/// 5. Remove those self-edges from the graph.
/// 6. For each collected edge, classify the path and emit the appropriate resolved edge or node.
///
/// Returns a [`RustResolveStats`] summary.
pub fn resolve_rust_uses(
    graph: &mut CodeGraph,
    project_root: &Path,
    _parse_results: &HashMap<PathBuf, ParseResult>,
    verbose: bool,
) -> RustResolveStats {
    let mut stats = RustResolveStats::default();

    // -----------------------------------------------------------------------
    // Step 1: Workspace discovery.
    // -----------------------------------------------------------------------
    let workspace_members = discover_rust_workspace_members(project_root);
    if workspace_members.is_empty() {
        // Not a Rust project — nothing to do.
        return stats;
    }

    let workspace_crate_names: HashSet<String> = workspace_members.keys().cloned().collect();

    if verbose {
        eprintln!(
            "  [rust-resolver] workspace crates: {:?}",
            workspace_crate_names
        );
    }

    // -----------------------------------------------------------------------
    // Step 2: Build a RustModTree for each crate.
    // -----------------------------------------------------------------------
    let mut crate_mod_trees: HashMap<String, RustModTree> = HashMap::new();
    for (crate_name, crate_root) in &workspace_members {
        let tree = build_mod_tree(crate_name, crate_root);
        crate_mod_trees.insert(crate_name.clone(), tree);
    }

    // -----------------------------------------------------------------------
    // Step 3: Build file_to_crate map (for each indexed file, which crate?).
    // -----------------------------------------------------------------------
    // file_to_crate: indexed by file PathBuf → crate name.
    // mod_map: String (module path) → PathBuf (file)  →  iterate values for PathBuf
    // reverse_map: PathBuf (file) → String (module path)  →  iterate keys for PathBuf
    let mut file_to_crate: HashMap<PathBuf, String> = HashMap::new();

    for (crate_name, tree) in &crate_mod_trees {
        // From mod_map: values are PathBuf file paths.
        for (_mod_path, file_path) in &tree.mod_map {
            file_to_crate.insert(file_path.clone(), crate_name.clone());
        }
        // From reverse_map: keys are PathBuf file paths.
        for (file_path, _mod_path) in &tree.reverse_map {
            file_to_crate.entry(file_path.clone()).or_insert_with(|| crate_name.clone());
        }
    }

    // Also try to populate from graph's file_index for Rust files not in any mod tree.
    // These are files that were indexed but not reachable from a crate root (e.g. build scripts,
    // proc-macro crates, tests in non-standard locations). We'll still give them a best-effort
    // crate_name assignment based on whether their path starts with a known crate root's directory.

    // -----------------------------------------------------------------------
    // Step 4: Collect self-edges to replace.
    //
    // Safety: we collect indices first (petgraph mutation pitfall — can't iterate and mutate).
    // -----------------------------------------------------------------------
    #[allow(clippy::type_complexity)]
    let mut self_edges: Vec<(petgraph::stable_graph::EdgeIndex, petgraph::stable_graph::NodeIndex, String, bool)> =
        Vec::new();

    for edge_idx in graph.graph.edge_indices() {
        let (src, tgt) = graph.graph.edge_endpoints(edge_idx).unwrap();
        if src != tgt {
            continue; // not a self-edge
        }
        match &graph.graph[edge_idx] {
            EdgeKind::RustImport { path } => {
                self_edges.push((edge_idx, src, path.clone(), false));
            }
            EdgeKind::ReExport { path } => {
                self_edges.push((edge_idx, src, path.clone(), true));
            }
            _ => {}
        }
    }

    if verbose {
        eprintln!(
            "  [rust-resolver] found {} self-edges to resolve",
            self_edges.len()
        );
    }

    // -----------------------------------------------------------------------
    // Step 5: Remove all collected self-edges.
    // -----------------------------------------------------------------------
    // Collect edge indices first; remove in reverse order to keep indices stable.
    let mut edge_indices: Vec<petgraph::stable_graph::EdgeIndex> =
        self_edges.iter().map(|(ei, _, _, _)| *ei).collect();
    edge_indices.sort_by(|a, b| b.cmp(a)); // reverse order
    for ei in edge_indices {
        graph.graph.remove_edge(ei);
    }

    // -----------------------------------------------------------------------
    // Step 6: Classify and emit resolved edges.
    // -----------------------------------------------------------------------
    for (_edge_idx, from_idx, path, is_reexport) in self_edges {
        // Get the source file path for super:: / self:: resolution.
        let from_file_path: Option<PathBuf> = match &graph.graph[from_idx] {
            GraphNode::File(fi) => Some(fi.path.clone()),
            _ => None,
        };

        let from_file = match &from_file_path {
            Some(p) => p,
            None => {
                // Source node is not a File node — skip (shouldn't happen, but defensive).
                stats.unresolved += 1;
                continue;
            }
        };

        // Determine which crate owns this file.
        let current_crate = file_to_crate
            .get(from_file)
            .cloned()
            .unwrap_or_default();

        let kind = classify_use_path(&path, &current_crate, &workspace_crate_names);

        match kind {
            UsePathKind::Builtin => {
                // Extract the root name: "std", "core", or "alloc".
                let root = path.split("::").next().unwrap_or("std");
                graph.add_builtin_node(from_idx, root, &path);
                stats.builtin += 1;
                if verbose {
                    eprintln!(
                        "  [rust-resolver] builtin: {} → {root}",
                        path
                    );
                }
            }

            UsePathKind::IntraCrate => {
                // Normalise to `crate::` absolute path.
                let resolved_path = if path.starts_with("super::") {
                    let mod_tree = crate_mod_trees.get(&current_crate);
                    mod_tree
                        .and_then(|t| resolve_super_path(&path, from_file, t))
                } else if path.starts_with("self::") {
                    let mod_tree = crate_mod_trees.get(&current_crate);
                    mod_tree
                        .and_then(|t| resolve_self_path(&path, from_file, t))
                } else {
                    // Already `crate::...`
                    Some(path.clone())
                };

                let resolved_path = match resolved_path {
                    Some(p) => p,
                    None => {
                        graph.add_unresolved_import(from_idx, &path, "rust: super:: exceeds module depth");
                        stats.unresolved += 1;
                        continue;
                    }
                };

                // Handle glob imports: strip `::*` and resolve the module prefix.
                let lookup_path = if resolved_path.ends_with("::*") {
                    resolved_path[..resolved_path.len() - 3].to_string()
                } else {
                    resolved_path.clone()
                };

                // Look up in the mod tree via progressive stripping.
                let mod_tree = crate_mod_trees.get(&current_crate);
                let target_file = mod_tree.and_then(|t| t.resolve_module_path(&lookup_path));

                match target_file {
                    Some(target_path) => {
                        // Check if this file is in the graph.
                        if let Some(&target_idx) = graph.file_index.get(target_path) {
                            graph.add_resolved_import(from_idx, target_idx, &path);
                            stats.resolved += 1;
                            if is_reexport {
                                stats.reexport_resolved += 1;
                            }
                            if verbose {
                                eprintln!(
                                    "  [rust-resolver] intra: {} → {}",
                                    path,
                                    target_path.display()
                                );
                            }
                        } else {
                            // File exists in mod tree but not in graph (e.g. excluded by config).
                            // Still count as resolved but no edge.
                            stats.resolved += 1;
                            if verbose {
                                eprintln!(
                                    "  [rust-resolver] intra (not indexed): {} → {}",
                                    path,
                                    target_path.display()
                                );
                            }
                        }
                    }
                    None => {
                        graph.add_unresolved_import(from_idx, &path, "rust: could not resolve module path");
                        stats.unresolved += 1;
                        if verbose {
                            eprintln!(
                                "  [rust-resolver] unresolved intra: {}",
                                path
                            );
                        }
                    }
                }
            }

            UsePathKind::CrossWorkspace => {
                // Resolve to the crate root file of the target workspace crate.
                let first_segment = path
                    .split("::")
                    .next()
                    .unwrap_or("")
                    .replace('-', "_");
                let crate_root = workspace_members.get(&first_segment);

                match crate_root {
                    Some(root_path) => {
                        if let Some(&target_idx) = graph.file_index.get(root_path) {
                            graph.add_resolved_import(from_idx, target_idx, &path);
                            stats.resolved += 1;
                            if is_reexport {
                                stats.reexport_resolved += 1;
                            }
                            if verbose {
                                eprintln!(
                                    "  [rust-resolver] cross-workspace: {} → {}",
                                    path,
                                    root_path.display()
                                );
                            }
                        } else {
                            // Crate root not indexed — still count as resolved.
                            stats.resolved += 1;
                        }
                    }
                    None => {
                        graph.add_unresolved_import(from_idx, &path, "rust: workspace crate root not found");
                        stats.unresolved += 1;
                    }
                }
            }

            UsePathKind::External => {
                // Extract package name: first `::` segment, normalised.
                let pkg_name = path
                    .split("::")
                    .next()
                    .unwrap_or(&path)
                    .replace('-', "_");
                graph.add_external_package(from_idx, &pkg_name, &path);
                stats.external += 1;
                if verbose {
                    eprintln!(
                        "  [rust-resolver] external: {} → {pkg_name}",
                        path
                    );
                }
            }
        }
    }

    if verbose {
        eprintln!(
            "  [rust-resolver] resolved={} external={} builtin={} unresolved={}",
            stats.resolved, stats.external, stats.builtin, stats.unresolved
        );
    }

    stats
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_workspace_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // --- classify_use_path tests ---

    #[test]
    fn test_classify_std_prefixed() {
        let ws = make_workspace_set(&[]);
        assert_eq!(classify_use_path("std::collections::HashMap", "", &ws), UsePathKind::Builtin);
        assert_eq!(classify_use_path("core::mem::size_of", "", &ws), UsePathKind::Builtin);
        assert_eq!(classify_use_path("alloc::vec::Vec", "", &ws), UsePathKind::Builtin);
    }

    #[test]
    fn test_classify_bare_builtin() {
        let ws = make_workspace_set(&[]);
        assert_eq!(classify_use_path("std", "", &ws), UsePathKind::Builtin);
        assert_eq!(classify_use_path("core", "", &ws), UsePathKind::Builtin);
        assert_eq!(classify_use_path("alloc", "", &ws), UsePathKind::Builtin);
    }

    #[test]
    fn test_classify_intra_crate() {
        let ws = make_workspace_set(&[]);
        assert_eq!(classify_use_path("crate::parser::imports", "", &ws), UsePathKind::IntraCrate);
        assert_eq!(classify_use_path("self::utils", "", &ws), UsePathKind::IntraCrate);
        assert_eq!(classify_use_path("super::sibling", "", &ws), UsePathKind::IntraCrate);
    }

    #[test]
    fn test_classify_cross_workspace() {
        let ws = make_workspace_set(&["my_lib"]);
        assert_eq!(classify_use_path("my_lib::Foo", "", &ws), UsePathKind::CrossWorkspace);
    }

    #[test]
    fn test_classify_external() {
        let ws = make_workspace_set(&[]);
        assert_eq!(classify_use_path("serde::Serialize", "", &ws), UsePathKind::External);
        assert_eq!(classify_use_path("tokio::runtime", "", &ws), UsePathKind::External);
    }

    #[test]
    fn test_classify_hyphen_workspace_crate() {
        // Hyphen-normalised crate names in workspace
        let ws = make_workspace_set(&["beta_utils"]);
        assert_eq!(classify_use_path("beta_utils::something", "", &ws), UsePathKind::CrossWorkspace);
    }

    // --- resolve_super_path tests ---

    #[test]
    fn test_resolve_super_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        std::fs::create_dir_all(p.join("src/parser")).unwrap();
        std::fs::write(p.join("src/lib.rs"), "pub mod parser;\n").unwrap();
        std::fs::write(p.join("src/parser.rs"), "pub mod imports;\n").unwrap();
        std::fs::write(p.join("src/parser/imports.rs"), "use super::Parser;").unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"test-crate\"\nversion = \"0.1.0\"\n",
        ).unwrap();

        let tree = crate::resolver::rust_mod_tree::build_mod_tree("test_crate", &p.join("src/lib.rs"));
        let imports_file = p.join("src/parser/imports.rs");
        let result = resolve_super_path("super::Parser", &imports_file, &tree);
        assert_eq!(result, Some("crate::parser::Parser".to_string()));
    }

    #[test]
    fn test_resolve_super_too_deep_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/lib.rs"), "").unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"test-crate\"\nversion = \"0.1.0\"\n",
        ).unwrap();

        let tree = crate::resolver::rust_mod_tree::build_mod_tree("test_crate", &p.join("src/lib.rs"));
        let lib_file = p.join("src/lib.rs");
        // super:: from crate root (0 segments after "crate") → too deep
        let result = resolve_super_path("super::Foo", &lib_file, &tree);
        assert!(result.is_none(), "super:: from root should fail");
    }

    #[test]
    fn test_resolve_self_path() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/lib.rs"), "pub mod parser;\n").unwrap();
        std::fs::write(p.join("src/parser.rs"), "").unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"test-crate\"\nversion = \"0.1.0\"\n",
        ).unwrap();

        let tree = crate::resolver::rust_mod_tree::build_mod_tree("test_crate", &p.join("src/lib.rs"));
        let parser_file = p.join("src/parser.rs");
        let result = resolve_self_path("self::Foo", &parser_file, &tree);
        assert_eq!(result, Some("crate::parser::Foo".to_string()));
    }
}

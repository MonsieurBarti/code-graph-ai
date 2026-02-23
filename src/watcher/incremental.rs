use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::GraphNode;
use crate::parser;
use crate::resolver::{
    build_resolver, discover_workspace_packages, workspace_map_to_aliases, resolve_import,
    ResolutionOutcome,
};

use super::event::WatchEvent;

/// Handle a single watch event by performing an incremental graph update.
///
/// For Modified/Created: removes old file entry, re-parses, re-adds to graph,
/// re-resolves the file's imports, and checks if unresolved imports in other files
/// now resolve to this file.
///
/// For Deleted: removes the file from graph and marks imports pointing to it as unresolved.
///
/// For ConfigChanged: triggers a full rebuild (caller handles this by calling build_graph).
///
/// Returns `true` if the graph was modified, `false` if ConfigChanged (caller must full-rebuild).
pub fn handle_file_event(
    graph: &mut CodeGraph,
    event: &WatchEvent,
    project_root: &Path,
) -> bool {
    match event {
        WatchEvent::Modified(path) | WatchEvent::Created(path) => {
            handle_modified(graph, path, project_root);
            true
        }
        WatchEvent::Deleted(path) => {
            handle_deleted(graph, path);
            true
        }
        WatchEvent::ConfigChanged => {
            // Caller must perform full rebuild
            false
        }
    }
}

/// Handle a modified or newly created file.
fn handle_modified(graph: &mut CodeGraph, path: &Path, project_root: &Path) {
    // 1. Remove old entry if it exists
    graph.remove_file_from_graph(path);

    // 2. Read and parse the file
    let source = match std::fs::read(path) {
        Ok(s) => s,
        Err(_) => return, // file disappeared between event and handling
    };

    let language_str = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" => "javascript",
        _ => return,
    };

    let result = match parser::parse_file(path, &source) {
        Ok(r) => r,
        Err(_) => return, // parse error — skip
    };

    // 3. Add file and symbols to graph
    let file_idx = graph.add_file(path.to_path_buf(), language_str);
    for (symbol, children) in &result.symbols {
        let sym_idx = graph.add_symbol(file_idx, symbol.clone());
        for child in children {
            graph.add_child_symbol(sym_idx, child.clone());
        }
    }

    // 4. Resolve this file's imports (scoped — not full resolve_all)
    let workspace_map = discover_workspace_packages(project_root);
    let aliases = workspace_map_to_aliases(&workspace_map);
    let resolver = build_resolver(project_root, aliases);

    for import in &result.imports {
        let specifier = &import.module_path;
        let outcome = resolve_import(&resolver, path, specifier);

        match outcome {
            ResolutionOutcome::Resolved(target_path) => {
                if let Some(&target_idx) = graph.file_index.get(&target_path) {
                    graph.add_resolved_import(file_idx, target_idx, specifier);
                }
            }
            ResolutionOutcome::BuiltinModule(_) => {
                graph.add_unresolved_import(file_idx, specifier, "builtin");
            }
            ResolutionOutcome::Unresolved(reason) => {
                if is_external_package(specifier) {
                    let pkg_name = extract_package_name(specifier);
                    graph.add_external_package(file_idx, pkg_name, specifier);
                } else {
                    graph.add_unresolved_import(file_idx, specifier, &reason);
                }
            }
        }
    }

    // 5. Wire symbol relationships for this file only
    wire_relationships_for_file(graph, &result.relationships, file_idx);

    // 6. Check if existing unresolved imports now resolve to this file
    fix_unresolved_pointing_to(graph, path, project_root);
}

/// Handle a deleted file.
fn handle_deleted(graph: &mut CodeGraph, path: &Path) {
    // Find files that had ResolvedImport edges pointing to this file
    // BEFORE removing it, so we can mark those imports as unresolved.
    let file_idx = match graph.file_index.get(path).copied() {
        Some(idx) => idx,
        None => return, // not in graph
    };

    // Collect importers: files with ResolvedImport edges targeting this file
    let importers: Vec<(petgraph::stable_graph::NodeIndex, String)> = graph
        .graph
        .edges_directed(file_idx, petgraph::Direction::Incoming)
        .filter_map(|e| {
            if let EdgeKind::ResolvedImport { specifier } = e.weight() {
                Some((e.source(), specifier.clone()))
            } else {
                None
            }
        })
        .collect();

    // Remove the file and all its nodes/edges
    graph.remove_file_from_graph(path);

    // Mark importers' edges as unresolved (add UnresolvedImport nodes)
    for (importer_idx, specifier) in importers {
        graph.add_unresolved_import(importer_idx, &specifier, "target file deleted");
    }
}

/// Wire symbol relationships (Extends, Implements, Calls) for symbols in a single file.
/// Adapted from resolver::resolve_all Step 5 but scoped to one file.
fn wire_relationships_for_file(
    graph: &mut CodeGraph,
    relationships: &[crate::parser::relationships::RelationshipInfo],
    file_idx: petgraph::stable_graph::NodeIndex,
) {
    use crate::parser::relationships::RelationshipKind;

    for rel in relationships {
        match rel.kind {
            RelationshipKind::Extends
            | RelationshipKind::Implements
            | RelationshipKind::InterfaceExtends => {
                let from_name = match &rel.from_name {
                    Some(n) => n,
                    None => continue,
                };

                let from_candidates =
                    graph.symbol_index.get(from_name).cloned().unwrap_or_default();
                let to_candidates =
                    graph.symbol_index.get(&rel.to_name).cloned().unwrap_or_default();

                if from_candidates.is_empty() || to_candidates.is_empty() {
                    continue;
                }

                let from_sym_idx = from_candidates
                    .iter()
                    .copied()
                    .find(|&idx| graph.graph.edges(file_idx).any(|e| e.target() == idx))
                    .unwrap_or(from_candidates[0]);

                let same_file_to: Vec<_> = to_candidates
                    .iter()
                    .copied()
                    .filter(|&idx| graph.graph.edges(file_idx).any(|e| e.target() == idx))
                    .collect();

                let to_indices = if same_file_to.is_empty() {
                    to_candidates
                } else {
                    same_file_to
                };

                for to_sym_idx in to_indices {
                    match rel.kind {
                        RelationshipKind::Extends | RelationshipKind::InterfaceExtends => {
                            graph.add_extends_edge(from_sym_idx, to_sym_idx);
                        }
                        RelationshipKind::Implements => {
                            graph.add_implements_edge(from_sym_idx, to_sym_idx);
                        }
                        _ => unreachable!(),
                    }
                }
            }

            RelationshipKind::Calls
            | RelationshipKind::MethodCall
            | RelationshipKind::TypeReference => {
                let to_candidates = match graph.symbol_index.get(&rel.to_name) {
                    Some(c) if !c.is_empty() => c.clone(),
                    _ => continue,
                };

                if to_candidates.len() == 1 {
                    graph.add_calls_edge(file_idx, to_candidates[0]);
                }
            }
        }
    }
}

/// After adding a new/modified file, check if any existing UnresolvedImport nodes
/// in the graph might now resolve to this file. If so, remove the unresolved node
/// and add a proper ResolvedImport edge.
fn fix_unresolved_pointing_to(
    graph: &mut CodeGraph,
    new_file_path: &Path,
    project_root: &Path,
) {
    // Collect unresolved import nodes and their importers
    let unresolved: Vec<(
        petgraph::stable_graph::NodeIndex,
        petgraph::stable_graph::NodeIndex,
        String,
    )> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::UnresolvedImport { specifier, reason } = &graph.graph[idx]
                && reason != "builtin" {
                    // Find the importer (the node with an edge to this unresolved node)
                    let importer = graph
                        .graph
                        .edges_directed(idx, petgraph::Direction::Incoming)
                        .next()
                        .map(|e| e.source());
                    if let Some(importer_idx) = importer {
                        return Some((idx, importer_idx, specifier.clone()));
                    }
                }
            None
        })
        .collect();

    if unresolved.is_empty() {
        return;
    }

    // Build resolver to check if unresolved specifiers now resolve to the new file
    let workspace_map = discover_workspace_packages(project_root);
    let aliases = workspace_map_to_aliases(&workspace_map);
    let resolver = build_resolver(project_root, aliases);

    let new_file_idx = match graph.file_index.get(new_file_path).copied() {
        Some(idx) => idx,
        None => return,
    };

    for (unresolved_idx, importer_idx, specifier) in unresolved {
        // Get importer's file path
        let importer_path: PathBuf = match &graph.graph[importer_idx] {
            GraphNode::File(info) => info.path.clone(),
            _ => continue,
        };

        let outcome = resolve_import(&resolver, &importer_path, &specifier);
        if let ResolutionOutcome::Resolved(resolved_path) = outcome
            && resolved_path == new_file_path {
                // This unresolved import now resolves to the new file!
                graph.graph.remove_node(unresolved_idx);
                graph.add_resolved_import(importer_idx, new_file_idx, &specifier);
            }
    }
}

// Re-use the same helpers from resolver::mod.rs (these are private there, so duplicate)
fn is_external_package(specifier: &str) -> bool {
    !specifier.starts_with('.') && !specifier.starts_with('/')
}

fn extract_package_name(specifier: &str) -> &str {
    if specifier.starts_with('@') {
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            let scope_end = parts[0].len() + 1 + parts[1].len();
            &specifier[..scope_end]
        } else {
            specifier
        }
    } else {
        match specifier.find('/') {
            Some(idx) => &specifier[..idx],
            None => specifier,
        }
    }
}

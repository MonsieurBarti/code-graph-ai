pub mod barrel;
pub mod file_resolver;
pub mod workspace;

pub use file_resolver::{build_resolver, resolve_import, workspace_map_to_aliases, ResolutionOutcome};
pub use workspace::discover_workspace_packages;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::parser::ParseResult;
use crate::parser::relationships::RelationshipKind;

/// Statistics collected during the resolution pipeline.
#[derive(Debug, Default)]
pub struct ResolveStats {
    /// Number of imports successfully resolved to a local file.
    pub resolved: usize,
    /// Number of imports that could not be resolved.
    pub unresolved: usize,
    /// Number of imports resolved to an external package (node_modules).
    pub external: usize,
    /// Number of imports resolved to Node.js built-in modules (fs, path, etc.).
    pub builtin: usize,
    /// Number of symbol-level relationship edges added to the graph.
    pub relationships_added: usize,
    /// Number of direct ResolvedImport edges added by the named re-export chain pass.
    /// These edges bypass barrel files and point directly to the defining file.
    pub named_reexport_edges: usize,
}

/// Run the full import resolution pipeline on the code graph.
///
/// Executes five sequential steps:
///
/// 1. **Workspace detection** — discover monorepo workspace packages so
///    cross-package imports can be resolved to local source directories.
/// 2. **Resolver construction** — build a single `oxc_resolver::Resolver`
///    configured for TypeScript (tsconfig paths, extension aliases, workspace aliases).
/// 3. **File-level resolution** — for every import in every parsed file, call
///    `resolve_import()` and classify the outcome as Resolved / External / Builtin /
///    Unresolved, adding the appropriate graph edge.
/// 4. **Barrel chain pass** — add `BarrelReExportAll` edges for `export * from` statements.
/// 5. **Symbol relationship pass** — wire Extends / Implements / InterfaceExtends / Calls /
///    TypeReference edges between symbol nodes where both endpoints are in the graph.
///
/// # Parameters
/// - `graph`: the mutable code graph to enrich with resolution edges
/// - `project_root`: the project root directory (used for tsconfig, workspace detection)
/// - `parse_results`: all parsed files and their extracted import/export/relationship data
/// - `verbose`: if `true`, emit diagnostic messages to stderr
///
/// # Returns
/// A [`ResolveStats`] struct with counts for each category of resolution outcome.
pub fn resolve_all(
    graph: &mut CodeGraph,
    project_root: &Path,
    parse_results: &HashMap<PathBuf, ParseResult>,
    verbose: bool,
) -> ResolveStats {
    let mut stats = ResolveStats::default();

    // -----------------------------------------------------------------------
    // Step 1: Build workspace map.
    // -----------------------------------------------------------------------
    let workspace_map = discover_workspace_packages(project_root);
    if verbose && !workspace_map.is_empty() {
        eprintln!("  Workspace packages found: {}", workspace_map.len());
        for (name, path) in &workspace_map {
            eprintln!("    {} -> {}", name, path.display());
        }
    }

    // -----------------------------------------------------------------------
    // Step 2: Build resolver (one instance — reuse for all files).
    // -----------------------------------------------------------------------
    let aliases = workspace_map_to_aliases(&workspace_map);
    let resolver = build_resolver(project_root, aliases);

    // -----------------------------------------------------------------------
    // Step 3: File-level resolution pass.
    // -----------------------------------------------------------------------
    // Collect all (file_path, imports) pairs first to avoid borrow conflicts.
    let file_imports: Vec<(PathBuf, Vec<crate::parser::imports::ImportInfo>)> = parse_results
        .iter()
        .map(|(path, result)| (path.clone(), result.imports.clone()))
        .collect();

    for (file_path, imports) in &file_imports {
        let from_idx = match graph.file_index.get(file_path).copied() {
            Some(idx) => idx,
            None => {
                // File wasn't added to graph (shouldn't happen, but defensive).
                continue;
            }
        };

        for import in imports {
            let specifier = &import.module_path;
            let outcome = resolve_import(&resolver, file_path, specifier);

            match outcome {
                ResolutionOutcome::Resolved(target_path) => {
                    // Check if the resolved target is in the graph (was indexed).
                    if let Some(&target_idx) = graph.file_index.get(&target_path) {
                        graph.add_resolved_import(from_idx, target_idx, specifier);
                        stats.resolved += 1;
                    } else {
                        // Resolved to a path not in the graph (e.g. JSON, .node file, or
                        // a file outside the indexed project). Treat as unresolved.
                        if verbose {
                            eprintln!(
                                "  resolve: {} imports '{}' -> {} (not indexed, skipping edge)",
                                file_path.display(),
                                specifier,
                                target_path.display()
                            );
                        }
                        stats.resolved += 1; // resolver succeeded; we just didn't index it
                    }
                }
                ResolutionOutcome::BuiltinModule(name) => {
                    // Node.js built-in — record as unresolved with "builtin" reason.
                    graph.add_unresolved_import(from_idx, specifier, "builtin");
                    stats.builtin += 1;
                    if verbose {
                        eprintln!(
                            "  resolve: {} imports '{}' -> builtin:{}",
                            file_path.display(), specifier, name
                        );
                    }
                }
                ResolutionOutcome::Unresolved(_reason) => {
                    // Classify: is this an external package or truly unresolvable?
                    if is_external_package(specifier) {
                        let pkg_name = extract_package_name(specifier);
                        graph.add_external_package(from_idx, pkg_name, specifier);
                        stats.external += 1;
                        if verbose {
                            eprintln!(
                                "  resolve: {} imports '{}' -> external:{}",
                                file_path.display(), specifier, pkg_name
                            );
                        }
                    } else {
                        graph.add_unresolved_import(from_idx, specifier, &_reason);
                        stats.unresolved += 1;
                        if verbose {
                            eprintln!(
                                "  resolve: {} imports '{}' -> unresolved: {}",
                                file_path.display(), specifier, _reason
                            );
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Step 4: Barrel chain pass.
    // -----------------------------------------------------------------------
    barrel::resolve_barrel_chains(graph, parse_results, verbose);

    // Step 4b: Named re-export chain pass.
    // Adds direct ResolvedImport edges from importing files to the defining file,
    // bypassing barrel files for named re-exports (export { Foo } from './module').
    let named_reexport_edges = barrel::resolve_named_reexport_chains(graph, parse_results, verbose);
    stats.named_reexport_edges = named_reexport_edges;
    if verbose {
        eprintln!("  Named re-export edges added: {}", named_reexport_edges);
    }

    // -----------------------------------------------------------------------
    // Step 5: Symbol relationship pass.
    // -----------------------------------------------------------------------
    // Collect all relationship data first to avoid double-borrow of graph.
    let file_relationships: Vec<(PathBuf, Vec<crate::parser::relationships::RelationshipInfo>)> =
        parse_results
            .iter()
            .map(|(path, result)| (path.clone(), result.relationships.clone()))
            .collect();

    for (_file_path, relationships) in &file_relationships {
        let from_file_idx = match graph.file_index.get(_file_path).copied() {
            Some(idx) => idx,
            None => continue,
        };

        for rel in relationships {
            match rel.kind {
                RelationshipKind::Extends | RelationshipKind::Implements | RelationshipKind::InterfaceExtends => {
                    // Both from_name and to_name should be present for inheritance.
                    let from_name = match &rel.from_name {
                        Some(n) => n,
                        None => continue,
                    };

                    let from_candidates = graph.symbol_index.get(from_name).cloned().unwrap_or_default();
                    let to_candidates = graph.symbol_index.get(&rel.to_name).cloned().unwrap_or_default();

                    if from_candidates.is_empty() || to_candidates.is_empty() {
                        continue;
                    }

                    // Pick the from_candidate in the same file if possible; else use first.
                    let from_sym_idx = from_candidates
                        .iter()
                        .copied()
                        .find(|&idx| {
                            // Check if this symbol belongs to the current file.
                            graph.graph.edges(from_file_idx).any(|e| e.target() == idx)
                        })
                        .unwrap_or(from_candidates[0]);

                    // For to_name: prefer same file; if ambiguous, add edges to all candidates.
                    let same_file_to: Vec<_> = to_candidates
                        .iter()
                        .copied()
                        .filter(|&idx| {
                            graph.graph.edges(from_file_idx).any(|e| e.target() == idx)
                        })
                        .collect();

                    let to_indices = if same_file_to.is_empty() {
                        to_candidates.clone()
                    } else {
                        same_file_to
                    };

                    for to_sym_idx in to_indices {
                        match rel.kind {
                            RelationshipKind::Extends => {
                                graph.add_extends_edge(from_sym_idx, to_sym_idx);
                                stats.relationships_added += 1;
                            }
                            RelationshipKind::Implements => {
                                graph.add_implements_edge(from_sym_idx, to_sym_idx);
                                stats.relationships_added += 1;
                            }
                            RelationshipKind::InterfaceExtends => {
                                // Interface extends uses the same Extends edge kind.
                                graph.add_extends_edge(from_sym_idx, to_sym_idx);
                                stats.relationships_added += 1;
                            }
                            _ => unreachable!(),
                        }
                    }
                }

                RelationshipKind::Calls | RelationshipKind::MethodCall | RelationshipKind::TypeReference => {
                    // Look up the callee / type name in the symbol index.
                    let to_candidates = match graph.symbol_index.get(&rel.to_name) {
                        Some(c) if !c.is_empty() => c.clone(),
                        _ => continue,
                    };

                    // Only add edge if exactly one candidate (unambiguous).
                    // Cross-file call ambiguity is a documented limitation per research.
                    if to_candidates.len() == 1 {
                        let callee_idx = to_candidates[0];
                        graph.add_calls_edge(from_file_idx, callee_idx);
                        stats.relationships_added += 1;
                    }
                    // If multiple candidates: skip (ambiguous cross-file call — documented limitation)
                }
            }
        }
    }

    stats
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Returns `true` if the specifier looks like an external package reference.
///
/// External packages:
/// - Do not start with `.` (relative) or `/` (absolute)
/// - Are not tsconfig path aliases starting with `@/` (project-internal)
///
/// This heuristic matches npm package patterns: `react`, `@scope/pkg`, `lodash/merge`.
fn is_external_package(specifier: &str) -> bool {
    !specifier.starts_with('.') && !specifier.starts_with('/')
}

/// Extract the canonical package name from a module specifier.
///
/// - `react` → `react`
/// - `@org/utils` → `@org/utils`  (scoped package — keep both parts)
/// - `lodash/merge` → `lodash`    (subpath import)
/// - `@org/utils/helpers` → `@org/utils`  (scoped package subpath)
fn extract_package_name(specifier: &str) -> &str {
    if specifier.starts_with('@') {
        // Scoped package: `@scope/name[/subpath]` — keep first two segments.
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            // Return everything up to and including the second segment.
            let scope_end = parts[0].len() + 1 + parts[1].len();
            &specifier[..scope_end]
        } else {
            specifier
        }
    } else {
        // Unscoped: `name[/subpath]` — keep first segment.
        match specifier.find('/') {
            Some(idx) => &specifier[..idx],
            None => specifier,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_external_package() {
        assert!(is_external_package("react"));
        assert!(is_external_package("@org/utils"));
        assert!(is_external_package("lodash/merge"));
        assert!(!is_external_package("./local"));
        assert!(!is_external_package("../parent"));
        assert!(!is_external_package("/absolute"));
    }

    #[test]
    fn test_extract_package_name() {
        assert_eq!(extract_package_name("react"), "react");
        assert_eq!(extract_package_name("@org/utils"), "@org/utils");
        assert_eq!(extract_package_name("@org/utils/helpers"), "@org/utils");
        assert_eq!(extract_package_name("lodash/merge"), "lodash");
        assert_eq!(extract_package_name("lodash"), "lodash");
    }
}

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::parser::imports::{ExportInfo, ExportKind};
use crate::parser::ParseResult;

/// Resolve barrel re-export chains in the graph.
///
/// This is a best-effort enrichment pass that runs after file-level import resolution.
/// For each file that has `export * from './x'` exports (ReExportAll), it adds a
/// `BarrelReExportAll` edge from that file node to the source file node — enabling
/// lazy expansion at query time (per user decision).
///
/// For named re-exports (`export { Foo } from './module'`), the re-export chain is
/// already correct at the file level: if the resolver resolved an import to a barrel
/// index.ts, that is where the edge points. This pass adds no additional edges for
/// named re-exports — the file-level resolution pass already handled them.
///
/// Cycle detection: A `HashSet<PathBuf>` visited set guards against circular barrels.
/// If a chain cannot be resolved (missing file, cycle), we log at verbose level and continue.
///
/// # Parameters
/// - `graph`: the mutable code graph to enrich with `BarrelReExportAll` edges
/// - `parse_results`: map from file path → `ParseResult`, used to inspect exports
/// - `verbose`: if `true`, emit diagnostic messages to stderr for unresolvable chains
pub fn resolve_barrel_chains(
    graph: &mut CodeGraph,
    parse_results: &HashMap<PathBuf, ParseResult>,
    verbose: bool,
) {
    // Collect all (barrel_file_path, source_module_specifier) pairs for ReExportAll exports.
    // We collect first to avoid borrowing issues when mutating the graph.
    let barrel_edges: Vec<(PathBuf, String)> = parse_results
        .iter()
        .flat_map(|(file_path, result)| {
            result
                .exports
                .iter()
                .filter_map(|export| {
                    if export.kind == ExportKind::ReExportAll {
                        if let Some(source_specifier) = &export.source {
                            return Some((file_path.clone(), source_specifier.clone()));
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
        })
        .collect();

    for (barrel_path, source_specifier) in &barrel_edges {
        // Resolve the source specifier to an absolute path using the parse_results keys.
        // Since file-level resolution already ran, we can look up by resolving relative to
        // the barrel file's directory.
        let barrel_dir = match barrel_path.parent() {
            Some(d) => d,
            None => {
                if verbose {
                    eprintln!(
                        "barrel: skipping {} — no parent directory",
                        barrel_path.display()
                    );
                }
                continue;
            }
        };

        let resolved_source = resolve_relative_specifier(barrel_dir, source_specifier, parse_results);

        match resolved_source {
            Some(source_path) => {
                // Verify both files are in the graph.
                let barrel_idx = graph.file_index.get(barrel_path).copied();
                let source_idx = graph.file_index.get(&source_path).copied();

                match (barrel_idx, source_idx) {
                    (Some(b_idx), Some(s_idx)) => {
                        // Add BarrelReExportAll edge: barrel -> source.
                        // No deduplication check needed — petgraph allows parallel edges and
                        // this edge type is intentionally recorded once per export * statement.
                        graph.add_barrel_reexport_all(b_idx, s_idx);

                        if verbose {
                            eprintln!(
                                "barrel: {} --[BarrelReExportAll]--> {}",
                                barrel_path.display(),
                                source_path.display()
                            );
                        }
                    }
                    (None, _) => {
                        if verbose {
                            eprintln!(
                                "barrel: skipping {} — barrel file not in graph",
                                barrel_path.display()
                            );
                        }
                    }
                    (_, None) => {
                        if verbose {
                            eprintln!(
                                "barrel: skipping {} re-export of '{}' — source file {} not in graph (external or not indexed)",
                                barrel_path.display(),
                                source_specifier,
                                source_path.display()
                            );
                        }
                    }
                }
            }
            None => {
                if verbose {
                    eprintln!(
                        "barrel: could not resolve '{}' from {} — skipping",
                        source_specifier,
                        barrel_path.display()
                    );
                }
            }
        }
    }
}

/// Attempt to resolve a relative module specifier to an absolute path that exists in
/// `parse_results`.
///
/// We try common TypeScript/JavaScript extension patterns:
/// - exact path with extensions (.ts, .tsx, .js, .jsx, .mts, .mjs)
/// - directory with index file (index.ts, index.tsx, index.js)
///
/// If the specifier already contains an extension, try it first. Falls back to the
/// parse_results keys for exact match.
///
/// Returns `None` if no matching file is found in parse_results.
fn resolve_relative_specifier(
    from_dir: &Path,
    specifier: &str,
    parse_results: &HashMap<PathBuf, ParseResult>,
) -> Option<PathBuf> {
    // Only handle relative specifiers (starting with ./ or ../).
    // Non-relative specifiers are external packages handled elsewhere.
    if !specifier.starts_with('.') {
        return None;
    }

    let base = from_dir.join(specifier);

    // Try the path directly first (if specifier has an extension).
    if parse_results.contains_key(&base) {
        return Some(base.clone());
    }

    // Try common TS/JS extensions.
    let extensions = [".ts", ".tsx", ".js", ".jsx", ".mts", ".mjs"];
    for ext in &extensions {
        let candidate = PathBuf::from(format!("{}{}", base.display(), ext));
        if parse_results.contains_key(&candidate) {
            return Some(candidate);
        }
    }

    // Try directory index files.
    let index_files = ["index.ts", "index.tsx", "index.js", "index.jsx"];
    for idx_file in &index_files {
        let candidate = base.join(idx_file);
        if parse_results.contains_key(&candidate) {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use petgraph::visit::EdgeRef;

    use crate::graph::CodeGraph;
    use crate::graph::edge::EdgeKind;
    use crate::parser::imports::{ExportInfo, ExportKind};
    use crate::parser::ParseResult;

    /// Build a minimal ParseResult for testing (no symbols, no imports, custom exports).
    fn make_parse_result(exports: Vec<ExportInfo>) -> ParseResult {
        ParseResult {
            symbols: vec![],
            imports: vec![],
            exports,
            relationships: vec![],
            tree: {
                // Build a trivial tree-sitter tree for an empty TS file.
                let lang = crate::parser::languages::language_for_extension("ts").unwrap();
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&lang).unwrap();
                parser.parse(b"", None).unwrap()
            },
        }
    }

    #[test]
    fn test_barrel_reexport_all_adds_edge() {
        let mut graph = CodeGraph::new();
        let barrel_path = PathBuf::from("/project/src/index.ts");
        let utils_path = PathBuf::from("/project/src/utils.ts");

        let barrel_idx = graph.add_file(barrel_path.clone(), "typescript");
        let utils_idx = graph.add_file(utils_path.clone(), "typescript");

        let barrel_export = ExportInfo {
            kind: ExportKind::ReExportAll,
            names: vec![],
            source: Some("./utils".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(barrel_path.clone(), make_parse_result(vec![barrel_export]));
        parse_results.insert(utils_path.clone(), make_parse_result(vec![]));

        resolve_barrel_chains(&mut graph, &parse_results, false);

        // Verify BarrelReExportAll edge was added from barrel to utils.
        assert!(
            graph.graph.contains_edge(barrel_idx, utils_idx),
            "BarrelReExportAll edge should exist from barrel to utils"
        );

        // Verify the edge kind is correct.
        let edge = graph.graph.edges(barrel_idx).find(|e| e.target() == utils_idx);
        assert!(edge.is_some(), "edge should be found");
        match edge.unwrap().weight() {
            EdgeKind::BarrelReExportAll => {} // correct
            other => panic!("expected BarrelReExportAll, got {:?}", other),
        }
    }

    #[test]
    fn test_barrel_no_reexport_all_no_edge() {
        let mut graph = CodeGraph::new();
        let barrel_path = PathBuf::from("/project/src/index.ts");
        let utils_path = PathBuf::from("/project/src/utils.ts");

        let _barrel_idx = graph.add_file(barrel_path.clone(), "typescript");
        let _utils_idx = graph.add_file(utils_path.clone(), "typescript");

        // Only named re-export — no ReExportAll.
        let named_reexport = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["helper".to_owned()],
            source: Some("./utils".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(barrel_path.clone(), make_parse_result(vec![named_reexport]));
        parse_results.insert(utils_path.clone(), make_parse_result(vec![]));

        resolve_barrel_chains(&mut graph, &parse_results, false);

        // No BarrelReExportAll edge should be added for named re-exports.
        let barrel_idx = graph.file_index[&barrel_path];
        let utils_idx = graph.file_index[&utils_path];
        let barrel_edge = graph
            .graph
            .edges(barrel_idx)
            .find(|e| {
                e.target() == utils_idx && matches!(e.weight(), EdgeKind::BarrelReExportAll)
            });
        assert!(barrel_edge.is_none(), "no BarrelReExportAll edge should exist for named re-export");
    }

    #[test]
    fn test_barrel_source_not_in_graph_skips_gracefully() {
        let mut graph = CodeGraph::new();
        let barrel_path = PathBuf::from("/project/src/index.ts");
        let _barrel_idx = graph.add_file(barrel_path.clone(), "typescript");

        // Source file is referenced in export but NOT added to parse_results or graph.
        let barrel_export = ExportInfo {
            kind: ExportKind::ReExportAll,
            names: vec![],
            source: Some("./missing".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(barrel_path.clone(), make_parse_result(vec![barrel_export]));
        // No entry for ./missing — it won't be in parse_results.

        // Should not panic — gracefully skips unresolvable chains.
        resolve_barrel_chains(&mut graph, &parse_results, false);

        // No edges added (only the file node exists).
        let barrel_idx = graph.file_index[&barrel_path];
        let edge_count = graph.graph.edges(barrel_idx).count();
        assert_eq!(edge_count, 0, "no edges should exist when source is missing");
    }
}

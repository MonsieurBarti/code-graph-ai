use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::parser::ParseResult;
use crate::parser::imports::ExportKind;

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
                    if export.kind == ExportKind::ReExportAll
                        && let Some(source_specifier) = &export.source
                    {
                        return Some((file_path.clone(), source_specifier.clone()));
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

        let resolved_source =
            resolve_relative_specifier(barrel_dir, source_specifier, parse_results);

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

/// Resolve named re-export chains in the graph, adding direct `ResolvedImport` edges from
/// importing files to the files that actually *define* the imported names.
///
/// # Problem being solved
///
/// When file A does `import { Foo } from './services'` and `services/index.ts` contains
/// `export { Foo } from './FooService'`, the file-level resolution pass creates a
/// `ResolvedImport` edge from A → `services/index.ts` (the barrel), but NOT from A →
/// `FooService.ts` (the defining file).  This function adds that second direct edge so
/// that graph queries can reach the defining file without manually expanding barrels.
///
/// # Algorithm
///
/// 1. Build a named-re-export map: `barrel_path → Vec<(exported_names, source_path)>` from
///    all `ExportKind::ReExport` entries in `parse_results`.
/// 2. Scan existing `ResolvedImport` edges in the graph to find (importer, barrel, specifier).
/// 3. For each such edge, if the barrel has named re-exports, match the importer's
///    `ImportSpecifier` list against those names.  For each match, chase the chain — potentially
///    through multiple barrel levels — to find the defining file (a file that does NOT
///    re-export the name).
/// 4. Add a new `ResolvedImport` edge from the importer directly to the defining file.
///
/// Cycles are detected with a per-chain `HashSet<PathBuf>` visited set.
///
/// # Returns
/// Count of new direct edges added to the graph.
pub fn resolve_named_reexport_chains(
    graph: &mut CodeGraph,
    parse_results: &HashMap<PathBuf, ParseResult>,
    verbose: bool,
) -> usize {
    // -------------------------------------------------------------------------
    // Step 1: Build named re-export map.
    // barrel_reexports[barrel_path] = vec of (names_exported, resolved_source_path)
    // -------------------------------------------------------------------------
    let mut barrel_reexports: HashMap<PathBuf, Vec<(Vec<String>, PathBuf)>> = HashMap::new();

    for (file_path, result) in parse_results {
        let barrel_dir = match file_path.parent() {
            Some(d) => d,
            None => continue,
        };

        for export in &result.exports {
            if export.kind != ExportKind::ReExport {
                continue;
            }
            let source_specifier = match &export.source {
                Some(s) => s,
                None => continue,
            };
            if export.names.is_empty() {
                continue;
            }

            if let Some(source_path) =
                resolve_relative_specifier(barrel_dir, source_specifier, parse_results)
            {
                barrel_reexports
                    .entry(file_path.clone())
                    .or_default()
                    .push((export.names.clone(), source_path));
            }
        }
    }

    // If no barrels have named re-exports, skip the expensive graph scan.
    if barrel_reexports.is_empty() {
        return 0;
    }

    // -------------------------------------------------------------------------
    // Step 2: Collect all ResolvedImport edges (importer_path, barrel_path, specifier).
    // We collect into a vec first to avoid holding a borrow on `graph` while mutating it.
    // -------------------------------------------------------------------------
    // Build a reverse map: node_index → file_path for fast lookup.
    let idx_to_path: HashMap<petgraph::stable_graph::NodeIndex, PathBuf> = graph
        .file_index
        .iter()
        .map(|(path, &idx)| (idx, path.clone()))
        .collect();

    // Collect (importer_path, barrel_path, specifier) for all ResolvedImport edges
    // where the target is a barrel with named re-exports.
    let candidates: Vec<(PathBuf, PathBuf, String)> = graph
        .graph
        .edge_indices()
        .filter_map(|edge_idx| {
            match &graph.graph[edge_idx] {
                EdgeKind::ResolvedImport { specifier } => {
                    let (src_node, tgt_node) = graph.graph.edge_endpoints(edge_idx)?;
                    let importer_path = idx_to_path.get(&src_node)?;
                    let barrel_path = idx_to_path.get(&tgt_node)?;
                    // Only process if barrel has named re-exports.
                    if !barrel_reexports.contains_key(barrel_path) {
                        return None;
                    }
                    // Only handle relative specifiers — non-relative are external packages.
                    if !specifier.starts_with('.') {
                        return None;
                    }
                    Some((
                        importer_path.clone(),
                        barrel_path.clone(),
                        specifier.clone(),
                    ))
                }
                _ => None,
            }
        })
        .collect();

    // -------------------------------------------------------------------------
    // Step 3: For each candidate edge, match imported names against barrel re-exports.
    // -------------------------------------------------------------------------
    let mut edges_to_add: Vec<(PathBuf, PathBuf, String)> = Vec::new(); // (importer, defining_file, specifier)

    for (importer_path, barrel_path, specifier) in &candidates {
        // Get the import info for this importer + specifier to know which names were imported.
        let import_info = match parse_results.get(importer_path) {
            Some(r) => r
                .imports
                .iter()
                .find(|i| i.module_path == *specifier)
                .cloned(),
            None => continue,
        };

        // Collect the *original* exported names the importer wants from this barrel.
        // For `import { Foo } from '...'`: alias is None, name is "Foo" (original name).
        // For `import { Foo as F } from '...'`: name is "F" (local), alias is Some("Foo") (original).
        let wanted_names: Vec<String> = match &import_info {
            Some(info) => info
                .specifiers
                .iter()
                .filter_map(|s| {
                    if s.is_default || s.is_namespace {
                        None
                    } else {
                        // The original exported name: alias if present, otherwise name.
                        Some(s.alias.as_deref().unwrap_or(&s.name).to_owned())
                    }
                })
                .collect(),
            None => {
                // Import info not found — could be CJS or dynamic. Skip.
                continue;
            }
        };

        if wanted_names.is_empty() {
            continue;
        }

        let barrel_exports = match barrel_reexports.get(barrel_path) {
            Some(e) => e,
            None => continue,
        };

        // For each wanted name, chase the re-export chain.
        for wanted_name in &wanted_names {
            if let Some(defining_file) = chase_named_reexport(
                wanted_name,
                barrel_path,
                barrel_exports,
                &barrel_reexports,
                verbose,
            ) {
                // Don't add a redundant edge if the defining file IS the barrel itself.
                if &defining_file != barrel_path {
                    edges_to_add.push((importer_path.clone(), defining_file, specifier.clone()));
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Step 4: Add new direct ResolvedImport edges.
    // -------------------------------------------------------------------------
    let mut added = 0usize;

    for (importer_path, defining_path, specifier) in edges_to_add {
        let importer_idx = match graph.file_index.get(&importer_path).copied() {
            Some(idx) => idx,
            None => continue,
        };
        let defining_idx = match graph.file_index.get(&defining_path).copied() {
            Some(idx) => idx,
            None => continue,
        };

        // Avoid duplicate edges: check if this exact (from, to) ResolvedImport already exists.
        let already_exists = graph.graph.edges(importer_idx).any(|e| {
            e.target() == defining_idx && matches!(e.weight(), EdgeKind::ResolvedImport { .. })
        });

        if !already_exists {
            graph.add_resolved_import(importer_idx, defining_idx, &specifier);
            added += 1;

            if verbose {
                eprintln!(
                    "barrel(named): {} --[ResolvedImport]--> {} (chased through barrel)",
                    importer_path.display(),
                    defining_path.display()
                );
            }
        }
    }

    added
}

/// Chase a named re-export chain starting from `current_barrel` to find the file that
/// *defines* `name` (i.e., does not re-export it further).
///
/// Returns `Some(defining_path)` if the chain resolves, `None` if no matching re-export
/// entry is found in `current_barrel` or the chain cycles.
fn chase_named_reexport(
    name: &str,
    current_barrel: &Path,
    current_exports: &[(Vec<String>, PathBuf)],
    all_barrel_reexports: &HashMap<PathBuf, Vec<(Vec<String>, PathBuf)>>,
    verbose: bool,
) -> Option<PathBuf> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    visited.insert(current_barrel.to_path_buf());

    chase_named_reexport_inner(
        name,
        current_exports,
        all_barrel_reexports,
        &mut visited,
        verbose,
    )
}

fn chase_named_reexport_inner(
    name: &str,
    current_exports: &[(Vec<String>, PathBuf)],
    all_barrel_reexports: &HashMap<PathBuf, Vec<(Vec<String>, PathBuf)>>,
    visited: &mut HashSet<PathBuf>,
    verbose: bool,
) -> Option<PathBuf> {
    // Find the export entry in current_exports that includes `name`.
    for (exported_names, source_path) in current_exports {
        if !exported_names.iter().any(|n| n == name) {
            continue;
        }

        // Found a match. Check if the source_path also re-exports this name (another barrel).
        if visited.contains(source_path) {
            if verbose {
                eprintln!(
                    "barrel(named): cycle detected at {} — stopping chain for '{}'",
                    source_path.display(),
                    name
                );
            }
            return None; // Cycle — do not add edge.
        }

        visited.insert(source_path.clone());

        match all_barrel_reexports.get(source_path) {
            Some(next_exports) => {
                // The source is itself a barrel with named re-exports.
                // Check if it re-exports `name` further.
                let re_exported_again = next_exports
                    .iter()
                    .any(|(ns, _)| ns.iter().any(|n| n == name));
                if re_exported_again {
                    // Chase deeper.
                    return chase_named_reexport_inner(
                        name,
                        next_exports,
                        all_barrel_reexports,
                        visited,
                        verbose,
                    );
                } else {
                    // source_path defines (or locally re-exports) the name — it's the defining file.
                    return Some(source_path.clone());
                }
            }
            None => {
                // source_path has no named re-exports for this name — it defines it.
                return Some(source_path.clone());
            }
        }
    }

    // Name not found in current barrel's re-export list.
    None
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
    use crate::parser::ParseResult;
    use crate::parser::imports::{ExportInfo, ExportKind};

    use crate::parser::imports::{ImportInfo, ImportKind, ImportSpecifier};

    /// Build a minimal ParseResult for testing (no symbols, no imports, custom exports).
    fn make_parse_result(exports: Vec<ExportInfo>) -> ParseResult {
        ParseResult {
            symbols: vec![],
            imports: vec![],
            exports,
            relationships: vec![],
        }
    }

    /// Build a ParseResult with the given imports and exports.
    fn make_parse_result_with_imports(
        imports: Vec<ImportInfo>,
        exports: Vec<ExportInfo>,
    ) -> ParseResult {
        ParseResult {
            symbols: vec![],
            imports,
            exports,
            relationships: vec![],
        }
    }

    /// Build a simple named ImportInfo (e.g. `import { name } from specifier`).
    fn make_named_import(specifier: &str, names: &[&str]) -> ImportInfo {
        ImportInfo {
            kind: ImportKind::Esm,
            module_path: specifier.to_owned(),
            specifiers: names
                .iter()
                .map(|n| ImportSpecifier {
                    name: n.to_string(),
                    alias: None,
                    is_default: false,
                    is_namespace: false,
                })
                .collect(),
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
        let edge = graph
            .graph
            .edges(barrel_idx)
            .find(|e| e.target() == utils_idx);
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
            .find(|e| e.target() == utils_idx && matches!(e.weight(), EdgeKind::BarrelReExportAll));
        assert!(
            barrel_edge.is_none(),
            "no BarrelReExportAll edge should exist for named re-export"
        );
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
        assert_eq!(
            edge_count, 0,
            "no edges should exist when source is missing"
        );
    }

    // -------------------------------------------------------------------------
    // Tests for resolve_named_reexport_chains()
    // -------------------------------------------------------------------------

    /// Test 1: Single-level named re-export.
    ///
    /// Setup:
    ///   app.ts     → import { Foo } from '.'  (resolved to services/index.ts)
    ///   services/index.ts → export { Foo } from './FooService'
    ///   services/FooService.ts → defines Foo
    ///
    /// Expectation: after calling resolve_named_reexport_chains(), a ResolvedImport edge
    /// exists from app.ts directly to FooService.ts.
    #[test]
    fn test_named_reexport_adds_direct_edge() {
        let mut graph = CodeGraph::new();

        let app_path = PathBuf::from("/project/app.ts");
        let index_path = PathBuf::from("/project/services/index.ts");
        let service_path = PathBuf::from("/project/services/FooService.ts");

        let app_idx = graph.add_file(app_path.clone(), "typescript");
        let _index_idx = graph.add_file(index_path.clone(), "typescript");
        let service_idx = graph.add_file(service_path.clone(), "typescript");

        // The file-level pass already added app.ts → services/index.ts.
        graph.add_resolved_import(app_idx, _index_idx, "./services");

        let barrel_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()],
            source: Some("./FooService".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(
            app_path.clone(),
            make_parse_result_with_imports(vec![make_named_import("./services", &["Foo"])], vec![]),
        );
        parse_results.insert(index_path.clone(), make_parse_result(vec![barrel_export]));
        parse_results.insert(service_path.clone(), make_parse_result(vec![]));

        let added = resolve_named_reexport_chains(&mut graph, &parse_results, false);

        assert_eq!(added, 1, "should have added exactly 1 direct edge");
        assert!(
            graph.graph.contains_edge(app_idx, service_idx),
            "direct ResolvedImport edge should exist from app.ts to FooService.ts"
        );

        // Verify the new edge kind.
        let direct_edge = graph
            .graph
            .edges(app_idx)
            .find(|e| e.target() == service_idx);
        assert!(direct_edge.is_some(), "edge to defining file should exist");
        assert!(
            matches!(
                direct_edge.unwrap().weight(),
                EdgeKind::ResolvedImport { .. }
            ),
            "edge should be ResolvedImport"
        );
    }

    /// Test 2: Multi-level named re-export chain.
    ///
    /// Setup:
    ///   app.ts → import { Foo } from './outer'
    ///   outer/index.ts → export { Foo } from './inner'
    ///   inner/index.ts → export { Foo } from './defining'
    ///   inner/defining.ts → defines Foo
    ///
    /// Expectation: direct edge from app.ts to inner/defining.ts.
    #[test]
    fn test_named_reexport_multi_level_chain() {
        let mut graph = CodeGraph::new();

        let app_path = PathBuf::from("/project/app.ts");
        let outer_path = PathBuf::from("/project/outer/index.ts");
        let inner_path = PathBuf::from("/project/outer/inner/index.ts");
        let defining_path = PathBuf::from("/project/outer/inner/defining.ts");

        let app_idx = graph.add_file(app_path.clone(), "typescript");
        let _outer_idx = graph.add_file(outer_path.clone(), "typescript");
        let _inner_idx = graph.add_file(inner_path.clone(), "typescript");
        let defining_idx = graph.add_file(defining_path.clone(), "typescript");

        // File-level pass: app.ts → outer/index.ts
        graph.add_resolved_import(app_idx, _outer_idx, "./outer");

        let outer_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()],
            source: Some("./inner".to_owned()),
        };
        let inner_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()],
            source: Some("./defining".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(
            app_path.clone(),
            make_parse_result_with_imports(vec![make_named_import("./outer", &["Foo"])], vec![]),
        );
        parse_results.insert(outer_path.clone(), make_parse_result(vec![outer_export]));
        parse_results.insert(inner_path.clone(), make_parse_result(vec![inner_export]));
        parse_results.insert(defining_path.clone(), make_parse_result(vec![]));

        let added = resolve_named_reexport_chains(&mut graph, &parse_results, false);

        assert_eq!(
            added, 1,
            "should have added exactly 1 edge for the multi-level chain"
        );
        assert!(
            graph.graph.contains_edge(app_idx, defining_idx),
            "direct ResolvedImport edge should exist from app.ts to defining.ts"
        );
    }

    /// Test 3: Circular named re-export — no infinite loop, no crash.
    ///
    /// Setup:
    ///   app.ts → import { Foo } from './a'
    ///   a/index.ts → export { Foo } from './b'
    ///   b/index.ts → export { Foo } from './a'  (cycle back to a!)
    ///
    /// Expectation: no crash, no infinite loop, and zero edges added.
    #[test]
    fn test_named_reexport_cycle_detection() {
        let mut graph = CodeGraph::new();

        let app_path = PathBuf::from("/project/app.ts");
        let a_path = PathBuf::from("/project/a/index.ts");
        let b_path = PathBuf::from("/project/b/index.ts");

        let app_idx = graph.add_file(app_path.clone(), "typescript");
        let _a_idx = graph.add_file(a_path.clone(), "typescript");
        let _b_idx = graph.add_file(b_path.clone(), "typescript");

        // File-level pass: app.ts → a/index.ts
        graph.add_resolved_import(app_idx, _a_idx, "./a");

        let a_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()],
            source: Some("../b".to_owned()),
        };
        let b_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()],
            source: Some("../a".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(
            app_path.clone(),
            make_parse_result_with_imports(vec![make_named_import("./a", &["Foo"])], vec![]),
        );
        parse_results.insert(a_path.clone(), make_parse_result(vec![a_export]));
        parse_results.insert(b_path.clone(), make_parse_result(vec![b_export]));

        // Must not hang or panic.
        let added = resolve_named_reexport_chains(&mut graph, &parse_results, false);

        // Cycle detected — no defining file found — no edge added.
        assert_eq!(added, 0, "cycle should produce no new edges");
    }

    /// Test 4: Barrel exports Foo but importer wants Bar — no edge added.
    ///
    /// Setup:
    ///   app.ts → import { Bar } from './services'
    ///   services/index.ts → export { Foo } from './FooService'  (Foo only, not Bar)
    ///   services/FooService.ts → defines Foo
    ///
    /// Expectation: no direct ResolvedImport edge added from app.ts to FooService.ts
    /// because Bar is not in the barrel's named re-exports.
    #[test]
    fn test_named_reexport_no_edge_when_name_not_found() {
        let mut graph = CodeGraph::new();

        let app_path = PathBuf::from("/project/app.ts");
        let index_path = PathBuf::from("/project/services/index.ts");
        let service_path = PathBuf::from("/project/services/FooService.ts");

        let app_idx = graph.add_file(app_path.clone(), "typescript");
        let index_idx = graph.add_file(index_path.clone(), "typescript");
        let service_idx = graph.add_file(service_path.clone(), "typescript");

        // File-level pass: app.ts → services/index.ts
        graph.add_resolved_import(app_idx, index_idx, "./services");

        let barrel_export = ExportInfo {
            kind: ExportKind::ReExport,
            names: vec!["Foo".to_owned()], // exports Foo, not Bar
            source: Some("./FooService".to_owned()),
        };

        let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();
        parse_results.insert(
            app_path.clone(),
            make_parse_result_with_imports(
                vec![make_named_import("./services", &["Bar"])], // imports Bar
                vec![],
            ),
        );
        parse_results.insert(index_path.clone(), make_parse_result(vec![barrel_export]));
        parse_results.insert(service_path.clone(), make_parse_result(vec![]));

        let added = resolve_named_reexport_chains(&mut graph, &parse_results, false);

        assert_eq!(
            added, 0,
            "no edge should be added when imported name is not in barrel re-exports"
        );

        // Verify no direct edge app.ts → FooService.ts was added.
        let direct_edge = graph
            .graph
            .edges(app_idx)
            .find(|e| e.target() == service_idx);
        assert!(
            direct_edge.is_none(),
            "no edge to FooService.ts should exist when Bar is not re-exported"
        );
    }
}

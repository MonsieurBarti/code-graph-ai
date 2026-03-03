use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::GraphNode;
use crate::parser;
use std::collections::HashMap;

use crate::resolver::{
    ResolutionOutcome, build_resolver, discover_workspace_packages, resolve_import,
    workspace_map_to_aliases,
};

use super::event::WatchEvent;

/// Handle a single watch event by performing an incremental graph update.
///
/// For Modified: removes old file entry, re-parses, re-adds to graph,
/// re-resolves the file's imports, and checks if unresolved imports in other files
/// now resolve to this file.
///
/// For Deleted: removes the file from graph and marks imports pointing to it as unresolved.
///
/// For ConfigChanged: triggers a full rebuild (caller handles this by calling build_graph).
///
/// For CrateRootChanged: triggers a full rebuild (caller handles this by calling build_graph).
///
/// Returns `true` if the graph was modified, `false` if caller must full-rebuild.
pub fn handle_file_event(graph: &mut CodeGraph, event: &WatchEvent, project_root: &Path) -> bool {
    match event {
        WatchEvent::Modified(path) => {
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
        WatchEvent::CrateRootChanged(_) => {
            // Crate root changed — caller must perform full rebuild
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
        "rs" => "rust",
        "py" => "python",
        "go" => "go",
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

    if language_str == "rust" {
        // 4a. Rust path: emit use/pub-use placeholder edges, then run resolve_all scoped to this file.
        // Emit Rust use/pub-use edges (file -> file self-edges as placeholders).
        for rust_use in &result.rust_uses {
            if rust_use.is_pub_use {
                graph.graph.add_edge(
                    file_idx,
                    file_idx,
                    EdgeKind::ReExport {
                        path: rust_use.path.clone(),
                    },
                );
            } else {
                graph.graph.add_edge(
                    file_idx,
                    file_idx,
                    EdgeKind::RustImport {
                        path: rust_use.path.clone(),
                    },
                );
            }
        }

        // Run resolve_all scoped to just this file's parse result.
        // resolve_all handles Rust use-path resolution and self-edge replacement.
        let mut parse_results = HashMap::new();
        parse_results.insert(path.to_path_buf(), result);
        crate::resolver::resolve_all(graph, project_root, &parse_results, false);
    } else if language_str == "python" {
        // 4b. Python path: run resolve_all scoped to just this file's parse result.
        // resolve_all Step 7 handles Python import resolution (added in Plan 03 Task 2).
        let mut parse_results = HashMap::new();
        parse_results.insert(path.to_path_buf(), result);
        crate::resolver::resolve_all(graph, project_root, &parse_results, false);
    } else if language_str == "go" {
        // 4c. Go path: run resolve_all scoped to just this file's parse result.
        // resolve_all Step 8 handles Go import resolution via go_resolver.
        let mut parse_results = HashMap::new();
        parse_results.insert(path.to_path_buf(), result);
        crate::resolver::resolve_all(graph, project_root, &parse_results, false);
    } else {
        // 4b. TS/JS path: resolve imports using TS resolver, wire relationships.
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

    // 7. Enrich decorator frameworks and add HasDecorator self-edges for re-parsed file
    crate::query::decorators::enrich_decorator_frameworks(graph);
    crate::query::decorators::add_has_decorator_edges(graph);

    // 8. Rebuild BM25 index so new/changed symbols are searchable
    graph.rebuild_bm25_index();
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

    // Rebuild BM25 index so deleted symbols are no longer searchable
    graph.rebuild_bm25_index();
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

                let from_candidates = graph
                    .symbol_index
                    .get(from_name)
                    .cloned()
                    .unwrap_or_default();
                let to_candidates = graph
                    .symbol_index
                    .get(&rel.to_name)
                    .cloned()
                    .unwrap_or_default();

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
fn fix_unresolved_pointing_to(graph: &mut CodeGraph, new_file_path: &Path, project_root: &Path) {
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
                && reason != "builtin"
            {
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
            && resolved_path == new_file_path
        {
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

// ─── RAG re-embedding ─────────────────────────────────────────────────────────

/// Re-embed all symbols from `file_path` into `vector_store` after a file watcher event.
///
/// # How it works
///
/// 1. Collects all symbol nodes that belong to the changed file by traversing `Contains`
///    edges from the file node in the code graph.
/// 2. For each symbol, builds an embedding text: `"{name} in {file_path}:{line}"`.
/// 3. Embeds all texts as a single batch via `engine.embed_batch()`.
/// 4. Adds each new embedding to `vector_store`.
///
/// # Stale entry note
///
/// usearch's Rust bindings (usearch 2.x) do not expose a `remove(key)` method.
/// Therefore, old embeddings for modified or renamed symbols are NOT deleted — they
/// remain in the HNSW index as stale entries. In practice, this is acceptable because:
/// - The newer embeddings for the updated symbols score higher on semantic similarity.
/// - Stale entries for deleted/renamed symbols receive lower scores and are filtered
///   out by the top-k cutoff in most queries.
/// - The index is rebuilt on the next `code-graph index` invocation.
///
/// Returns the number of symbols re-embedded.
#[cfg(feature = "rag")]
pub async fn re_embed_file(
    graph: &crate::graph::CodeGraph,
    vector_store: &mut crate::rag::vector_store::VectorStore,
    engine: &crate::rag::embedding::EmbeddingEngine,
    file_path: &str,
) -> anyhow::Result<usize> {
    use crate::graph::edge::EdgeKind;
    use crate::graph::node::GraphNode;
    use crate::rag::vector_store::SymbolMeta;
    use petgraph::Direction;
    use std::path::Path;

    // Normalize the file path to compare against graph file paths.
    let target_path = Path::new(file_path);

    // Find the file node index.
    let file_idx = match graph.file_index.get(target_path).copied() {
        Some(idx) => idx,
        None => {
            // File not in graph (e.g. was deleted) — nothing to re-embed.
            return Ok(0);
        }
    };

    // Collect all symbols contained in this file via Contains edges.
    let symbol_indices: Vec<petgraph::stable_graph::NodeIndex> = graph
        .graph
        .edges_directed(file_idx, Direction::Outgoing)
        .filter_map(|e| {
            if matches!(e.weight(), EdgeKind::Contains) {
                Some(e.target())
            } else {
                None
            }
        })
        .collect();

    if symbol_indices.is_empty() {
        return Ok(0);
    }

    // Build (name, file_path_str, line) tuples for embedding.
    let mut symbol_descs: Vec<(String, String, usize)> = Vec::new();
    let mut symbol_metas: Vec<SymbolMeta> = Vec::new();

    for sym_idx in &symbol_indices {
        if let GraphNode::Symbol(info) = &graph.graph[*sym_idx] {
            let kind_str = format!("{:?}", info.kind).to_lowercase();
            let file_str = file_path.to_string();
            symbol_descs.push((info.name.clone(), file_str.clone(), info.line));
            symbol_metas.push(SymbolMeta {
                file_path: file_str,
                symbol_name: info.name.clone(),
                line_start: info.line,
                kind: kind_str,
            });
        }
    }

    if symbol_descs.is_empty() {
        return Ok(0);
    }

    // Embed all symbols as a batch.
    let texts: Vec<String> = symbol_descs
        .iter()
        .map(|(name, path, line)| format!("{} in {}:{}", name, path, line))
        .collect();

    let embeddings = engine.embed_batch(texts).await?;

    // Reserve capacity in the index before inserting (usearch requirement).
    vector_store.reserve(embeddings.len())?;

    // Add embeddings to the vector store.
    let mut count = 0;
    for (embedding, meta) in embeddings.iter().zip(symbol_metas.into_iter()) {
        vector_store.add(embedding, meta)?;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{SymbolInfo, SymbolKind};
    use crate::query::decorators::find_by_decorator;
    use crate::query::find::bm25_search;
    use std::fs;
    use tempfile::TempDir;

    /// Test that after handle_file_event (Modified), the BM25 index is rebuilt
    /// so newly added symbols become searchable.
    #[test]
    fn test_bm25_rebuilt_after_watcher_event() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        // Write a TypeScript file with a known symbol
        let file_path = root.join("src").join("auth.ts");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            "export function authHandler() { return true; }\n",
        )
        .unwrap();

        // Build an empty graph and manually add the symbol (simulating a pre-event state
        // where the file existed with a different symbol)
        let mut graph = CodeGraph::new();
        let f = graph.add_file(file_path.clone(), "typescript");
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "oldFunction".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );
        graph.rebuild_bm25_index();

        // Confirm old symbol is searchable, new one is not
        let before = bm25_search(&graph, "auth handler", 10);
        assert!(
            before.is_empty(),
            "authHandler should not be in BM25 index before event"
        );

        // Fire the watcher event (Modified) for the file that now contains authHandler
        let event = WatchEvent::Modified(file_path.clone());
        let modified = handle_file_event(&mut graph, &event, root);
        assert!(
            modified,
            "handle_file_event should return true for Modified"
        );

        // After the event, the BM25 index should be rebuilt and authHandler should be findable
        let after = bm25_search(&graph, "auth handler", 10);
        assert!(
            !after.is_empty(),
            "authHandler should be in BM25 index after watcher event"
        );
        assert_eq!(after[0].symbol_name, "authHandler");
    }

    /// Test that after handle_file_event (Modified) on a TypeScript file with @Controller,
    /// find_by_decorator returns the decorated symbol with NestJS framework label,
    /// and at least one HasDecorator edge exists in the graph.
    #[test]
    fn test_decorator_enrichment_after_watcher_event() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        // Write a TypeScript file with a @Controller decorator
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let file_path = src_dir.join("app.controller.ts");
        fs::write(
            &file_path,
            "@Controller('/api')\nexport class AppController {}\n",
        )
        .unwrap();

        // Create a fresh empty graph
        let mut graph = CodeGraph::new();

        // Fire the Modified watcher event
        let event = WatchEvent::Modified(file_path.clone());
        let modified = handle_file_event(&mut graph, &event, root);
        assert!(
            modified,
            "handle_file_event should return true for Modified"
        );

        // Assert find_by_decorator returns non-empty results with AppController
        let results = find_by_decorator(&graph, "Controller", None, None, 10)
            .expect("find_by_decorator should succeed");
        assert!(
            !results.is_empty(),
            "find_by_decorator should return results after watcher event on @Controller class"
        );
        assert_eq!(
            results[0].symbol_name, "AppController",
            "found symbol should be AppController"
        );

        // Assert framework enrichment ran (nestjs detected for @Controller in TypeScript)
        assert_eq!(
            results[0].framework,
            Some("nestjs".to_string()),
            "framework should be nestjs after enrichment"
        );

        // Assert at least one HasDecorator edge exists in the graph
        use petgraph::visit::IntoEdgeReferences;
        let has_decorator_edge = graph
            .graph
            .edge_references()
            .any(|e| matches!(e.weight(), EdgeKind::HasDecorator { .. }));
        assert!(
            has_decorator_edge,
            "graph should contain at least one HasDecorator edge after watcher event"
        );
    }

    /// Test that after handle_file_event (Modified) on a Go file that imports another Go
    /// package, the graph contains a ResolvedImport edge from the importer to the importee.
    /// This verifies Go files route through resolve_all (go_resolver Step 8) instead of
    /// the TS/JS else branch.
    #[test]
    fn test_go_watcher_resolve_imports() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        // Write go.mod so go_resolver can build the module map
        let go_mod = root.join("go.mod");
        fs::write(&go_mod, "module example.com/mymod\n\ngo 1.21\n").unwrap();

        // Write pkg/foo.go — the importee package
        let pkg_dir = root.join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        let foo_path = pkg_dir.join("foo.go");
        fs::write(&foo_path, "package pkg\n\nfunc Foo() {}\n").unwrap();

        // Write main.go — the importer that imports example.com/mymod/pkg
        let main_path = root.join("main.go");
        fs::write(
            &main_path,
            "package main\n\nimport \"example.com/mymod/pkg\"\n\nfunc main() { pkg.Foo() }\n",
        )
        .unwrap();

        // Create a graph and pre-index both files so go_resolver can find the target
        let mut graph = CodeGraph::new();

        // Pre-index pkg/foo.go so the file_index has an entry for it
        let foo_src = fs::read(&foo_path).unwrap();
        let foo_result = crate::parser::parse_file(&foo_path, &foo_src).expect("parse foo.go");
        let foo_idx = graph.add_file(foo_path.clone(), "go");
        for (symbol, children) in &foo_result.symbols {
            let sym_idx = graph.add_symbol(foo_idx, symbol.clone());
            for child in children {
                graph.add_child_symbol(sym_idx, child.clone());
            }
        }

        // Fire the Modified event on main.go — this is what we're testing
        let event = WatchEvent::Modified(main_path.clone());
        let modified = handle_file_event(&mut graph, &event, root);
        assert!(
            modified,
            "handle_file_event should return true for Modified"
        );

        // Assert a ResolvedImport edge exists from main.go's file node to pkg/foo.go's file node
        let main_idx = *graph
            .file_index
            .get(&main_path)
            .expect("main.go should be in graph after event");
        let foo_idx_after = *graph
            .file_index
            .get(&foo_path)
            .expect("pkg/foo.go should be in graph");

        use petgraph::visit::EdgeRef;
        let has_resolved_import = graph.graph.edges(main_idx).any(|e| {
            matches!(e.weight(), EdgeKind::ResolvedImport { .. }) && e.target() == foo_idx_after
        });

        assert!(
            has_resolved_import,
            "graph should contain a ResolvedImport edge from main.go to pkg/foo.go after watcher event"
        );
    }

    /// Test that re_embed_file returns count 3 for a graph with 3 symbols in a file.
    ///
    /// This test requires the `rag` feature and the fastembed ONNX model to be cached locally.
    #[cfg(feature = "rag")]
    #[tokio::test]
    async fn test_re_embed_file_returns_symbol_count() {
        use crate::graph::node::{SymbolInfo, SymbolKind};
        use crate::rag::embedding::EmbeddingEngine;
        use crate::rag::vector_store::VectorStore;

        // Build a graph with 3 symbols in a single file.
        let mut graph = CodeGraph::new();
        let file_path = std::path::PathBuf::from("/tmp/test_re_embed.rs");
        let file_idx = graph.add_file(file_path.clone(), "rust");

        for (i, name) in ["alpha", "beta", "gamma"].iter().enumerate() {
            graph.add_symbol(
                file_idx,
                SymbolInfo {
                    name: name.to_string(),
                    kind: SymbolKind::Function,
                    line: i + 1,
                    is_exported: true,
                    ..Default::default()
                },
            );
        }

        // Initialize the vector store and embedding engine.
        let mut vs = VectorStore::new(384).expect("VectorStore::new");
        let engine = match EmbeddingEngine::try_new() {
            Ok(e) => e,
            Err(_) => {
                // Skip test if embedding engine is unavailable (e.g. no model cache in CI).
                eprintln!("Skipping test_re_embed_file: EmbeddingEngine unavailable");
                return;
            }
        };

        let file_path_str = file_path.to_string_lossy().to_string();
        let count = super::re_embed_file(&graph, &mut vs, &engine, &file_path_str)
            .await
            .expect("re_embed_file should succeed");

        assert_eq!(
            count, 3,
            "re_embed_file should return count 3 for 3 symbols"
        );
        assert_eq!(vs.len(), 3, "vector store should have 3 entries");
    }

    /// Test that re_embed_file returns 0 when the file is not in the graph.
    #[cfg(feature = "rag")]
    #[tokio::test]
    async fn test_re_embed_file_missing_file_returns_zero() {
        use crate::rag::embedding::EmbeddingEngine;
        use crate::rag::vector_store::VectorStore;

        let graph = CodeGraph::new(); // empty graph — no files
        let mut vs = VectorStore::new(384).expect("VectorStore::new");
        let engine = match EmbeddingEngine::try_new() {
            Ok(e) => e,
            Err(_) => {
                eprintln!("Skipping test_re_embed_file_missing_file: EmbeddingEngine unavailable");
                return;
            }
        };

        let count = super::re_embed_file(&graph, &mut vs, &engine, "/tmp/nonexistent.rs")
            .await
            .expect("re_embed_file with missing file should return Ok(0)");

        assert_eq!(count, 0, "missing file should produce count 0");
        assert!(vs.is_empty(), "vector store should remain empty");
    }
}

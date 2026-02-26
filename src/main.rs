mod cache;
mod cli;
mod config;
mod export;
mod graph;
mod language;
mod mcp;
mod output;
mod parser;
mod query;
mod resolver;
mod walker;
mod watcher;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use petgraph::visit::EdgeRef;
use rayon::prelude::*;

use cli::{Cli, Commands};
use config::CodeGraphConfig;
use graph::{CodeGraph, edge::EdgeKind, node::SymbolKind};
use language::LanguageKind;
use output::{IndexStats, print_summary};
use parser::ParseResult;
use parser::imports::ImportKind;
use graph::node::classify_file_kind;
use walker::{walk_non_parsed_files, walk_project};

/// Rust-specific symbol counts, separated from TS/JS counts for mixed-language projects.
struct RustSymbolCounts {
    fns: usize,
    structs: usize,
    enums: usize,
    traits: usize,
    impl_methods: usize,
    type_aliases: usize,
    consts: usize,
    statics: usize,
    macros: usize,
}

/// Count symbols belonging to Rust files (language == "rust") in the graph.
fn count_rust_symbols(graph: &CodeGraph) -> RustSymbolCounts {
    use graph::node::GraphNode;
    use petgraph::Direction;

    let mut counts = RustSymbolCounts {
        fns: 0,
        structs: 0,
        enums: 0,
        traits: 0,
        impl_methods: 0,
        type_aliases: 0,
        consts: 0,
        statics: 0,
        macros: 0,
    };

    for idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(ref s) = graph.graph[idx] {
            // Check if this symbol belongs to a Rust file via a Contains edge.
            let in_rust_file = graph
                .graph
                .edges_directed(idx, Direction::Incoming)
                .any(|e| {
                    if let EdgeKind::Contains = e.weight()
                        && let GraphNode::File(ref f) = graph.graph[e.source()]
                    {
                        return f.language == "rust";
                    }
                    false
                });
            if !in_rust_file {
                // Check ChildOf parent path (trait method children live under parent symbols)
                let parent_in_rust =
                    graph
                        .graph
                        .edges_directed(idx, Direction::Outgoing)
                        .any(|e| {
                            if let EdgeKind::ChildOf = e.weight() {
                                let parent = e.target();
                                graph
                                    .graph
                                    .edges_directed(parent, Direction::Incoming)
                                    .any(|pe| {
                                        if let EdgeKind::Contains = pe.weight()
                                            && let GraphNode::File(ref f) = graph.graph[pe.source()]
                                        {
                                            return f.language == "rust";
                                        }
                                        false
                                    })
                            } else {
                                false
                            }
                        });
                if !parent_in_rust {
                    continue;
                }
            }
            match s.kind {
                SymbolKind::Function => counts.fns += 1,
                SymbolKind::Struct => counts.structs += 1,
                SymbolKind::Enum => counts.enums += 1,
                SymbolKind::Trait => counts.traits += 1,
                SymbolKind::ImplMethod => counts.impl_methods += 1,
                SymbolKind::TypeAlias => counts.type_aliases += 1,
                SymbolKind::Const => counts.consts += 1,
                SymbolKind::Static => counts.statics += 1,
                SymbolKind::Macro => counts.macros += 1,
                _ => {}
            }
        }
    }
    counts
}

/// Populate `FileInfo.crate_name` for all Rust files in the graph.
///
/// Calls `discover_rust_workspace_members` to get crate name → root file mappings, then builds
/// a `RustModTree` per crate, and for each file in the graph whose path appears in a mod tree,
/// sets the `crate_name` field on the corresponding `FileInfo` node.
///
/// This is called AFTER graph population (so all file nodes exist) and BEFORE `resolve_all`
/// (so the resolver can use crate_name for classification).
pub(crate) fn populate_rust_crate_names(graph: &mut CodeGraph, project_root: &Path) {
    use graph::node::GraphNode;
    use resolver::cargo_workspace::discover_rust_workspace_members;
    use resolver::rust_mod_tree::build_mod_tree;

    let workspace_members = discover_rust_workspace_members(project_root);
    if workspace_members.is_empty() {
        return;
    }

    // Build file → crate_name map from all mod trees.
    let mut file_to_crate: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();
    for (crate_name, crate_root) in &workspace_members {
        let tree = build_mod_tree(crate_name, crate_root);
        // mod_map: String (module path) → PathBuf (file); iterate values for file paths.
        for file_path in tree.mod_map.values() {
            file_to_crate.insert(file_path.clone(), crate_name.clone());
        }
        // reverse_map: PathBuf (file) → String (module path); iterate keys for file paths.
        for file_path in tree.reverse_map.keys() {
            file_to_crate
                .entry(file_path.clone())
                .or_insert_with(|| crate_name.clone());
        }
    }

    // Apply crate_name to matching FileInfo nodes in the graph.
    // Collect (index, path) pairs first to avoid simultaneous mutable + immutable borrow.
    let rust_file_nodes: Vec<(petgraph::stable_graph::NodeIndex, PathBuf)> = graph
        .graph
        .node_indices()
        .filter_map(|idx| {
            if let GraphNode::File(ref fi) = graph.graph[idx]
                && fi.language == "rust"
            {
                return Some((idx, fi.path.clone()));
            }
            None
        })
        .collect();

    for (idx, file_path) in rust_file_nodes {
        if let Some(crate_name) = file_to_crate.get(&file_path)
            && let GraphNode::File(ref mut fi) = graph.graph[idx]
        {
            fi.crate_name = Some(crate_name.clone());
        }
    }
}

/// Parse a --language flag string into a canonical language string for use in filters.
///
/// Returns the canonical language string ("rust", "typescript", "javascript") or None.
/// Returns an error if the string is not a recognized language alias.
fn parse_language_filter(lang_str: Option<&str>) -> Result<Option<&'static str>> {
    match lang_str {
        None => Ok(None),
        Some(s) => match LanguageKind::from_str_loose(s) {
            Some(LanguageKind::Rust) => Ok(Some("rust")),
            Some(LanguageKind::TypeScript) => Ok(Some("typescript")),
            Some(LanguageKind::JavaScript) => Ok(Some("javascript")),
            None => anyhow::bail!(
                "unknown language '{}'. Valid: rust/rs, typescript/ts, javascript/js",
                s
            ),
        },
    }
}

/// Returns true if the file at `path` belongs to the given language string.
///
/// Determines language from file extension. Used for post-filtering results
/// in Refs, Impact, Circular, and Context commands.
fn file_language_matches(path: &Path, lang: &str) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match lang {
        "rust" => ext == "rs",
        "typescript" => matches!(ext, "ts" | "tsx"),
        "javascript" => matches!(ext, "js" | "jsx"),
        _ => false,
    }
}

/// Build the code graph for a project at `path` by walking, parsing, and resolving all files.
///
/// This is the shared pipeline used by all query subcommands. The Index command has its own
/// inline copy so it can also compute detailed stats without a second pass.
pub(crate) fn build_graph(path: &Path, verbose: bool) -> Result<CodeGraph> {
    let config = CodeGraphConfig::load(path);
    let files = walk_project(path, &config, verbose, None)?;

    // Phase 1: Parse all files in parallel (CPU-bound — rayon par_iter).
    let raw_results: Vec<(PathBuf, &'static str, ParseResult)> = files
        .par_iter()
        .filter_map(|file_path| {
            let source = std::fs::read(file_path).ok()?;
            let language_str: &'static str =
                match file_path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                    "ts" => "typescript",
                    "tsx" => "tsx",
                    "js" | "jsx" => "javascript",
                    "rs" => "rust",
                    _ => return None,
                };
            let result = parser::parse_file_parallel(file_path, &source).ok()?;
            Some((file_path.clone(), language_str, result))
        })
        .collect();

    // Phase 2: Insert into graph sequentially (petgraph is not Send).
    let mut graph = CodeGraph::new();
    let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

    for (file_path, language_str, result) in raw_results {
        let file_idx = graph.add_file(file_path.clone(), language_str);

        for (symbol, children) in &result.symbols {
            let sym_idx = graph.add_symbol(file_idx, symbol.clone());
            for child in children {
                graph.add_child_symbol(sym_idx, child.clone());
            }
        }

        // Emit Rust use/pub-use edges (file -> file self-edge as placeholder; Phase 9 resolves)
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

        if verbose {
            eprintln!(
                "  {} symbols, {} imports, {} exports from {}",
                result.symbols.len(),
                result.imports.len(),
                result.exports.len(),
                file_path.display()
            );
        }

        parse_results.insert(file_path, result);
    }

    // Populate crate_name on FileInfo for all Rust files.
    populate_rust_crate_names(&mut graph, path);

    resolver::resolve_all(&mut graph, path, &parse_results, verbose);

    // Phase 12: Discover and add non-parsed files as File nodes (no symbols, no imports).
    let non_parsed = walk_non_parsed_files(path, &config)?;
    for file_path in non_parsed {
        let kind = classify_file_kind(&file_path);
        graph.add_non_parsed_file(file_path, kind);
    }

    Ok(graph)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index {
            path,
            verbose,
            json,
            language,
        } => {
            // 1. Load config (always succeeds — defaults when file is absent).
            let config = CodeGraphConfig::load(&path);

            // 2. Parse --language flag values into a language filter set.
            // When --language is not specified, auto-detect from config files at project root.
            // The detected set is informational in Phase 7 — the walk still discovers all
            // supported extensions, and the post-walk counts confirm actual presence.
            let _detected_languages = language::detect_languages(&path);

            let allowed_languages: Option<HashSet<LanguageKind>> = if language.is_empty() {
                None // auto-detect: walk all supported extensions
            } else {
                let mut set = HashSet::new();
                for lang_str in &language {
                    match LanguageKind::from_str_loose(lang_str) {
                        Some(lk) => {
                            set.insert(lk);
                        }
                        None => anyhow::bail!(
                            "unknown language '{}'. Valid: typescript, javascript, rust (or ts, js, rs)",
                            lang_str
                        ),
                    }
                }
                Some(set)
            };

            // 3. Start timer.
            let start = std::time::Instant::now();

            // 4. Walk files.
            let files = walk_project(&path, &config, verbose, allowed_languages.as_ref())?;

            // 5. Compute per-language file counts from the walk result BEFORE parsing.
            // Rust files are counted but not parsed — they must not enter the parse pipeline.
            let ts_file_count = files
                .iter()
                .filter(|f| matches!(f.extension().and_then(|e| e.to_str()), Some("ts" | "tsx")))
                .count();
            let js_file_count = files
                .iter()
                .filter(|f| matches!(f.extension().and_then(|e| e.to_str()), Some("js" | "jsx")))
                .count();
            let rust_file_count = files
                .iter()
                .filter(|f| matches!(f.extension().and_then(|e| e.to_str()), Some("rs")))
                .count();

            // 6. Create graph.
            let mut graph = CodeGraph::new();

            // Import/export counts (accumulated across all files).
            let mut total_imports: usize = 0;
            let mut total_exports: usize = 0;
            let mut esm_imports: usize = 0;
            let mut cjs_imports: usize = 0;
            let mut dynamic_imports: usize = 0;
            let mut rust_use_count: usize = 0;
            let mut rust_pub_use_count: usize = 0;

            // Parse results map — retained for the resolution step.
            let mut parse_results: HashMap<PathBuf, ParseResult> = HashMap::new();

            // 7. Parse all TS/JS/RS files in parallel (rayon par_iter).
            let raw_results: Vec<(PathBuf, &'static str, ParseResult)> = files
                .par_iter()
                .filter_map(|file_path| {
                    let source = std::fs::read(file_path).ok()?;
                    let language_str: &'static str =
                        match file_path.extension().and_then(|e| e.to_str()).unwrap_or("") {
                            "ts" => "typescript",
                            "tsx" => "tsx",
                            "js" | "jsx" => "javascript",
                            "rs" => "rust",
                            _ => return None,
                        };
                    let result = parser::parse_file_parallel(file_path, &source).ok()?;
                    Some((file_path.clone(), language_str, result))
                })
                .collect();

            // skipped = files that couldn't be read or parsed.
            let skipped = files.len() - raw_results.len();

            // 8. Insert into graph sequentially + accumulate stats.
            for (file_path, language_str, result) in raw_results {
                // Add file node to graph.
                let file_idx = graph.add_file(file_path.clone(), language_str);

                // Add symbols (parent + child).
                for (symbol, children) in &result.symbols {
                    let sym_idx = graph.add_symbol(file_idx, symbol.clone());
                    for child in children {
                        graph.add_child_symbol(sym_idx, child.clone());
                    }
                }

                // Accumulate import/export counts.
                total_imports += result.imports.len();
                total_exports += result.exports.len();
                for imp in &result.imports {
                    match imp.kind {
                        ImportKind::Esm => esm_imports += 1,
                        ImportKind::Cjs => cjs_imports += 1,
                        ImportKind::DynamicImport => dynamic_imports += 1,
                    }
                }

                // Accumulate Rust use/pub-use counts and emit edges.
                for rust_use in &result.rust_uses {
                    if rust_use.is_pub_use {
                        rust_pub_use_count += 1;
                        graph.graph.add_edge(
                            file_idx,
                            file_idx,
                            EdgeKind::ReExport {
                                path: rust_use.path.clone(),
                            },
                        );
                    } else {
                        rust_use_count += 1;
                        graph.graph.add_edge(
                            file_idx,
                            file_idx,
                            EdgeKind::RustImport {
                                path: rust_use.path.clone(),
                            },
                        );
                    }
                }

                if verbose {
                    eprintln!(
                        "  {} symbols, {} imports, {} exports from {}",
                        result.symbols.len(),
                        result.imports.len(),
                        result.exports.len(),
                        file_path.display()
                    );
                }

                // Store parse result for the resolution pass.
                parse_results.insert(file_path, result);
            }

            // Populate crate_name on FileInfo for all Rust files.
            populate_rust_crate_names(&mut graph, &path);

            // 7. Resolve imports, barrel chains, and symbol relationships.
            let resolve_stats = resolver::resolve_all(&mut graph, &path, &parse_results, verbose);

            if verbose {
                eprintln!(
                    "  Resolution: {} resolved, {} external, {} unresolved, {} builtins",
                    resolve_stats.resolved,
                    resolve_stats.external,
                    resolve_stats.unresolved,
                    resolve_stats.builtin,
                );
                eprintln!(
                    "  Relationships: {} edges added",
                    resolve_stats.relationships_added
                );
            }

            // 8. Compute stats from graph.
            let elapsed_secs = start.elapsed().as_secs_f64();
            let breakdown: HashMap<SymbolKind, usize> = graph.symbols_by_kind();

            // Compute per-language symbol counts for Rust-specific fields.
            // Rust-only kinds (Struct, Trait, ImplMethod, Const, Static, Macro) map directly.
            // For shared kinds (Function, Enum, TypeAlias) we count only Rust-file symbols.
            let rust_symbol_counts = count_rust_symbols(&graph);

            let stats = IndexStats {
                file_count: graph.file_count(),
                functions: *breakdown.get(&SymbolKind::Function).unwrap_or(&0),
                classes: *breakdown.get(&SymbolKind::Class).unwrap_or(&0),
                interfaces: *breakdown.get(&SymbolKind::Interface).unwrap_or(&0),
                type_aliases: *breakdown.get(&SymbolKind::TypeAlias).unwrap_or(&0),
                enums: *breakdown.get(&SymbolKind::Enum).unwrap_or(&0),
                variables: *breakdown.get(&SymbolKind::Variable).unwrap_or(&0),
                components: *breakdown.get(&SymbolKind::Component).unwrap_or(&0),
                methods: *breakdown.get(&SymbolKind::Method).unwrap_or(&0),
                properties: *breakdown.get(&SymbolKind::Property).unwrap_or(&0),
                imports: total_imports,
                esm_imports,
                cjs_imports,
                dynamic_imports,
                exports: total_exports,
                skipped,
                elapsed_secs,
                resolved_imports: resolve_stats.resolved,
                unresolved_imports: resolve_stats.unresolved,
                external_packages: resolve_stats.external,
                builtin_modules: resolve_stats.builtin,
                relationship_edges: resolve_stats.relationships_added,
                ts_file_count,
                js_file_count,
                rust_file_count,
                rust_fns: rust_symbol_counts.fns,
                rust_structs: rust_symbol_counts.structs,
                rust_enums: rust_symbol_counts.enums,
                rust_traits: rust_symbol_counts.traits,
                rust_impl_methods: rust_symbol_counts.impl_methods,
                rust_type_aliases: rust_symbol_counts.type_aliases,
                rust_consts: rust_symbol_counts.consts,
                rust_statics: rust_symbol_counts.statics,
                rust_macros: rust_symbol_counts.macros,
                rust_use_statements: rust_use_count,
                rust_pub_use_reexports: rust_pub_use_count,
            };

            // 9. Print summary.
            print_summary(&stats, json);

            // 10. Save graph to disk cache for fast cold starts.
            if let Err(e) = cache::save_cache(&path, &graph)
                && verbose
            {
                eprintln!("  Cache save failed: {}", e);
            }
        }

        Commands::Find {
            path,
            symbol,
            case_insensitive,
            kind,
            file,
            format,
            language,
        } => {
            // Validate regex FIRST before the expensive index pipeline (Research Pitfall 4).
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let language_filter = parse_language_filter(language.as_deref())?;

            let graph = build_graph(&path, false)?;
            let results = query::find::find_symbol(
                &graph,
                &symbol,
                case_insensitive,
                &kind,
                file.as_deref(),
                &path,
                language_filter,
            )?;

            if results.is_empty() {
                if let Some(lang) = language_filter {
                    eprintln!(
                        "No {} symbols found. Run `code-graph stats` to see indexed languages.",
                        lang
                    );
                } else {
                    eprintln!("no symbols matching '{}' found", symbol);
                }
                std::process::exit(1);
            }

            query::output::format_find_results(&results, &format, &path);
        }

        Commands::Stats {
            path,
            format,
            language,
        } => {
            let language_filter = parse_language_filter(language.as_deref())?;
            let graph = build_graph(&path, false)?;
            let stats = query::stats::project_stats(&graph);
            query::output::format_stats(&stats, &format, language_filter);
        }

        Commands::Refs {
            path,
            symbol,
            case_insensitive,
            kind: _,
            file: _,
            format,
            language,
        } => {
            // Validate regex FIRST before the expensive index pipeline.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let language_filter = parse_language_filter(language.as_deref())?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            // Collect all matched NodeIndices.
            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let mut results = query::refs::find_refs(&graph, &symbol, &all_indices, &path);

            // Apply language filter as post-filter on file path extension.
            if let Some(lang) = language_filter {
                results.retain(|r| file_language_matches(&r.file_path, lang));
            }

            if results.is_empty() {
                if let Some(lang) = language_filter {
                    eprintln!(
                        "No {} references found. Run `code-graph stats` to see indexed languages.",
                        lang
                    );
                } else {
                    eprintln!("no references to '{}' found", symbol);
                }
            } else {
                query::output::format_refs_results(&results, &format, &path);
            }
        }

        Commands::Impact {
            path,
            symbol,
            case_insensitive,
            tree,
            format,
            language,
        } => {
            // Validate regex FIRST.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let language_filter = parse_language_filter(language.as_deref())?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            let all_indices: Vec<petgraph::stable_graph::NodeIndex> = matches
                .iter()
                .flat_map(|(_, indices)| indices.iter().copied())
                .collect();

            let mut results = query::impact::blast_radius(&graph, &all_indices, &path);

            // Apply language filter as post-filter on file path extension.
            if let Some(lang) = language_filter {
                results.retain(|r| file_language_matches(&r.file_path, lang));
            }

            query::output::format_impact_results(&results, &format, &path, tree);
        }

        Commands::Circular {
            path,
            format,
            language,
        } => {
            let language_filter = parse_language_filter(language.as_deref())?;

            let graph = build_graph(&path, false)?;
            let mut cycles = query::circular::find_circular(&graph, &path);

            // Apply language filter: retain cycles where all files match the language.
            if let Some(lang) = language_filter {
                cycles.retain(|c| c.files.iter().all(|f| file_language_matches(f, lang)));
            }

            if cycles.is_empty() {
                println!("no circular dependencies found");
            } else {
                query::output::format_circular_results(&cycles, &format, &path);
            }
        }

        Commands::Context {
            path,
            symbol,
            case_insensitive,
            format,
            language,
        } => {
            // Validate regex FIRST before the expensive index pipeline.
            regex::RegexBuilder::new(&symbol)
                .case_insensitive(case_insensitive)
                .build()
                .map_err(|e| anyhow::anyhow!("invalid symbol pattern '{}': {}", symbol, e))?;

            let language_filter = parse_language_filter(language.as_deref())?;

            let graph = build_graph(&path, false)?;
            let matches = query::find::match_symbols(&graph, &symbol, case_insensitive)?;

            if matches.is_empty() {
                eprintln!("no symbols matching '{}' found", symbol);
                std::process::exit(1);
            }

            // Build one SymbolContext per matched symbol name.
            let mut results: Vec<query::context::SymbolContext> = matches
                .iter()
                .map(|(name, indices)| query::context::symbol_context(&graph, name, indices, &path))
                .collect();

            // Apply language filter to context results: filter definition/reference file paths.
            if let Some(lang) = language_filter {
                for ctx in &mut results {
                    ctx.definitions
                        .retain(|d| file_language_matches(&d.file_path, lang));
                    ctx.references
                        .retain(|r| file_language_matches(&r.file_path, lang));
                    ctx.callers
                        .retain(|c| file_language_matches(&c.file_path, lang));
                    ctx.callees
                        .retain(|c| file_language_matches(&c.file_path, lang));
                }
                results.retain(|ctx| !ctx.definitions.is_empty());
            }

            if results.is_empty()
                && let Some(lang) = language_filter
            {
                eprintln!(
                    "No {} symbols found. Run `code-graph stats` to see indexed languages.",
                    lang
                );
                std::process::exit(1);
            }

            query::output::format_context_results(&results, &format, &path, &symbol);
        }

        Commands::Mcp { path, watch } => {
            let project_root = path.unwrap_or_else(|| {
                std::env::current_dir().expect("cannot determine current directory")
            });
            mcp::run(project_root, watch).await?;
        }

        Commands::Export {
            path,
            format,
            granularity,
            stdout,
            root,
            symbol,
            depth,
            exclude,
        } => {
            let graph = build_graph(&path, false)?;
            let params = export::model::ExportParams {
                format,
                granularity,
                root_filter: root,
                symbol_filter: symbol,
                depth,
                exclude_patterns: exclude,
                project_root: path.clone(),
                stdout,
            };
            let result = export::export_graph(&graph, &params)?;

            if stdout {
                print!("{}", result.content);
            } else {
                // Write to .code-graph/graph.{dot|mmd}
                let output_dir = path.join(".code-graph");
                std::fs::create_dir_all(&output_dir)?;
                let ext = match params.format {
                    export::model::ExportFormat::Dot => "dot",
                    export::model::ExportFormat::Mermaid => "mmd",
                };
                let output_path = output_dir.join(format!("graph.{}", ext));
                std::fs::write(&output_path, &result.content)?;
                // Summary to stderr (keeps stdout clean for --stdout piping).
                eprintln!(
                    "Exported {} nodes, {} edges to {}",
                    result.node_count,
                    result.edge_count,
                    output_path.display()
                );
            }

            // Print any advisory warnings from scale guards.
            for warning in &result.warnings {
                eprintln!("Warning: {}", warning);
            }
        }

        Commands::Watch { path } => {
            eprintln!("Indexing {}...", path.display());
            let mut graph = build_graph(&path, false)?;
            eprintln!(
                "Indexed {} files, {} symbols. Starting watcher...",
                graph.file_count(),
                graph.symbol_count()
            );

            // Save initial cache
            if let Err(e) = cache::save_cache(&path, &graph) {
                eprintln!("[cache] failed to save: {}", e);
            }

            // Start watcher
            let (handle, mut rx) = watcher::start_watcher(&path)
                .map_err(|e| anyhow::anyhow!("failed to start watcher: {}", e))?;

            // Keep handle alive — dropping it stops the watcher
            let _handle = handle;

            eprintln!("Watching for changes... (press Ctrl+C to stop)");

            // Process events — terminal status output goes to stderr (Phase 1 convention)
            while let Some(event) = rx.recv().await {
                match &event {
                    watcher::event::WatchEvent::Modified(p) => {
                        let start = std::time::Instant::now();
                        watcher::incremental::handle_file_event(&mut graph, &event, &path);
                        let elapsed = start.elapsed();
                        eprintln!(
                            "[watch] incremental: {} ({:.1}ms)",
                            p.strip_prefix(&path).unwrap_or(p).display(),
                            elapsed.as_secs_f64() * 1000.0,
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                    watcher::event::WatchEvent::Deleted(p) => {
                        watcher::incremental::handle_file_event(&mut graph, &event, &path);
                        eprintln!(
                            "[watch] deleted: {} ({} files, {} symbols)",
                            p.strip_prefix(&path).unwrap_or(p).display(),
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                    watcher::event::WatchEvent::ConfigChanged => {
                        eprintln!("[watch] config changed — full re-index...");
                        let start = std::time::Instant::now();
                        graph = build_graph(&path, false)?;
                        let elapsed = start.elapsed();
                        eprintln!(
                            "[watch] re-indexed in {:.1}ms ({} files, {} symbols)",
                            elapsed.as_secs_f64() * 1000.0,
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                    watcher::event::WatchEvent::CrateRootChanged(p) => {
                        let filename = p.file_name().unwrap_or_default().to_string_lossy();
                        eprintln!("[watch] full re-index: {} changed", filename);
                        let start = std::time::Instant::now();
                        graph = build_graph(&path, false)?;
                        let elapsed = start.elapsed();
                        eprintln!(
                            "[watch] re-indexed in {:.1}ms ({} files, {} symbols)",
                            elapsed.as_secs_f64() * 1000.0,
                            graph.file_count(),
                            graph.symbol_count()
                        );
                        let _ = cache::save_cache(&path, &graph);
                    }
                }
            }
        }
    }

    Ok(())
}

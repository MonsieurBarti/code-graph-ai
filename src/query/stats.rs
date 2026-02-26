use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{GraphNode, SymbolKind},
};

/// Per-crate symbol breakdown (for workspace projects with multiple crates).
#[derive(Debug)]
pub struct CrateStats {
    /// Normalized crate name (hyphens → underscores).
    pub crate_name: String,
    /// Number of source files in this crate.
    pub file_count: usize,
    /// Total symbol count across all files in this crate.
    pub symbol_count: usize,
    // Symbol counts by kind
    pub fn_count: usize,
    pub struct_count: usize,
    pub enum_count: usize,
    pub trait_count: usize,
    pub impl_method_count: usize,
    pub type_alias_count: usize,
    pub const_count: usize,
    pub static_count: usize,
    pub macro_count: usize,
}

/// Aggregated project statistics derived from the code graph.
#[derive(Debug)]
pub struct ProjectStats {
    pub file_count: usize,
    pub symbol_count: usize,
    pub functions: usize,
    pub classes: usize,
    pub interfaces: usize,
    pub type_aliases: usize,
    pub enums: usize,
    pub variables: usize,
    pub components: usize,
    pub methods: usize,
    pub properties: usize,
    pub import_edges: usize,
    pub external_packages: usize,
    pub unresolved_imports: usize,
    // Rust-specific counts (Phase 8)
    pub rust_fns: usize,
    pub rust_structs: usize,
    pub rust_enums: usize,
    pub rust_traits: usize,
    pub rust_impl_methods: usize,
    pub rust_type_aliases: usize,
    pub rust_consts: usize,
    pub rust_statics: usize,
    pub rust_macros: usize,
    pub rust_imports: usize,
    pub rust_reexports: usize,
    // Phase 9 additions: per-crate breakdowns and dependency counts
    /// Per-crate symbol breakdowns (non-empty only for workspace projects).
    pub rust_crate_stats: Vec<CrateStats>,
    /// Number of distinct Builtin nodes (std, core, alloc).
    pub builtin_count: usize,
    /// Number of edges pointing to Builtin nodes (usage count).
    pub builtin_usage_count: usize,
    /// Number of edges pointing to ExternalPackage nodes (usage count).
    pub external_usage_count: usize,
    // Phase 12: Non-parsed file counts
    /// Total number of non-parsed (non-source) files in the graph.
    pub non_parsed_files: usize,
    /// Count of doc files (FileKind::Doc).
    pub doc_files: usize,
    /// Count of config files (FileKind::Config).
    pub config_files: usize,
    /// Count of CI files (FileKind::Ci).
    pub ci_files: usize,
    /// Count of asset files (FileKind::Asset).
    pub asset_files: usize,
    /// Count of other non-parsed files (FileKind::Other).
    pub other_files: usize,
    /// Count of source files (FileKind::Source) -- for clarity in output.
    pub source_files: usize,
}

/// Compute project statistics from a built `CodeGraph`.
pub fn project_stats(graph: &CodeGraph) -> ProjectStats {
    let breakdown = graph.symbols_by_kind();

    let import_edges = graph
        .graph
        .edge_indices()
        .filter(|&e| matches!(graph.graph[e], EdgeKind::ResolvedImport { .. }))
        .count();

    let rust_imports = graph
        .graph
        .edge_indices()
        .filter(|&e| matches!(graph.graph[e], EdgeKind::RustImport { .. }))
        .count();

    let rust_reexports = graph
        .graph
        .edge_indices()
        .filter(|&e| matches!(graph.graph[e], EdgeKind::ReExport { .. }))
        .count();

    let mut external_packages = 0usize;
    let mut unresolved_imports = 0usize;
    let mut builtin_count = 0usize;

    for idx in graph.graph.node_indices() {
        match graph.graph[idx] {
            GraphNode::ExternalPackage(_) => external_packages += 1,
            GraphNode::UnresolvedImport { .. } => unresolved_imports += 1,
            GraphNode::Builtin { .. } => builtin_count += 1,
            _ => {}
        }
    }

    // Count edges pointing to Builtin and ExternalPackage nodes.
    let mut builtin_usage_count = 0usize;
    let mut external_usage_count = 0usize;
    for edge_idx in graph.graph.edge_indices() {
        if let EdgeKind::ResolvedImport { .. } = &graph.graph[edge_idx] {
            let (_src, tgt) = graph.graph.edge_endpoints(edge_idx).unwrap();
            match &graph.graph[tgt] {
                GraphNode::Builtin { .. } => builtin_usage_count += 1,
                GraphNode::ExternalPackage(_) => external_usage_count += 1,
                _ => {}
            }
        }
    }

    // Count Rust-specific symbols by checking parent file language
    let mut rust_fns = 0usize;
    let mut rust_structs = 0usize;
    let mut rust_enums = 0usize;
    let mut rust_traits = 0usize;
    let mut rust_impl_methods = 0usize;
    let mut rust_type_aliases = 0usize;
    let mut rust_consts = 0usize;
    let mut rust_statics = 0usize;
    let mut rust_macros = 0usize;

    for idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(ref s) = graph.graph[idx] {
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
            let parent_in_rust = if !in_rust_file {
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
                    })
            } else {
                false
            };

            if !in_rust_file && !parent_in_rust {
                continue;
            }

            match s.kind {
                SymbolKind::Function => rust_fns += 1,
                SymbolKind::Struct => rust_structs += 1,
                SymbolKind::Enum => rust_enums += 1,
                SymbolKind::Trait => rust_traits += 1,
                SymbolKind::ImplMethod => rust_impl_methods += 1,
                SymbolKind::TypeAlias => rust_type_aliases += 1,
                SymbolKind::Const => rust_consts += 1,
                SymbolKind::Static => rust_statics += 1,
                SymbolKind::Macro => rust_macros += 1,
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Per-crate breakdown (Phase 9).
    //
    // Group Rust file nodes by their crate_name field, then count symbols per crate.
    // Only populated when more than one crate is present (single-crate projects don't need it).
    // ---------------------------------------------------------------------------
    let rust_crate_stats = compute_crate_stats(graph);

    // Phase 12: Count files by FileKind
    let mut source_files = 0usize;
    let mut doc_files = 0usize;
    let mut config_files = 0usize;
    let mut ci_files = 0usize;
    let mut asset_files = 0usize;
    let mut other_files = 0usize;
    for idx in graph.graph.node_indices() {
        if let GraphNode::File(ref fi) = graph.graph[idx] {
            match fi.kind {
                crate::graph::node::FileKind::Source => source_files += 1,
                crate::graph::node::FileKind::Doc => doc_files += 1,
                crate::graph::node::FileKind::Config => config_files += 1,
                crate::graph::node::FileKind::Ci => ci_files += 1,
                crate::graph::node::FileKind::Asset => asset_files += 1,
                crate::graph::node::FileKind::Other => other_files += 1,
            }
        }
    }
    let non_parsed_files = doc_files + config_files + ci_files + asset_files + other_files;

    ProjectStats {
        file_count: graph.file_index.len(),
        symbol_count: graph.symbol_count(),
        functions: *breakdown.get(&SymbolKind::Function).unwrap_or(&0),
        classes: *breakdown.get(&SymbolKind::Class).unwrap_or(&0),
        interfaces: *breakdown.get(&SymbolKind::Interface).unwrap_or(&0),
        type_aliases: *breakdown.get(&SymbolKind::TypeAlias).unwrap_or(&0),
        enums: *breakdown.get(&SymbolKind::Enum).unwrap_or(&0),
        variables: *breakdown.get(&SymbolKind::Variable).unwrap_or(&0),
        components: *breakdown.get(&SymbolKind::Component).unwrap_or(&0),
        methods: *breakdown.get(&SymbolKind::Method).unwrap_or(&0),
        properties: *breakdown.get(&SymbolKind::Property).unwrap_or(&0),
        import_edges,
        external_packages,
        unresolved_imports,
        rust_fns,
        rust_structs,
        rust_enums,
        rust_traits,
        rust_impl_methods,
        rust_type_aliases,
        rust_consts,
        rust_statics,
        rust_macros,
        rust_imports,
        rust_reexports,
        rust_crate_stats,
        builtin_count,
        builtin_usage_count,
        external_usage_count,
        // Phase 12: Non-parsed file counts
        non_parsed_files,
        doc_files,
        config_files,
        ci_files,
        asset_files,
        other_files,
        source_files,
    }
}

// ---------------------------------------------------------------------------
// Per-crate breakdown computation
// ---------------------------------------------------------------------------

/// Build per-crate symbol stats by grouping files by their `crate_name` field.
///
/// Returns an empty `Vec` if there are no Rust files with `crate_name` set, or if all
/// files belong to a single unnamed crate (not worth showing a one-row breakdown).
fn compute_crate_stats(graph: &CodeGraph) -> Vec<CrateStats> {
    use std::collections::HashMap;

    // Collect (crate_name, file_idx) pairs from Rust files with crate_name set.
    let mut crate_files: HashMap<String, Vec<petgraph::stable_graph::NodeIndex>> = HashMap::new();

    for idx in graph.graph.node_indices() {
        if let GraphNode::File(ref fi) = graph.graph[idx]
            && fi.language == "rust"
            && let Some(ref cn) = fi.crate_name
        {
            crate_files.entry(cn.clone()).or_default().push(idx);
        }
    }

    // Only build per-crate breakdown if there are multiple crates.
    if crate_files.len() <= 1 {
        return Vec::new();
    }

    let mut result: Vec<CrateStats> = crate_files
        .into_iter()
        .map(|(crate_name, file_indices)| {
            let file_count = file_indices.len();
            let mut sym_count = 0usize;
            let mut fn_count = 0usize;
            let mut struct_count = 0usize;
            let mut enum_count = 0usize;
            let mut trait_count = 0usize;
            let mut impl_method_count = 0usize;
            let mut type_alias_count = 0usize;
            let mut const_count = 0usize;
            let mut static_count = 0usize;
            let mut macro_count = 0usize;

            // For each file in this crate, find all symbols via Contains edges.
            for file_idx in &file_indices {
                for edge in graph.graph.edges(*file_idx) {
                    if let EdgeKind::Contains = edge.weight()
                        && let GraphNode::Symbol(ref s) = graph.graph[edge.target()]
                    {
                        sym_count += 1;
                        match s.kind {
                            SymbolKind::Function => fn_count += 1,
                            SymbolKind::Struct => struct_count += 1,
                            SymbolKind::Enum => enum_count += 1,
                            SymbolKind::Trait => trait_count += 1,
                            SymbolKind::ImplMethod => impl_method_count += 1,
                            SymbolKind::TypeAlias => type_alias_count += 1,
                            SymbolKind::Const => const_count += 1,
                            SymbolKind::Static => static_count += 1,
                            SymbolKind::Macro => macro_count += 1,
                            _ => {}
                        }
                        // Also count child symbols (via ChildOf edges from children).
                        for child_edge in graph
                            .graph
                            .edges_directed(edge.target(), Direction::Incoming)
                        {
                            if let EdgeKind::ChildOf = child_edge.weight() {
                                sym_count += 1;
                                if let GraphNode::Symbol(ref cs) = graph.graph[child_edge.source()]
                                {
                                    match cs.kind {
                                        SymbolKind::ImplMethod => impl_method_count += 1,
                                        SymbolKind::Property => {} // don't double count
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }

            CrateStats {
                crate_name,
                file_count,
                symbol_count: sym_count,
                fn_count,
                struct_count,
                enum_count,
                trait_count,
                impl_method_count,
                type_alias_count,
                const_count,
                static_count,
                macro_count,
            }
        })
        .collect();

    // Sort by crate name for deterministic output.
    result.sort_by(|a, b| a.crate_name.cmp(&b.crate_name));
    result
}

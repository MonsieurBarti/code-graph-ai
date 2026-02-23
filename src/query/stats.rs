use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{GraphNode, SymbolKind},
};

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

    for idx in graph.graph.node_indices() {
        match graph.graph[idx] {
            GraphNode::ExternalPackage(_) => external_packages += 1,
            GraphNode::UnresolvedImport { .. } => unresolved_imports += 1,
            _ => {}
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
                    if let EdgeKind::Contains = e.weight() {
                        if let GraphNode::File(ref f) = graph.graph[e.source()] {
                            return f.language == "rust";
                        }
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
                                    if let EdgeKind::Contains = pe.weight() {
                                        if let GraphNode::File(ref f) = graph.graph[pe.source()] {
                                            return f.language == "rust";
                                        }
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
    }
}

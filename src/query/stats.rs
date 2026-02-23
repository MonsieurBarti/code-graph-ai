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
}

/// Compute project statistics from a built `CodeGraph`.
pub fn project_stats(graph: &CodeGraph) -> ProjectStats {
    let breakdown = graph.symbols_by_kind();

    let import_edges = graph
        .graph
        .edge_indices()
        .filter(|&e| matches!(graph.graph[e], EdgeKind::ResolvedImport { .. }))
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
    }
}

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{GraphNode, SymbolKind},
};
use crate::query::find::FindResult;
use crate::query::refs::RefResult;

/// Information about a symbol involved in a call or inheritance relationship.
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub symbol_name: String,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub line: usize,
}

/// The 360-degree view of a symbol: definition, references, callers, callees, and inheritance.
#[derive(Debug, Clone)]
pub struct SymbolContext {
    /// Symbol name that was queried.
    pub symbol_name: String,
    /// Where the symbol is defined (same as find results).
    pub definitions: Vec<FindResult>,
    /// Files that reference/import the symbol (same as refs results).
    pub references: Vec<RefResult>,
    /// Symbols that this symbol calls (outgoing Calls edges from symbols in the same file).
    pub callees: Vec<CallInfo>,
    /// Symbols that call this symbol (incoming Calls edges).
    pub callers: Vec<CallInfo>,
    /// Symbols this extends (outgoing Extends edges).
    pub extends: Vec<CallInfo>,
    /// Symbols this implements (outgoing Implements edges).
    pub implements: Vec<CallInfo>,
    /// Symbols that extend this (incoming Extends edges).
    pub extended_by: Vec<CallInfo>,
    /// Symbols that implement this (incoming Implements edges).
    pub implemented_by: Vec<CallInfo>,
}

/// Build a 360-degree context view for a symbol.
///
/// - `graph`: the code graph to query
/// - `symbol_name`: display name for the query
/// - `symbol_indices`: all NodeIndices of the matching symbol (may span multiple files)
/// - `project_root`: used for computing relative paths
pub fn symbol_context(
    graph: &CodeGraph,
    symbol_name: &str,
    symbol_indices: &[NodeIndex],
    project_root: &Path,
) -> SymbolContext {
    // -------------------------------------------------------------------------
    // Definitions: for each symbol NodeIndex, find parent file via Contains edge
    // and build a FindResult.
    // -------------------------------------------------------------------------
    let mut definitions: Vec<FindResult> = Vec::new();
    let mut def_dedup: HashSet<(PathBuf, usize)> = HashSet::new();

    for &sym_idx in symbol_indices {
        let sym_info = match &graph.graph[sym_idx] {
            GraphNode::Symbol(info) => info.clone(),
            _ => continue,
        };

        let file_info = find_containing_file(graph, sym_idx);
        if let Some(fi) = file_info {
            let key = (fi.path.clone(), sym_info.line);
            if !def_dedup.contains(&key) {
                def_dedup.insert(key);
                definitions.push(FindResult {
                    symbol_name: sym_info.name.clone(),
                    kind: sym_info.kind.clone(),
                    file_path: fi.path.clone(),
                    line: sym_info.line,
                    col: sym_info.col,
                    is_exported: sym_info.is_exported,
                    is_default: sym_info.is_default,
                });
            }
        }
    }
    definitions.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    // -------------------------------------------------------------------------
    // References: reuse find_refs for import and call references.
    // -------------------------------------------------------------------------
    let references =
        crate::query::refs::find_refs(graph, symbol_name, symbol_indices, project_root);

    // -------------------------------------------------------------------------
    // Callers: symbols that have an outgoing Calls edge to any of our symbol nodes
    // (incoming Calls edge to our symbol).
    // -------------------------------------------------------------------------
    let mut callers: Vec<CallInfo> = Vec::new();
    let mut caller_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();

    for &sym_idx in symbol_indices {
        for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
            if !matches!(edge_ref.weight(), EdgeKind::Calls) {
                continue;
            }
            let caller_idx = edge_ref.source();
            if let Some(ci) = build_call_info(graph, caller_idx) {
                let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                if !caller_dedup.contains(&key) {
                    caller_dedup.insert(key);
                    callers.push(ci);
                }
            }
        }
    }
    callers.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    // -------------------------------------------------------------------------
    // Callees: symbols that our symbol (or its parent file) calls via outgoing Calls edges.
    //
    // Per Phase 2 graph structure: Calls edges go from file node -> callee symbol node
    // for unscoped file-level calls. For symbol-to-symbol calls, they go from caller
    // symbol -> callee symbol. We check both:
    //   1. Outgoing Calls from the symbol node itself.
    //   2. Outgoing Calls from the symbol's parent file node (file-level calls).
    // -------------------------------------------------------------------------
    let mut callees: Vec<CallInfo> = Vec::new();
    let mut callee_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();

    for &sym_idx in symbol_indices {
        // Direct symbol -> symbol calls.
        for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
            if !matches!(edge_ref.weight(), EdgeKind::Calls) {
                continue;
            }
            let callee_idx = edge_ref.target();
            if let Some(ci) = build_call_info(graph, callee_idx) {
                let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                if !callee_dedup.contains(&key) {
                    callee_dedup.insert(key);
                    callees.push(ci);
                }
            }
        }

        // File-level calls: outgoing Calls from the symbol's parent file.
        if let Some(file_idx) = find_containing_file_idx(graph, sym_idx) {
            for edge_ref in graph.graph.edges_directed(file_idx, Direction::Outgoing) {
                if !matches!(edge_ref.weight(), EdgeKind::Calls) {
                    continue;
                }
                let callee_idx = edge_ref.target();
                if let Some(ci) = build_call_info(graph, callee_idx) {
                    let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                    if !callee_dedup.contains(&key) {
                        callee_dedup.insert(key);
                        callees.push(ci);
                    }
                }
            }
        }
    }
    callees.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    // -------------------------------------------------------------------------
    // Extends / Implements: outgoing Extends/Implements from symbol nodes.
    // Extended_by / Implemented_by: incoming Extends/Implements to symbol nodes.
    // -------------------------------------------------------------------------
    let mut extends: Vec<CallInfo> = Vec::new();
    let mut implements: Vec<CallInfo> = Vec::new();
    let mut extended_by: Vec<CallInfo> = Vec::new();
    let mut implemented_by: Vec<CallInfo> = Vec::new();

    let mut ext_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();
    let mut impl_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();
    let mut extby_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();
    let mut implby_dedup: HashSet<(String, PathBuf, usize)> = HashSet::new();

    for &sym_idx in symbol_indices {
        // Outgoing: what this symbol extends or implements.
        for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
            let target_idx = edge_ref.target();
            match edge_ref.weight() {
                EdgeKind::Extends => {
                    if let Some(ci) = build_call_info(graph, target_idx) {
                        let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                        if !ext_dedup.contains(&key) {
                            ext_dedup.insert(key);
                            extends.push(ci);
                        }
                    }
                }
                EdgeKind::Implements => {
                    if let Some(ci) = build_call_info(graph, target_idx) {
                        let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                        if !impl_dedup.contains(&key) {
                            impl_dedup.insert(key);
                            implements.push(ci);
                        }
                    }
                }
                _ => {}
            }
        }

        // Incoming: what extends/implements this symbol.
        for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
            let source_idx = edge_ref.source();
            match edge_ref.weight() {
                EdgeKind::Extends => {
                    if let Some(ci) = build_call_info(graph, source_idx) {
                        let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                        if !extby_dedup.contains(&key) {
                            extby_dedup.insert(key);
                            extended_by.push(ci);
                        }
                    }
                }
                EdgeKind::Implements => {
                    if let Some(ci) = build_call_info(graph, source_idx) {
                        let key = (ci.symbol_name.clone(), ci.file_path.clone(), ci.line);
                        if !implby_dedup.contains(&key) {
                            implby_dedup.insert(key);
                            implemented_by.push(ci);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    extends.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    implements.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    extended_by.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    implemented_by.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    SymbolContext {
        symbol_name: symbol_name.to_string(),
        definitions,
        references,
        callees,
        callers,
        extends,
        implements,
        extended_by,
        implemented_by,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Find the FileInfo for a symbol node via an incoming Contains edge.
fn find_containing_file(
    graph: &CodeGraph,
    sym_idx: NodeIndex,
) -> Option<crate::graph::node::FileInfo> {
    // Direct Contains edge from file to symbol.
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge_ref.weight(), EdgeKind::Contains)
            && let GraphNode::File(ref fi) = graph.graph[edge_ref.source()]
        {
            return Some(fi.clone());
        }
    }

    // Child symbol: follow ChildOf edge to parent, then Contains on parent.
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
        if matches!(edge_ref.weight(), EdgeKind::ChildOf) {
            let parent_idx = edge_ref.target();
            if let Some(fi) = find_containing_file(graph, parent_idx) {
                return Some(fi);
            }
        }
    }

    None
}

/// Return the NodeIndex of the File node that contains `sym_idx` via Contains/ChildOf edges.
fn find_containing_file_idx(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<NodeIndex> {
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge_ref.weight(), EdgeKind::Contains) {
            let source = edge_ref.source();
            if matches!(graph.graph[source], GraphNode::File(_)) {
                return Some(source);
            }
        }
    }

    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
        if matches!(edge_ref.weight(), EdgeKind::ChildOf) {
            return find_containing_file_idx(graph, edge_ref.target());
        }
    }

    None
}

/// Build a CallInfo from a graph node index.
///
/// Only Symbol nodes produce meaningful CallInfo entries. File nodes and others
/// are skipped (returns None).
fn build_call_info(graph: &CodeGraph, node_idx: NodeIndex) -> Option<CallInfo> {
    match &graph.graph[node_idx] {
        GraphNode::Symbol(info) => {
            // Find file path for this symbol.
            let file_path = find_containing_file(graph, node_idx)
                .map(|fi| fi.path)
                .unwrap_or_default();
            Some(CallInfo {
                symbol_name: info.name.clone(),
                kind: info.kind.clone(),
                file_path,
                line: info.line,
            })
        }
        // File and external nodes are not useful as call targets/sources in this context.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::{
        CodeGraph,
        node::{SymbolInfo, SymbolKind, SymbolVisibility},
    };

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }

    /// Build a graph with:
    ///   service.ts: class UserService
    ///   controller.ts: function handleRequest (has Calls edge to UserService)
    fn graph_with_calls() -> (CodeGraph, PathBuf, NodeIndex, NodeIndex) {
        let root = root();
        let mut graph = CodeGraph::new();

        let service = graph.add_file(root.join("service.ts"), "typescript");
        let user_service = graph.add_symbol(
            service,
            SymbolInfo {
                name: "UserService".into(),
                kind: SymbolKind::Class,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        let controller = graph.add_file(root.join("controller.ts"), "typescript");
        let handle_request = graph.add_symbol(
            controller,
            SymbolInfo {
                name: "handleRequest".into(),
                kind: SymbolKind::Function,
                line: 3,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );
        // handleRequest calls UserService
        graph.add_calls_edge(handle_request, user_service);

        (graph, root, user_service, handle_request)
    }

    #[test]
    fn test_symbol_with_caller_has_callers() {
        let (graph, root, user_service, handle_request) = graph_with_calls();
        let ctx = symbol_context(&graph, "UserService", &[user_service], &root);

        assert_eq!(ctx.callers.len(), 1, "UserService should have one caller");
        assert_eq!(ctx.callers[0].symbol_name, "handleRequest");
        assert_eq!(ctx.callers[0].line, 3);
        assert!(ctx.callers[0].file_path.ends_with("controller.ts"));

        // handleRequest calls UserService, so UserService has no callees of its own
        let _ = handle_request; // suppress unused warning
        assert!(
            ctx.callees.is_empty(),
            "UserService has no outgoing Calls edges"
        );
    }

    #[test]
    fn test_caller_symbol_has_callee() {
        let (graph, root, _user_service, handle_request) = graph_with_calls();
        let ctx = symbol_context(&graph, "handleRequest", &[handle_request], &root);

        // handleRequest calls UserService — should appear in callees (from file-level Calls walk)
        // Note: add_calls_edge(handle_request, user_service) adds symbol-to-symbol Calls edge
        assert_eq!(ctx.callees.len(), 1, "handleRequest should have one callee");
        assert_eq!(ctx.callees[0].symbol_name, "UserService");
    }

    #[test]
    fn test_symbol_with_extends_has_extends_list() {
        let root = root();
        let mut graph = CodeGraph::new();

        let base_file = graph.add_file(root.join("base.ts"), "typescript");
        let base_class = graph.add_symbol(
            base_file,
            SymbolInfo {
                name: "BaseService".into(),
                kind: SymbolKind::Class,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        let child_file = graph.add_file(root.join("child.ts"), "typescript");
        let child_class = graph.add_symbol(
            child_file,
            SymbolInfo {
                name: "ChildService".into(),
                kind: SymbolKind::Class,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        graph.add_extends_edge(child_class, base_class);

        // Query ChildService — should see extends = [BaseService]
        let ctx = symbol_context(&graph, "ChildService", &[child_class], &root);
        assert_eq!(ctx.extends.len(), 1);
        assert_eq!(ctx.extends[0].symbol_name, "BaseService");

        // Query BaseService — should see extended_by = [ChildService]
        let ctx2 = symbol_context(&graph, "BaseService", &[base_class], &root);
        assert_eq!(ctx2.extended_by.len(), 1);
        assert_eq!(ctx2.extended_by[0].symbol_name, "ChildService");
    }

    #[test]
    fn test_empty_graph_produces_empty_context() {
        let root = root();
        let graph = CodeGraph::new();
        let ctx = symbol_context(&graph, "Anything", &[], &root);

        assert!(ctx.definitions.is_empty());
        assert!(ctx.references.is_empty());
        assert!(ctx.callees.is_empty());
        assert!(ctx.callers.is_empty());
        assert!(ctx.extends.is_empty());
        assert!(ctx.implements.is_empty());
        assert!(ctx.extended_by.is_empty());
        assert!(ctx.implemented_by.is_empty());
    }

    #[test]
    fn test_implements_relationship() {
        let root = root();
        let mut graph = CodeGraph::new();

        let iface_file = graph.add_file(root.join("iface.ts"), "typescript");
        let iface = graph.add_symbol(
            iface_file,
            SymbolInfo {
                name: "IService".into(),
                kind: SymbolKind::Interface,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        let impl_file = graph.add_file(root.join("impl.ts"), "typescript");
        let impl_class = graph.add_symbol(
            impl_file,
            SymbolInfo {
                name: "ServiceImpl".into(),
                kind: SymbolKind::Class,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        graph.add_implements_edge(impl_class, iface);

        // ServiceImpl implements IService
        let ctx = symbol_context(&graph, "ServiceImpl", &[impl_class], &root);
        assert_eq!(ctx.implements.len(), 1);
        assert_eq!(ctx.implements[0].symbol_name, "IService");

        // IService is implemented by ServiceImpl
        let ctx2 = symbol_context(&graph, "IService", &[iface], &root);
        assert_eq!(ctx2.implemented_by.len(), 1);
        assert_eq!(ctx2.implemented_by[0].symbol_name, "ServiceImpl");
    }
}

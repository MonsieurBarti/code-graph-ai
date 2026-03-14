use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

/// Return the NodeIndex of the File node that contains `sym_idx` via a Contains or ChildOf edge.
///
/// Shared utility used by impact.rs, rename.rs, and other query modules.
pub(crate) fn find_containing_file_idx(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<NodeIndex> {
    // Direct Contains edge: File -> Symbol (incoming to symbol).
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Incoming) {
        if matches!(edge_ref.weight(), EdgeKind::Contains) {
            let source = edge_ref.source();
            if matches!(graph.graph[source], GraphNode::File(_)) {
                return Some(source);
            }
        }
    }

    // Child symbol: ChildOf edge from child (outgoing) to parent symbol, then Contains on parent.
    for edge_ref in graph.graph.edges_directed(sym_idx, Direction::Outgoing) {
        if matches!(edge_ref.weight(), EdgeKind::ChildOf) {
            let parent_idx = edge_ref.target();
            if let Some(file_idx) = find_containing_file_idx(graph, parent_idx) {
                return Some(file_idx);
            }
        }
    }

    None
}

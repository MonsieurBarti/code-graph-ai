use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use crate::graph::{
    CodeGraph,
    edge::EdgeKind,
    node::{GraphNode, SymbolInfo, SymbolKind},
};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Structural signature of a symbol for clone detection.
///
/// Two symbols with the same `StructuralHash` are considered structural clones.
/// Components: (SymbolKind, body_size, outgoing_edge_count, incoming_edge_count, decorator_count).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StructuralSignature {
    kind: SymbolKind,
    body_size: usize,
    outgoing_edges: usize,
    incoming_edges: usize,
    decorator_count: usize,
}

/// Compute a u64 hash from a `StructuralSignature`.
fn compute_structural_hash(sig: &StructuralSignature) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sig.hash(&mut hasher);
    hasher.finish()
}

/// A single symbol member within a clone group.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneMember {
    pub name: String,
    pub kind: String,
    pub file: PathBuf,
    pub line: usize,
    pub body_size: usize,
}

/// A group of structurally identical symbols (clones).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneGroup {
    /// Hash identifying this structural signature.
    pub hash: u64,
    /// Human-readable description of the structural signature.
    pub signature: String,
    /// Members of this clone group.
    pub members: Vec<CloneMember>,
}

/// Result of clone detection analysis.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneGroupResult {
    /// Clone groups with >= min_group members.
    pub groups: Vec<CloneGroup>,
    /// Total number of symbols analyzed.
    pub total_symbols_analyzed: usize,
}

// ---------------------------------------------------------------------------
// Main query function
// ---------------------------------------------------------------------------

/// Detect structural clones: symbols with identical structural signatures.
///
/// - `graph`: the code graph to analyze
/// - `root`: the project root path (used for relative path computation)
/// - `scope`: optional path scope; if provided, only analyze symbols under this path
/// - `min_group`: minimum group size to report (default 2)
///
/// Returns a `CloneGroupResult` with groups of structurally identical symbols.
pub fn find_clones(
    graph: &CodeGraph,
    root: &Path,
    scope: Option<&Path>,
    min_group: usize,
) -> CloneGroupResult {
    // Compute absolute scope path if provided
    let abs_scope: Option<PathBuf> = scope.map(|s| {
        if s.is_absolute() {
            s.to_path_buf()
        } else {
            root.join(s)
        }
    });

    // Helper: check if a path is under the scope
    let in_scope = |path: &Path| -> bool {
        match &abs_scope {
            None => true,
            Some(scope_path) => path.starts_with(scope_path),
        }
    };

    // Build a map: symbol NodeIndex -> file info (path)
    // We iterate all Symbol nodes, find their containing file via incoming Contains edge.
    let mut sym_to_file: HashMap<petgraph::stable_graph::NodeIndex, PathBuf> = HashMap::new();

    for node_idx in graph.graph.node_indices() {
        if let GraphNode::Symbol(_) = &graph.graph[node_idx] {
            for edge in graph.graph.edges_directed(node_idx, Direction::Incoming) {
                if matches!(edge.weight(), EdgeKind::Contains)
                    && let GraphNode::File(fi) = &graph.graph[edge.source()]
                {
                    sym_to_file.insert(node_idx, fi.path.clone());
                    break;
                }
            }
        }
    }

    // Group symbols by structural hash
    let mut hash_groups: HashMap<u64, (StructuralSignature, Vec<CloneMember>)> = HashMap::new();
    let mut total_symbols_analyzed: usize = 0;

    for node_idx in graph.graph.node_indices() {
        let sym = match &graph.graph[node_idx] {
            GraphNode::Symbol(s) => s,
            _ => continue,
        };

        // Get file path for this symbol
        let file_path = match sym_to_file.get(&node_idx) {
            Some(p) => p,
            None => continue, // orphan symbol, skip
        };

        // Check scope
        if !in_scope(file_path) {
            continue;
        }

        total_symbols_analyzed += 1;

        // Compute structural signature
        let sig = compute_signature(graph, node_idx, sym);
        let hash = compute_structural_hash(&sig);

        let kind_str = crate::query::find::kind_to_str(&sym.kind).to_string();
        let body_size = sig.body_size;
        let member = CloneMember {
            name: sym.name.clone(),
            kind: kind_str,
            file: file_path.clone(),
            line: sym.line,
            body_size,
        };

        hash_groups
            .entry(hash)
            .or_insert_with(|| (sig, Vec::new()))
            .1
            .push(member);
    }

    // Filter groups by min_group and build result
    let mut groups: Vec<CloneGroup> = hash_groups
        .into_iter()
        .filter(|(_, (_, members))| members.len() >= min_group)
        .map(|(hash, (sig, mut members))| {
            // Sort members for deterministic output: by file path, then line
            members.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
            CloneGroup {
                hash,
                signature: format_signature(&sig),
                members,
            }
        })
        .collect();

    // Sort groups by member count descending, then by hash for determinism
    groups.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then(a.hash.cmp(&b.hash))
    });

    CloneGroupResult {
        groups,
        total_symbols_analyzed,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the structural signature for a symbol node.
fn compute_signature(
    graph: &CodeGraph,
    node_idx: petgraph::stable_graph::NodeIndex,
    sym: &SymbolInfo,
) -> StructuralSignature {
    let body_size = sym.line_end.saturating_sub(sym.line);

    let outgoing_edges = graph
        .graph
        .edges_directed(node_idx, Direction::Outgoing)
        .count();

    let incoming_edges = graph
        .graph
        .edges_directed(node_idx, Direction::Incoming)
        .count();

    let decorator_count = sym.decorators.len();

    StructuralSignature {
        kind: sym.kind.clone(),
        body_size,
        outgoing_edges,
        incoming_edges,
        decorator_count,
    }
}

/// Format a structural signature as a human-readable string.
fn format_signature(sig: &StructuralSignature) -> String {
    format!(
        "kind={} body={} out={} in={} decorators={}",
        crate::query::find::kind_to_str(&sig.kind),
        sig.body_size,
        sig.outgoing_edges,
        sig.incoming_edges,
        sig.decorator_count,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::graph::{
        CodeGraph,
        node::{DecoratorInfo, SymbolInfo, SymbolKind},
    };

    fn make_symbol(name: &str, kind: SymbolKind, line: usize, line_end: usize) -> SymbolInfo {
        SymbolInfo {
            name: name.into(),
            kind,
            line,
            line_end,
            ..Default::default()
        }
    }

    fn make_symbol_with_decorators(
        name: &str,
        kind: SymbolKind,
        line: usize,
        line_end: usize,
        decorator_count: usize,
    ) -> SymbolInfo {
        let decorators: Vec<DecoratorInfo> = (0..decorator_count)
            .map(|i| DecoratorInfo {
                name: format!("decorator_{}", i),
                ..Default::default()
            })
            .collect();
        SymbolInfo {
            name: name.into(),
            kind,
            line,
            line_end,
            decorators,
            ..Default::default()
        }
    }

    #[test]
    fn test_identical_symbols_grouped() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_a = root.join("src/utils.rs");
        let file_b = root.join("src/helpers.rs");
        let file_a_idx = graph.add_file(file_a.clone(), "rust");
        let file_b_idx = graph.add_file(file_b.clone(), "rust");

        // Two functions with identical structural signatures:
        // same kind (Function), same body_size (10), same edge counts (0), same decorators (0)
        graph.add_symbol(
            file_a_idx,
            make_symbol("process_data", SymbolKind::Function, 1, 11),
        );
        graph.add_symbol(
            file_b_idx,
            make_symbol("transform_data", SymbolKind::Function, 1, 11),
        );

        let result = find_clones(&graph, &root, None, 2);
        assert_eq!(
            result.groups.len(),
            1,
            "Two identical symbols should form one clone group"
        );
        assert_eq!(
            result.groups[0].members.len(),
            2,
            "Clone group should have 2 members"
        );

        let names: Vec<&str> = result.groups[0]
            .members
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert!(names.contains(&"process_data"));
        assert!(names.contains(&"transform_data"));
    }

    #[test]
    fn test_distinct_symbols_not_grouped() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");

        // Different kind
        graph.add_symbol(file_idx, make_symbol("my_fn", SymbolKind::Function, 1, 11));
        // Different body_size
        graph.add_symbol(file_idx, make_symbol("my_class", SymbolKind::Class, 1, 51));
        // Another function but different body size
        graph.add_symbol(
            file_idx,
            make_symbol("small_fn", SymbolKind::Function, 1, 4),
        );

        let result = find_clones(&graph, &root, None, 2);
        assert_eq!(
            result.groups.len(),
            0,
            "Distinct symbols should not form clone groups"
        );
    }

    #[test]
    fn test_min_group_filter() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_idx = graph.add_file(root.join("src/mod.rs"), "rust");

        // Three identical functions
        graph.add_symbol(file_idx, make_symbol("fn_a", SymbolKind::Function, 1, 6));
        graph.add_symbol(file_idx, make_symbol("fn_b", SymbolKind::Function, 10, 15));
        graph.add_symbol(file_idx, make_symbol("fn_c", SymbolKind::Function, 20, 25));

        // With min_group=2, should find one group of 3
        let result = find_clones(&graph, &root, None, 2);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].members.len(), 3);

        // With min_group=4, should find no groups
        let result = find_clones(&graph, &root, None, 4);
        assert_eq!(
            result.groups.len(),
            0,
            "Group of 3 should be filtered out when min_group=4"
        );
    }

    #[test]
    fn test_scope_filter() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");

        // File inside scope
        let in_scope_file = root.join("src/module/a.rs");
        let in_scope_idx = graph.add_file(in_scope_file.clone(), "rust");
        graph.add_symbol(
            in_scope_idx,
            make_symbol("fn_in_scope_1", SymbolKind::Function, 1, 11),
        );

        // Another file inside scope with same signature
        let in_scope_file2 = root.join("src/module/b.rs");
        let in_scope_idx2 = graph.add_file(in_scope_file2.clone(), "rust");
        graph.add_symbol(
            in_scope_idx2,
            make_symbol("fn_in_scope_2", SymbolKind::Function, 1, 11),
        );

        // File outside scope with same signature
        let out_scope_file = root.join("other/c.rs");
        let out_scope_idx = graph.add_file(out_scope_file.clone(), "rust");
        graph.add_symbol(
            out_scope_idx,
            make_symbol("fn_out_scope", SymbolKind::Function, 1, 11),
        );

        // Run with scope = "src/module"
        let scope_path = PathBuf::from("src/module");
        let result = find_clones(&graph, &root, Some(&scope_path), 2);

        assert_eq!(
            result.groups.len(),
            1,
            "Should find 1 clone group within scope"
        );
        assert_eq!(
            result.groups[0].members.len(),
            2,
            "Clone group should have 2 in-scope members"
        );

        let names: Vec<&str> = result.groups[0]
            .members
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert!(names.contains(&"fn_in_scope_1"));
        assert!(names.contains(&"fn_in_scope_2"));
        assert!(
            !names.contains(&"fn_out_scope"),
            "Out-of-scope symbol should be excluded"
        );
    }

    #[test]
    fn test_decorator_count_differentiates() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_idx = graph.add_file(root.join("src/app.rs"), "rust");

        // Same kind and body_size but different decorator counts
        graph.add_symbol(
            file_idx,
            make_symbol_with_decorators("fn_no_dec", SymbolKind::Function, 1, 11, 0),
        );
        graph.add_symbol(
            file_idx,
            make_symbol_with_decorators("fn_with_dec", SymbolKind::Function, 15, 25, 2),
        );

        let result = find_clones(&graph, &root, None, 2);
        assert_eq!(
            result.groups.len(),
            0,
            "Different decorator counts should prevent grouping"
        );
    }

    #[test]
    fn test_total_symbols_analyzed() {
        let mut graph = CodeGraph::new();
        let root = PathBuf::from("/project");
        let file_idx = graph.add_file(root.join("src/lib.rs"), "rust");

        graph.add_symbol(file_idx, make_symbol("fn_a", SymbolKind::Function, 1, 6));
        graph.add_symbol(file_idx, make_symbol("fn_b", SymbolKind::Function, 10, 15));
        graph.add_symbol(file_idx, make_symbol("cls_a", SymbolKind::Class, 20, 50));

        let result = find_clones(&graph, &root, None, 2);
        assert_eq!(
            result.total_symbols_analyzed, 3,
            "Should count all analyzed symbols"
        );
    }
}

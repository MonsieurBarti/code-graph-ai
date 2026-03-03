use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};

/// Confidence tier for impact analysis results.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
    /// Reserved for future use: confidence cannot be determined.
    #[allow(dead_code)]
    Unknown,
}

impl std::fmt::Display for ConfidenceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfidenceTier::High => write!(f, "HIGH"),
            ConfidenceTier::Medium => write!(f, "MEDIUM"),
            ConfidenceTier::Low => write!(f, "LOW"),
            ConfidenceTier::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

/// Risk tier for diff-impact classification based on downstream file count.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskTier {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for RiskTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskTier::High => write!(f, "HIGH"),
            RiskTier::Medium => write!(f, "MEDIUM"),
            RiskTier::Low => write!(f, "LOW"),
        }
    }
}

/// A single file in the blast-radius (impact) result set.
#[derive(Debug, Clone)]
pub struct ImpactResult {
    /// Absolute path to the affected file.
    pub file_path: PathBuf,
    /// BFS depth from the defining file(s) of the queried symbol (for --tree view).
    pub depth: usize,
    /// Confidence tier for this impact result.
    pub confidence: ConfidenceTier,
    /// Human-readable basis for the confidence tier.
    pub basis: String,
}

/// Result of diff-based impact analysis: a changed file and its downstream blast radius.
#[derive(Debug, Clone)]
pub struct DiffImpactResult {
    /// The file that changed (from git diff).
    pub changed_file: PathBuf,
    /// All downstream affected files with confidence.
    pub affected: Vec<ImpactResult>,
    /// Risk tier based on affected count.
    pub risk: RiskTier,
}

/// Score confidence for an impact result based on depth and edge metadata.
/// CALLS edges boost to HIGH regardless of depth. Depth 1 is also HIGH (direct importer).
fn score_confidence(depth: usize, has_direct_call: bool) -> (ConfidenceTier, String) {
    if has_direct_call || depth == 1 {
        (
            ConfidenceTier::High,
            format!("direct caller at depth {depth}"),
        )
    } else if depth <= 3 {
        (
            ConfidenceTier::Medium,
            format!("transitive dependency at depth {depth}"),
        )
    } else {
        (
            ConfidenceTier::Low,
            format!("deep transitive dependency at depth {depth}"),
        )
    }
}

/// Classify risk tier based on number of affected downstream files.
pub fn classify_risk(
    affected_count: usize,
    high_threshold: usize,
    medium_threshold: usize,
) -> RiskTier {
    if affected_count > high_threshold {
        RiskTier::High
    } else if affected_count >= medium_threshold {
        RiskTier::Medium
    } else {
        RiskTier::Low
    }
}

/// Compute the blast radius of changing the given symbols.
///
/// Performs a custom BFS on INCOMING `ResolvedImport` edges only (not Calls, Contains, etc.),
/// starting from the file(s) that define the queried symbols.
///
/// Returns all transitively dependent files sorted by depth (ascending) then by path.
pub fn blast_radius(
    graph: &CodeGraph,
    symbol_indices: &[NodeIndex],
    project_root: &Path,
) -> Vec<ImpactResult> {
    let _ = project_root; // kept for API consistency with find_refs

    // Step 1: Collect starting file indices — the file(s) that define the queried symbols.
    let mut starting_files: HashSet<NodeIndex> = HashSet::new();
    for &sym_idx in symbol_indices {
        if let Some(file_idx) = find_containing_file_idx(graph, sym_idx) {
            starting_files.insert(file_idx);
        }
    }

    if starting_files.is_empty() {
        return Vec::new();
    }

    // Step 2: Custom BFS following only incoming ResolvedImport edges (reverse import graph).
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut depths: HashMap<NodeIndex, usize> = HashMap::new();

    // Seed with starting files at depth 0.
    for &start_idx in &starting_files {
        queue.push_back(start_idx);
        visited.insert(start_idx);
        depths.insert(start_idx, 0);
    }

    while let Some(current) = queue.pop_front() {
        let current_depth = depths[&current];

        // Walk INCOMING edges to find files that import this file.
        for edge_ref in graph.graph.edges_directed(current, Direction::Incoming) {
            if matches!(edge_ref.weight(), EdgeKind::ResolvedImport { .. }) {
                let source = edge_ref.source();
                // Only follow File nodes — skip Symbol, ExternalPackage, UnresolvedImport.
                if !visited.contains(&source) && matches!(graph.graph[source], GraphNode::File(_)) {
                    visited.insert(source);
                    depths.insert(source, current_depth + 1);
                    queue.push_back(source);
                }
            }
        }
    }

    // Step 3: Collect results, excluding the starting files themselves.
    let mut results: Vec<ImpactResult> = visited
        .iter()
        .filter(|&&idx| !starting_files.contains(&idx))
        .filter_map(|&idx| {
            if let GraphNode::File(ref fi) = graph.graph[idx] {
                let depth = depths[&idx];

                // Check if this file node has an outgoing CALLS edge directly to any of the queried symbols.
                let has_direct_call =
                    graph
                        .graph
                        .edges_directed(idx, Direction::Outgoing)
                        .any(|e| {
                            matches!(e.weight(), EdgeKind::Calls)
                                && symbol_indices.contains(&e.target())
                        });

                let (confidence, basis) = score_confidence(depth, has_direct_call);

                Some(ImpactResult {
                    file_path: fi.path.clone(),
                    depth,
                    confidence,
                    basis,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by depth ascending, then by file path for deterministic output.
    results.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.file_path.cmp(&b.file_path)));

    results
}

fn risk_ord(r: &RiskTier) -> u8 {
    match r {
        RiskTier::High => 0,
        RiskTier::Medium => 1,
        RiskTier::Low => 2,
    }
}

/// Map git-changed files to their downstream impact in the graph.
///
/// For each changed file, runs blast_radius to find transitively dependent files.
/// Classifies risk based on configurable thresholds.
pub fn diff_impact(
    graph: &CodeGraph,
    changed_files: &[PathBuf],
    project_root: &Path,
    high_threshold: usize,
    medium_threshold: usize,
) -> Vec<DiffImpactResult> {
    let mut results = Vec::new();
    for changed in changed_files {
        // Normalize: try both absolute and relative paths against file_index
        let file_idx = graph.file_index.get(changed).or_else(|| {
            let abs = project_root.join(changed);
            graph.file_index.get(&abs)
        });

        let file_idx = match file_idx {
            Some(&idx) => idx,
            None => continue, // File not in graph (e.g., non-source file)
        };

        // Collect symbol indices defined in this file
        let symbol_indices: Vec<NodeIndex> = graph
            .graph
            .edges_directed(file_idx, Direction::Outgoing)
            .filter(|e| matches!(e.weight(), EdgeKind::Contains))
            .map(|e| e.target())
            .collect();

        // If no symbols, use the file node itself as the blast radius seed
        let seeds = if symbol_indices.is_empty() {
            vec![file_idx]
        } else {
            symbol_indices
        };

        let affected = blast_radius(graph, &seeds, project_root);
        let risk = classify_risk(affected.len(), high_threshold, medium_threshold);

        results.push(DiffImpactResult {
            changed_file: changed.clone(),
            affected,
            risk,
        });
    }

    // Sort by risk tier (HIGH first) then by affected count descending
    results.sort_by(|a, b| {
        let risk_ord_val = risk_ord(&a.risk).cmp(&risk_ord(&b.risk));
        risk_ord_val.then(b.affected.len().cmp(&a.affected.len()))
    });

    results
}

// ---------------------------------------------------------------------------
// Private helper
// ---------------------------------------------------------------------------

/// Return the NodeIndex of the File node that contains `sym_idx` via a Contains or ChildOf edge.
fn find_containing_file_idx(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<NodeIndex> {
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::graph::{
        CodeGraph,
        node::{SymbolInfo, SymbolKind},
    };

    /// Build a three-file graph:
    ///   a.ts defines `foo`; b.ts imports a.ts; c.ts imports b.ts (transitive).
    fn three_file_chain() -> (
        CodeGraph,
        PathBuf,
        NodeIndex,
        NodeIndex,
        NodeIndex,
        NodeIndex,
    ) {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        let b_file = graph.add_file(root.join("b.ts"), "typescript");
        graph.add_resolved_import(b_file, a_file, "./a");

        let c_file = graph.add_file(root.join("c.ts"), "typescript");
        graph.add_resolved_import(c_file, b_file, "./b");

        (graph, root, foo_sym, a_file, b_file, c_file)
    }

    #[test]
    fn test_direct_importer_in_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_b = results.iter().any(|r| r.file_path.ends_with("b.ts"));
        assert!(
            has_b,
            "b.ts directly imports a.ts and must appear in blast radius"
        );
    }

    #[test]
    fn test_transitive_importer_in_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_c = results.iter().any(|r| r.file_path.ends_with("c.ts"));
        assert!(
            has_c,
            "c.ts transitively imports a.ts and must appear in blast radius"
        );
    }

    #[test]
    fn test_defining_file_excluded_from_blast_radius() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        let has_a = results.iter().any(|r| r.file_path.ends_with("a.ts"));
        assert!(
            !has_a,
            "a.ts defines foo and should NOT appear in its own blast radius"
        );
    }

    #[test]
    fn test_non_importing_file_excluded() {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        // unrelated.ts has no edge to a.ts.
        let _unrelated = graph.add_file(root.join("unrelated.ts"), "typescript");

        let results = blast_radius(&graph, &[foo_sym], &root);
        let has_unrelated = results
            .iter()
            .any(|r| r.file_path.ends_with("unrelated.ts"));
        assert!(
            !has_unrelated,
            "unrelated.ts should not appear in blast radius"
        );
    }

    #[test]
    fn test_depth_tracking() {
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);

        // b.ts is at depth 1 (directly imports a.ts), c.ts is at depth 2.
        let b_result = results
            .iter()
            .find(|r| r.file_path.ends_with("b.ts"))
            .unwrap();
        let c_result = results
            .iter()
            .find(|r| r.file_path.ends_with("c.ts"))
            .unwrap();

        assert_eq!(b_result.depth, 1, "b.ts should be at depth 1");
        assert_eq!(c_result.depth, 2, "c.ts should be at depth 2");
    }

    #[test]
    fn test_calls_edges_not_followed_in_blast_radius() {
        // A Calls edge from caller.ts to foo should NOT make caller.ts appear in blast radius.
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        // caller.ts has a Calls edge to foo but NOT a ResolvedImport edge to a.ts.
        let caller_file = graph.add_file(root.join("caller.ts"), "typescript");
        graph.add_calls_edge(caller_file, foo_sym);

        let results = blast_radius(&graph, &[foo_sym], &root);
        let has_caller = results.iter().any(|r| r.file_path.ends_with("caller.ts"));
        assert!(
            !has_caller,
            "Calls edge should not be followed in blast radius BFS"
        );
    }

    // ---------------------------------------------------------------------------
    // Confidence tier tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_confidence_tier_high_depth_1() {
        let (tier, basis) = score_confidence(1, false);
        assert_eq!(tier, ConfidenceTier::High);
        assert!(
            basis.contains("direct caller"),
            "basis should mention 'direct caller', got: {basis}"
        );
        assert!(
            basis.contains("1"),
            "basis should mention depth 1, got: {basis}"
        );
    }

    #[test]
    fn test_confidence_tier_medium_depth_2() {
        let (tier, basis) = score_confidence(2, false);
        assert_eq!(tier, ConfidenceTier::Medium);
        assert!(
            basis.contains("transitive dependency"),
            "basis should mention 'transitive dependency', got: {basis}"
        );
        assert!(
            basis.contains("2"),
            "basis should mention depth 2, got: {basis}"
        );
    }

    #[test]
    fn test_confidence_tier_medium_depth_3() {
        let (tier, _basis) = score_confidence(3, false);
        assert_eq!(tier, ConfidenceTier::Medium);
    }

    #[test]
    fn test_confidence_tier_low_depth_4() {
        let (tier, basis) = score_confidence(4, false);
        assert_eq!(tier, ConfidenceTier::Low);
        assert!(
            basis.contains("deep transitive dependency"),
            "basis should mention 'deep transitive dependency', got: {basis}"
        );
        assert!(
            basis.contains("4"),
            "basis should mention depth 4, got: {basis}"
        );
    }

    #[test]
    fn test_confidence_calls_edge_boosts_to_high() {
        // Depth 3, but has a CALLS edge to the target symbol -> HIGH
        let (tier, basis) = score_confidence(3, true);
        assert_eq!(
            tier,
            ConfidenceTier::High,
            "CALLS edge should boost confidence to HIGH regardless of depth"
        );
        assert!(
            basis.contains("direct caller"),
            "basis should mention 'direct caller', got: {basis}"
        );
    }

    #[test]
    fn test_confidence_in_blast_radius_depth_1() {
        // b.ts is at depth 1 -> should be HIGH
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);
        let b_result = results
            .iter()
            .find(|r| r.file_path.ends_with("b.ts"))
            .unwrap();
        assert_eq!(
            b_result.confidence,
            ConfidenceTier::High,
            "depth 1 should yield HIGH confidence"
        );
    }

    #[test]
    fn test_confidence_in_blast_radius_depth_2() {
        // c.ts is at depth 2 -> should be MEDIUM
        let (graph, root, foo_sym, _, _, _) = three_file_chain();
        let results = blast_radius(&graph, &[foo_sym], &root);
        let c_result = results
            .iter()
            .find(|r| r.file_path.ends_with("c.ts"))
            .unwrap();
        assert_eq!(
            c_result.confidence,
            ConfidenceTier::Medium,
            "depth 2 should yield MEDIUM confidence"
        );
    }

    // ---------------------------------------------------------------------------
    // Risk tier tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_risk_tier_high() {
        let risk = classify_risk(25, 20, 5);
        assert_eq!(
            risk,
            RiskTier::High,
            "25 downstream files should be HIGH risk"
        );
    }

    #[test]
    fn test_risk_tier_medium() {
        let risk = classify_risk(10, 20, 5);
        assert_eq!(
            risk,
            RiskTier::Medium,
            "10 downstream files should be MEDIUM risk"
        );
    }

    #[test]
    fn test_risk_tier_low() {
        let risk = classify_risk(3, 20, 5);
        assert_eq!(risk, RiskTier::Low, "3 downstream files should be LOW risk");
    }

    #[test]
    fn test_risk_tier_custom_thresholds() {
        // With thresholds 10/3, 8 files should be Medium
        let risk = classify_risk(8, 10, 3);
        assert_eq!(
            risk,
            RiskTier::Medium,
            "8 files with thresholds 10/3 should be MEDIUM risk"
        );
    }

    // ---------------------------------------------------------------------------
    // diff_impact tests
    // ---------------------------------------------------------------------------

    /// Build an import chain: a.ts -> b.ts -> c.ts (b imports a, c imports b)
    /// Returns (graph, root, a_file_path, b_file_path, c_file_path)
    fn import_chain_graph() -> (CodeGraph, PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = PathBuf::from("/proj");
        let mut graph = CodeGraph::new();

        let a_file = graph.add_file(root.join("a.ts"), "typescript");
        let _foo_sym = graph.add_symbol(
            a_file,
            SymbolInfo {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        let b_file = graph.add_file(root.join("b.ts"), "typescript");
        graph.add_resolved_import(b_file, a_file, "./a");

        let c_file = graph.add_file(root.join("c.ts"), "typescript");
        graph.add_resolved_import(c_file, b_file, "./b");

        (
            graph,
            root.clone(),
            root.join("a.ts"),
            root.join("b.ts"),
            root.join("c.ts"),
        )
    }

    #[test]
    fn test_diff_impact_maps_changed_files() {
        let (graph, root, a_path, _b_path, _c_path) = import_chain_graph();
        let results = diff_impact(&graph, std::slice::from_ref(&a_path), &root, 20, 5);

        assert!(
            !results.is_empty(),
            "should have results for changed file a.ts"
        );
        let a_result = results
            .iter()
            .find(|r| r.changed_file == a_path)
            .expect("a.ts should appear in results");

        let has_b = a_result
            .affected
            .iter()
            .any(|r| r.file_path.ends_with("b.ts"));
        let has_c = a_result
            .affected
            .iter()
            .any(|r| r.file_path.ends_with("c.ts"));
        assert!(has_b, "b.ts should be in affected files for a.ts change");
        assert!(has_c, "c.ts should be in affected files for a.ts change");
    }

    #[test]
    fn test_diff_impact_no_changed_files() {
        let (graph, root, _, _, _) = import_chain_graph();
        let results = diff_impact(&graph, &[], &root, 20, 5);
        assert!(
            results.is_empty(),
            "empty diff should produce empty results"
        );
    }

    #[test]
    fn test_diff_impact_changed_file_not_in_graph() {
        let (graph, root, _, _, _) = import_chain_graph();
        let nonexistent = root.join("nonexistent.ts");
        let results = diff_impact(&graph, &[nonexistent], &root, 20, 5);
        assert!(
            results.is_empty(),
            "file not in graph should be skipped gracefully"
        );
    }

    #[test]
    fn test_diff_impact_with_risk_tier() {
        // Use very low thresholds so 2 files = HIGH risk
        let (graph, root, a_path, _, _) = import_chain_graph();
        let results = diff_impact(&graph, std::slice::from_ref(&a_path), &root, 1, 1);

        let a_result = results
            .iter()
            .find(|r| r.changed_file == a_path)
            .expect("a.ts should appear in results");

        // a.ts change affects b.ts and c.ts (2 files), threshold is 1 -> HIGH
        assert_eq!(
            a_result.risk,
            RiskTier::High,
            "2 affected files with threshold 1 should be HIGH risk"
        );
    }
}

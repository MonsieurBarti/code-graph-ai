use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::graph::{CodeGraph, edge::EdgeKind, node::GraphNode};
use petgraph::Direction;
use petgraph::visit::EdgeRef;

/// Subdirectory under CACHE_DIR for snapshot storage.
pub const SNAPSHOTS_DIR: &str = "snapshots";
/// Maximum number of stored snapshots before auto-rotation deletes oldest.
pub const MAX_SNAPSHOTS: usize = 10;

// ---------------------------------------------------------------------------
// Snapshot data types
// ---------------------------------------------------------------------------

/// A lightweight JSON fingerprint of the code graph at a point in time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphSnapshot {
    pub name: String,
    /// Unix timestamp seconds when snapshot was created.
    pub created_at: u64,
    pub project_root: String,
    /// Key = relative path from project root.
    pub files: HashMap<String, SnapshotFile>,
}

/// Fingerprint of a single file within a snapshot.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SnapshotFile {
    pub symbol_count: usize,
    /// Number of outgoing ResolvedImport edges from this file.
    pub import_count: usize,
    /// Number of incoming ResolvedImport/BarrelReExportAll edges to this file.
    pub importer_count: usize,
    pub symbols: Vec<SnapshotSymbol>,
}

/// Fingerprint of a single symbol within a snapshot file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SnapshotSymbol {
    pub name: String,
    /// Kind string via kind_to_str (e.g. "fn", "struct").
    pub kind: String,
    pub line: usize,
    /// Number of incoming Calls edges.
    pub caller_count: usize,
}

// ---------------------------------------------------------------------------
// Diff result types
// ---------------------------------------------------------------------------

/// The result of comparing two graph snapshots.
pub struct GraphDiff {
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    /// (file, symbol_name)
    pub added_symbols: Vec<(String, String)>,
    /// (file, symbol_name)
    pub removed_symbols: Vec<(String, String)>,
    pub modified_symbols: Vec<SymbolChange>,
}

/// A symbol that changed between two snapshots.
pub struct SymbolChange {
    pub file: String,
    pub name: String,
    /// Human-readable change descriptions, e.g. ["line 10 → 15", "callers 3 → 5"].
    pub changes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Snapshot builder
// ---------------------------------------------------------------------------

/// Build a GraphSnapshot from the current live graph.
pub fn graph_to_snapshot(graph: &CodeGraph, root: &Path, name: &str) -> GraphSnapshot {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut files: HashMap<String, SnapshotFile> = HashMap::new();

    for idx in graph.graph.node_indices() {
        if let GraphNode::File(ref file_info) = graph.graph[idx] {
            // Compute relative path from root
            let rel = file_info
                .path
                .strip_prefix(root)
                .unwrap_or(&file_info.path);
            let rel_str = rel.to_string_lossy().to_string();

            // Count incoming ResolvedImport / BarrelReExportAll edges (importers)
            let importer_count = graph
                .graph
                .edges_directed(idx, Direction::Incoming)
                .filter(|e| {
                    matches!(
                        e.weight(),
                        EdgeKind::ResolvedImport { .. } | EdgeKind::BarrelReExportAll
                    )
                })
                .count();

            // Count outgoing ResolvedImport edges (imports)
            let import_count = graph
                .graph
                .edges_directed(idx, Direction::Outgoing)
                .filter(|e| matches!(e.weight(), EdgeKind::ResolvedImport { .. }))
                .count();

            // Collect symbols via outgoing Contains edges
            let mut symbols: Vec<SnapshotSymbol> = Vec::new();
            for edge in graph.graph.edges_directed(idx, Direction::Outgoing) {
                if let EdgeKind::Contains = edge.weight() {
                    let sym_idx = edge.target();
                    if let GraphNode::Symbol(ref sym_info) = graph.graph[sym_idx] {
                        // Count incoming Calls edges
                        let caller_count = graph
                            .graph
                            .edges_directed(sym_idx, Direction::Incoming)
                            .filter(|e| matches!(e.weight(), EdgeKind::Calls))
                            .count();

                        symbols.push(SnapshotSymbol {
                            name: sym_info.name.clone(),
                            kind: crate::query::find::kind_to_str(&sym_info.kind).to_string(),
                            line: sym_info.line,
                            caller_count,
                        });
                    }
                }
            }

            let symbol_count = symbols.len();
            files.insert(
                rel_str,
                SnapshotFile {
                    symbol_count,
                    import_count,
                    importer_count,
                    symbols,
                },
            );
        }
    }

    GraphSnapshot {
        name: name.to_string(),
        created_at,
        project_root: root.to_string_lossy().to_string(),
        files,
    }
}

// ---------------------------------------------------------------------------
// Snapshot persistence
// ---------------------------------------------------------------------------

/// Returns the path to the snapshot directory for a project.
pub fn snapshot_dir(project_root: &Path) -> PathBuf {
    project_root
        .join(crate::cache::envelope::CACHE_DIR)
        .join(SNAPSHOTS_DIR)
}

/// Returns the path to a specific snapshot file.
pub fn snapshot_path(project_root: &Path, name: &str) -> PathBuf {
    snapshot_dir(project_root).join(format!("{}.json", name))
}

/// Validate a snapshot name: alphanumeric, hyphens, underscores. Length 1-64.
fn validate_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("snapshot name cannot be empty");
    }
    if name.len() > 64 {
        anyhow::bail!(
            "snapshot name too long ({} chars, max 64)",
            name.len()
        );
    }
    for ch in name.chars() {
        if !ch.is_alphanumeric() && ch != '-' && ch != '_' {
            anyhow::bail!(
                "snapshot name '{}' contains invalid character '{}' — only alphanumeric, hyphens, and underscores allowed",
                name, ch
            );
        }
    }
    Ok(())
}

/// Create a named snapshot of the current graph, writing it to disk.
///
/// Validates the name, auto-rotates if more than MAX_SNAPSHOTS exist,
/// then writes a pretty-printed JSON file.
pub fn create_snapshot(graph: &CodeGraph, root: &Path, name: &str) -> anyhow::Result<()> {
    validate_name(name)?;

    let dir = snapshot_dir(root);
    std::fs::create_dir_all(&dir)?;

    // Auto-rotate: delete oldest if at cap
    let existing = list_snapshots(root)?;
    if existing.len() >= MAX_SNAPSHOTS {
        // existing is sorted newest first; oldest is last
        if let Some((oldest_name, _)) = existing.last() {
            let oldest_path = snapshot_path(root, oldest_name);
            let _ = std::fs::remove_file(oldest_path);
        }
    }

    let snapshot = graph_to_snapshot(graph, root, name);
    let json = serde_json::to_string_pretty(&snapshot)?;
    std::fs::write(snapshot_path(root, name), json)?;

    Ok(())
}

/// Load a named snapshot from disk.
pub fn load_snapshot(project_root: &Path, name: &str) -> anyhow::Result<GraphSnapshot> {
    let path = snapshot_path(project_root, name);
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("snapshot '{}' not found: {}", name, e))?;
    let snapshot: GraphSnapshot = serde_json::from_str(&contents)?;
    Ok(snapshot)
}

/// List all stored snapshots, sorted by created_at descending (newest first).
///
/// Returns `(name, created_at)` pairs.
pub fn list_snapshots(project_root: &Path) -> anyhow::Result<Vec<(String, u64)>> {
    let dir = snapshot_dir(project_root);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut results: Vec<(String, u64)> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                // Parse the JSON to get created_at
                match std::fs::read_to_string(&path) {
                    Ok(contents) => {
                        if let Ok(snap) = serde_json::from_str::<GraphSnapshot>(&contents) {
                            results.push((stem.to_string(), snap.created_at));
                        }
                    }
                    Err(_) => {} // skip unreadable files
                }
            }
        }
    }

    // Sort newest first
    results.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(results)
}

/// Delete a named snapshot. Returns error if not found.
pub fn delete_snapshot(project_root: &Path, name: &str) -> anyhow::Result<()> {
    let path = snapshot_path(project_root, name);
    std::fs::remove_file(&path)
        .map_err(|e| anyhow::anyhow!("snapshot '{}' not found: {}", name, e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Diff computation
// ---------------------------------------------------------------------------

/// Compare two snapshots (or a snapshot against the current live graph).
///
/// - `from`: name of the base snapshot
/// - `to`: optional name of the target snapshot; if None, uses the live graph
/// - `graph`: the current live graph (used when `to` is None)
pub fn compute_diff(
    root: &Path,
    from: &str,
    to: Option<&str>,
    graph: &CodeGraph,
) -> Result<GraphDiff, String> {
    let from_snap = load_snapshot(root, from)
        .map_err(|e| format!("cannot load snapshot '{}': {}", from, e))?;

    let to_snap: GraphSnapshot = match to {
        Some(name) => load_snapshot(root, name)
            .map_err(|e| format!("cannot load snapshot '{}': {}", name, e))?,
        None => graph_to_snapshot(graph, root, "__live__"),
    };

    let from_files = &from_snap.files;
    let to_files = &to_snap.files;

    let mut added_files: Vec<String> = Vec::new();
    let mut removed_files: Vec<String> = Vec::new();
    let mut added_symbols: Vec<(String, String)> = Vec::new();
    let mut removed_symbols: Vec<(String, String)> = Vec::new();
    let mut modified_symbols: Vec<SymbolChange> = Vec::new();

    // Files in `to` but not `from` = added
    for key in to_files.keys() {
        if !from_files.contains_key(key) {
            added_files.push(key.clone());
        }
    }

    // Files in `from` but not `to` = removed
    for key in from_files.keys() {
        if !to_files.contains_key(key) {
            removed_files.push(key.clone());
        }
    }

    // Files in both: compare symbols
    for (file_key, from_file) in from_files {
        if let Some(to_file) = to_files.get(file_key) {
            // Build maps from symbol name -> snapshot symbol
            let from_syms: HashMap<&str, &SnapshotSymbol> = from_file
                .symbols
                .iter()
                .map(|s| (s.name.as_str(), s))
                .collect();
            let to_syms: HashMap<&str, &SnapshotSymbol> = to_file
                .symbols
                .iter()
                .map(|s| (s.name.as_str(), s))
                .collect();

            // Added symbols: in `to` but not `from`
            for name in to_syms.keys() {
                if !from_syms.contains_key(name) {
                    added_symbols.push((file_key.clone(), name.to_string()));
                }
            }

            // Removed symbols: in `from` but not `to`
            for name in from_syms.keys() {
                if !to_syms.contains_key(name) {
                    removed_symbols.push((file_key.clone(), name.to_string()));
                }
            }

            // Modified symbols: in both — check for differences
            for (name, from_sym) in &from_syms {
                if let Some(to_sym) = to_syms.get(name) {
                    let mut changes: Vec<String> = Vec::new();
                    if from_sym.kind != to_sym.kind {
                        changes.push(format!("kind {} → {}", from_sym.kind, to_sym.kind));
                    }
                    if from_sym.line != to_sym.line {
                        changes.push(format!("line {} → {}", from_sym.line, to_sym.line));
                    }
                    if from_sym.caller_count != to_sym.caller_count {
                        changes.push(format!(
                            "callers {} → {}",
                            from_sym.caller_count, to_sym.caller_count
                        ));
                    }
                    if !changes.is_empty() {
                        modified_symbols.push(SymbolChange {
                            file: file_key.clone(),
                            name: name.to_string(),
                            changes,
                        });
                    }
                }
            }
        }
    }

    // Sort for deterministic output
    added_files.sort();
    removed_files.sort();
    added_symbols.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    removed_symbols.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    modified_symbols.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));

    Ok(GraphDiff {
        added_files,
        removed_files,
        added_symbols,
        removed_symbols,
        modified_symbols,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::CodeGraph;
    use tempfile::TempDir;

    /// Build a minimal graph with a single file + one symbol for testing.
    fn build_test_graph() -> (CodeGraph, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();

        // Create a dummy source file
        let src_path = root.join("src").join("lib.rs");
        std::fs::create_dir_all(src_path.parent().unwrap()).unwrap();
        std::fs::write(&src_path, "pub fn hello() {}").unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.add_file(src_path, "rust");

        use crate::graph::node::{SymbolInfo, SymbolKind, SymbolVisibility};
        let sym = SymbolInfo {
            name: "hello".to_string(),
            kind: SymbolKind::Function,
            line: 1,
            col: 0,
            is_exported: true,
            is_default: false,
            visibility: SymbolVisibility::Pub,
            trait_impl: None,
        };
        graph.add_symbol(file_idx, sym);

        (graph, tmp)
    }

    #[test]
    fn test_graph_to_snapshot() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        let snap = graph_to_snapshot(&graph, root, "test");
        assert_eq!(snap.name, "test");
        assert!(!snap.files.is_empty(), "snapshot should have at least one file");

        // Check the file entry
        let file_key = snap.files.keys().next().unwrap();
        let snap_file = &snap.files[file_key];
        assert_eq!(snap_file.symbol_count, 1);
        assert_eq!(snap_file.symbols.len(), 1);
        assert_eq!(snap_file.symbols[0].name, "hello");
        assert_eq!(snap_file.symbols[0].kind, "function");
    }

    #[test]
    fn test_create_and_load_snapshot() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        create_snapshot(&graph, root, "baseline").expect("create should succeed");

        let loaded = load_snapshot(root, "baseline").expect("load should succeed");
        assert_eq!(loaded.name, "baseline");
        assert!(!loaded.files.is_empty());
    }

    #[test]
    fn test_auto_rotate_cap() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        // Create MAX_SNAPSHOTS + 1 snapshots
        for i in 0..=MAX_SNAPSHOTS {
            // Small sleep to ensure distinct created_at timestamps
            // We manipulate directly to avoid slow tests by using names
            let name = format!("snap-{:02}", i);
            create_snapshot(&graph, root, &name).expect("create should succeed");
        }

        // There should be at most MAX_SNAPSHOTS files
        let listed = list_snapshots(root).expect("list should succeed");
        assert!(
            listed.len() <= MAX_SNAPSHOTS,
            "auto-rotate should keep at most {} snapshots, found {}",
            MAX_SNAPSHOTS,
            listed.len()
        );
    }

    #[test]
    fn test_snapshot_name_validation() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        // Valid names
        assert!(create_snapshot(&graph, root, "valid-name").is_ok());
        assert!(create_snapshot(&graph, root, "valid_name").is_ok());
        assert!(create_snapshot(&graph, root, "ValidName123").is_ok());

        // Invalid names
        assert!(create_snapshot(&graph, root, "").is_err(), "empty name should fail");
        assert!(
            create_snapshot(&graph, root, "has space").is_err(),
            "name with space should fail"
        );
        assert!(
            create_snapshot(&graph, root, "has/slash").is_err(),
            "name with slash should fail"
        );
        assert!(
            create_snapshot(&graph, root, "has.dot").is_err(),
            "name with dot should fail"
        );
        // 65-char name
        let long_name = "a".repeat(65);
        assert!(
            create_snapshot(&graph, root, &long_name).is_err(),
            "name longer than 64 chars should fail"
        );
    }

    fn make_snapshot(name: &str, files: HashMap<String, SnapshotFile>) -> GraphSnapshot {
        GraphSnapshot {
            name: name.to_string(),
            created_at: 0,
            project_root: "/tmp".to_string(),
            files,
        }
    }

    fn make_file(symbols: Vec<SnapshotSymbol>) -> SnapshotFile {
        SnapshotFile {
            symbol_count: symbols.len(),
            import_count: 0,
            importer_count: 0,
            symbols,
        }
    }

    fn make_sym(name: &str, kind: &str, line: usize, callers: usize) -> SnapshotSymbol {
        SnapshotSymbol {
            name: name.to_string(),
            kind: kind.to_string(),
            line,
            caller_count: callers,
        }
    }

    /// Helper: diff two manually constructed snapshots (no disk I/O needed).
    fn diff_snapshots(from: &GraphSnapshot, to: &GraphSnapshot) -> GraphDiff {
        let mut added_files = Vec::new();
        let mut removed_files = Vec::new();
        let mut added_symbols = Vec::new();
        let mut removed_symbols = Vec::new();
        let mut modified_symbols = Vec::new();

        for key in to.files.keys() {
            if !from.files.contains_key(key) {
                added_files.push(key.clone());
            }
        }
        for key in from.files.keys() {
            if !to.files.contains_key(key) {
                removed_files.push(key.clone());
            }
        }
        for (file_key, from_file) in &from.files {
            if let Some(to_file) = to.files.get(file_key) {
                let from_syms: HashMap<&str, &SnapshotSymbol> =
                    from_file.symbols.iter().map(|s| (s.name.as_str(), s)).collect();
                let to_syms: HashMap<&str, &SnapshotSymbol> =
                    to_file.symbols.iter().map(|s| (s.name.as_str(), s)).collect();

                for name in to_syms.keys() {
                    if !from_syms.contains_key(name) {
                        added_symbols.push((file_key.clone(), name.to_string()));
                    }
                }
                for name in from_syms.keys() {
                    if !to_syms.contains_key(name) {
                        removed_symbols.push((file_key.clone(), name.to_string()));
                    }
                }
                for (name, from_sym) in &from_syms {
                    if let Some(to_sym) = to_syms.get(name) {
                        let mut changes = Vec::new();
                        if from_sym.kind != to_sym.kind {
                            changes.push(format!("kind {} → {}", from_sym.kind, to_sym.kind));
                        }
                        if from_sym.line != to_sym.line {
                            changes.push(format!("line {} → {}", from_sym.line, to_sym.line));
                        }
                        if from_sym.caller_count != to_sym.caller_count {
                            changes.push(format!(
                                "callers {} → {}",
                                from_sym.caller_count, to_sym.caller_count
                            ));
                        }
                        if !changes.is_empty() {
                            modified_symbols.push(SymbolChange {
                                file: file_key.clone(),
                                name: name.to_string(),
                                changes,
                            });
                        }
                    }
                }
            }
        }

        added_files.sort();
        removed_files.sort();
        added_symbols.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        removed_symbols.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        modified_symbols.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));

        GraphDiff {
            added_files,
            removed_files,
            added_symbols,
            removed_symbols,
            modified_symbols,
        }
    }

    #[test]
    fn test_diff_added_file() {
        let from = make_snapshot("from", HashMap::new());
        let mut to_files = HashMap::new();
        to_files.insert("src/new.rs".to_string(), make_file(vec![]));
        let to = make_snapshot("to", to_files);

        let diff = diff_snapshots(&from, &to);
        assert_eq!(diff.added_files, vec!["src/new.rs"]);
        assert!(diff.removed_files.is_empty());
    }

    #[test]
    fn test_diff_removed_file() {
        let mut from_files = HashMap::new();
        from_files.insert("src/old.rs".to_string(), make_file(vec![]));
        let from = make_snapshot("from", from_files);
        let to = make_snapshot("to", HashMap::new());

        let diff = diff_snapshots(&from, &to);
        assert!(diff.added_files.is_empty());
        assert_eq!(diff.removed_files, vec!["src/old.rs"]);
    }

    #[test]
    fn test_diff_added_symbol() {
        let mut from_files = HashMap::new();
        from_files.insert("src/lib.rs".to_string(), make_file(vec![]));
        let from = make_snapshot("from", from_files);

        let mut to_files = HashMap::new();
        to_files.insert(
            "src/lib.rs".to_string(),
            make_file(vec![make_sym("new_fn", "function", 5, 0)]),
        );
        let to = make_snapshot("to", to_files);

        let diff = diff_snapshots(&from, &to);
        assert!(diff.added_files.is_empty());
        assert_eq!(diff.added_symbols, vec![("src/lib.rs".to_string(), "new_fn".to_string())]);
        assert!(diff.removed_symbols.is_empty());
    }

    #[test]
    fn test_diff_removed_symbol() {
        let mut from_files = HashMap::new();
        from_files.insert(
            "src/lib.rs".to_string(),
            make_file(vec![make_sym("old_fn", "function", 5, 0)]),
        );
        let from = make_snapshot("from", from_files);

        let mut to_files = HashMap::new();
        to_files.insert("src/lib.rs".to_string(), make_file(vec![]));
        let to = make_snapshot("to", to_files);

        let diff = diff_snapshots(&from, &to);
        assert!(diff.removed_files.is_empty());
        assert_eq!(
            diff.removed_symbols,
            vec![("src/lib.rs".to_string(), "old_fn".to_string())]
        );
        assert!(diff.added_symbols.is_empty());
    }

    #[test]
    fn test_diff_modified_symbol() {
        let mut from_files = HashMap::new();
        from_files.insert(
            "src/lib.rs".to_string(),
            make_file(vec![make_sym("parse", "function", 10, 3)]),
        );
        let from = make_snapshot("from", from_files);

        let mut to_files = HashMap::new();
        to_files.insert(
            "src/lib.rs".to_string(),
            make_file(vec![make_sym("parse", "function", 15, 5)]),
        );
        let to = make_snapshot("to", to_files);

        let diff = diff_snapshots(&from, &to);
        assert!(diff.added_symbols.is_empty());
        assert!(diff.removed_symbols.is_empty());
        assert_eq!(diff.modified_symbols.len(), 1);
        let change = &diff.modified_symbols[0];
        assert_eq!(change.name, "parse");
        assert!(change.changes.iter().any(|c| c.contains("line 10")));
        assert!(change.changes.iter().any(|c| c.contains("callers 3")));
    }

    #[test]
    fn test_diff_no_changes() {
        let mut files = HashMap::new();
        files.insert(
            "src/lib.rs".to_string(),
            make_file(vec![make_sym("foo", "function", 1, 0)]),
        );
        let snap1 = make_snapshot("snap1", files.clone());
        let snap2 = make_snapshot("snap2", files);

        let diff = diff_snapshots(&snap1, &snap2);
        assert!(diff.added_files.is_empty());
        assert!(diff.removed_files.is_empty());
        assert!(diff.added_symbols.is_empty());
        assert!(diff.removed_symbols.is_empty());
        assert!(diff.modified_symbols.is_empty());
    }

    #[test]
    fn test_list_snapshots() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        create_snapshot(&graph, root, "snap-a").unwrap();
        create_snapshot(&graph, root, "snap-b").unwrap();

        let listed = list_snapshots(root).unwrap();
        assert_eq!(listed.len(), 2);
        // Verify names are present
        let names: Vec<&str> = listed.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"snap-a"));
        assert!(names.contains(&"snap-b"));
    }

    #[test]
    fn test_delete_snapshot() {
        let (graph, tmp) = build_test_graph();
        let root = tmp.path();

        create_snapshot(&graph, root, "to-delete").unwrap();
        delete_snapshot(root, "to-delete").expect("delete should succeed");

        // Should error when not found
        assert!(
            load_snapshot(root, "to-delete").is_err(),
            "loading deleted snapshot should fail"
        );

        // Should also error when deleting non-existent
        assert!(
            delete_snapshot(root, "nonexistent").is_err(),
            "deleting nonexistent snapshot should fail"
        );
    }
}

// QUERY-03 NOTE: The requirement "git diff files mapped to affected symbols with risk tier
// classification" is already fully satisfied by the existing `diff-impact` CLI subcommand,
// backed by `diff_impact()` in impact.rs.
// The existing IMPACT-03 implementation covers the identical requirement.
// No additional implementation is needed for QUERY-03.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use petgraph::stable_graph::NodeIndex;

use crate::graph::{CodeGraph, node::GraphNode};
use crate::query::refs::find_refs;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single site that must be updated during a rename operation.
///
/// `plan_rename` returns one item per definition site and one per reference site.
/// No files on disk are modified — this is a planning-only function.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RenameItem {
    /// Absolute path of the file containing this occurrence.
    pub file_path: PathBuf,
    /// 1-based line number of the occurrence. 0 means the exact line is unknown (import site).
    pub line: usize,
    /// The current text to be replaced (the old symbol name).
    pub old_text: String,
    /// The replacement text (the new symbol name).
    pub new_text: String,
    /// Optional human-readable note for special cases (e.g. import sites).
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Generate a rename plan for `symbol` → `new_name` without modifying any files on disk.
///
/// Returns one `RenameItem` per:
/// - Definition site (from symbol_index)
/// - Reference site: call sites (with line) and import sites (line=0, note added)
///
/// Items are deduplicated by (file_path, line) and sorted by file_path then line.
pub fn plan_rename(
    graph: &CodeGraph,
    symbol: &str,
    new_name: &str,
    root: &Path,
) -> Vec<RenameItem> {
    let indices: Vec<NodeIndex> = match graph.symbol_index.get(symbol) {
        Some(v) => v.clone(),
        None => return Vec::new(),
    };

    let mut items: Vec<RenameItem> = Vec::new();
    let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();

    // Step 1: Definition sites from symbol_index.
    for &sym_idx in &indices {
        let info = match &graph.graph[sym_idx] {
            GraphNode::Symbol(i) => i.clone(),
            _ => continue,
        };

        // Locate the containing file via Contains or ChildOf chain.
        let file_path = match find_containing_file_path(graph, sym_idx) {
            Some(fp) => fp,
            None => continue,
        };

        let key = (file_path.clone(), info.line);
        if seen.insert(key) {
            items.push(RenameItem {
                file_path,
                line: info.line,
                old_text: symbol.to_string(),
                new_text: new_name.to_string(),
                note: None,
            });
        }
    }

    // Step 2: Reference sites (calls + import refs) via find_refs.
    let refs = find_refs(graph, symbol, &indices, root);
    for r in refs {
        let line = r.line.unwrap_or(0);
        let key = (r.file_path.clone(), line);
        if seen.insert(key) {
            let note = if line == 0 {
                Some("import site — verify manually".to_string())
            } else {
                None
            };
            items.push(RenameItem {
                file_path: r.file_path,
                line,
                old_text: symbol.to_string(),
                new_text: new_name.to_string(),
                note,
            });
        }
    }

    // Step 3: Sort by file_path then line.
    items.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    items
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Find the file path containing a symbol node, using the shared utility.
fn find_containing_file_path(graph: &CodeGraph, sym_idx: NodeIndex) -> Option<PathBuf> {
    let file_idx = super::util::find_containing_file_idx(graph, sym_idx)?;
    if let GraphNode::File(fi) = &graph.graph[file_idx] {
        Some(fi.path.clone())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::graph::node::{SymbolInfo, SymbolKind};

    fn root() -> PathBuf {
        PathBuf::from("/proj")
    }

    #[test]
    fn test_plan_rename_basic() {
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        let def_file = g.add_file(r.join("src/foo.rs"), "rust");
        let foo_sym = g.add_symbol(
            def_file,
            SymbolInfo {
                name: "Foo".into(),
                kind: SymbolKind::Struct,
                line: 10,
                ..Default::default()
            },
        );

        // Two reference sites (callers of Foo).
        let caller1 = g.add_file(r.join("src/bar.rs"), "rust");
        let bar_sym = g.add_symbol(
            caller1,
            SymbolInfo {
                name: "bar".into(),
                kind: SymbolKind::Function,
                line: 5,
                ..Default::default()
            },
        );
        g.add_calls_edge(bar_sym, foo_sym);

        let caller2 = g.add_file(r.join("src/baz.rs"), "rust");
        let baz_sym = g.add_symbol(
            caller2,
            SymbolInfo {
                name: "baz".into(),
                kind: SymbolKind::Function,
                line: 7,
                ..Default::default()
            },
        );
        g.add_calls_edge(baz_sym, foo_sym);

        let items = plan_rename(&g, "Foo", "Bar", &r);

        // 1 definition + 2 call refs = 3 items.
        assert_eq!(items.len(), 3, "expected 3 rename items (1 def + 2 refs)");

        // All items have old_text="Foo" and new_text="Bar".
        for item in &items {
            assert_eq!(item.old_text, "Foo");
            assert_eq!(item.new_text, "Bar");
        }

        // Definition at line 10.
        let def_item = items
            .iter()
            .find(|i| i.file_path.ends_with("foo.rs"))
            .unwrap();
        assert_eq!(def_item.line, 10);
    }

    #[test]
    fn test_plan_rename_no_disk_writes() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let mut g = crate::graph::CodeGraph::new();

        // Create in-memory graph only (no files on disk).
        let f = g.add_file(root.join("src/thing.rs"), "rust");
        g.add_symbol(
            f,
            SymbolInfo {
                name: "MyStruct".into(),
                kind: SymbolKind::Struct,
                line: 1,
                ..Default::default()
            },
        );

        // Record files on disk before rename.
        let files_before: Vec<_> = std::fs::read_dir(&root)
            .map(|d| d.collect::<Vec<_>>())
            .unwrap_or_default();

        let _items = plan_rename(&g, "MyStruct", "RenamedStruct", &root);

        // Files on disk must be identical to before.
        let files_after: Vec<_> = std::fs::read_dir(&root)
            .map(|d| d.collect::<Vec<_>>())
            .unwrap_or_default();
        assert_eq!(
            files_before.len(),
            files_after.len(),
            "plan_rename must not create any files on disk"
        );
    }

    #[test]
    fn test_plan_rename_unknown_symbol() {
        let g = crate::graph::CodeGraph::new();
        let r = root();
        let items = plan_rename(&g, "DoesNotExist", "NewName", &r);
        assert!(
            items.is_empty(),
            "unknown symbol should return empty rename plan"
        );
    }

    #[test]
    fn test_plan_rename_import_refs() {
        let r = root();
        let mut g = crate::graph::CodeGraph::new();

        // foo.ts defines Foo.
        let def_file = g.add_file(r.join("src/foo.ts"), "typescript");
        g.add_symbol(
            def_file,
            SymbolInfo {
                name: "Foo".into(),
                kind: SymbolKind::Class,
                line: 1,
                is_exported: true,
                ..Default::default()
            },
        );

        // bar.ts imports from foo.ts → ResolvedImport edge creates an import ref.
        let importer = g.add_file(r.join("src/bar.ts"), "typescript");
        g.add_resolved_import(importer, def_file, "./foo");

        let items = plan_rename(&g, "Foo", "FooRenamed", &r);

        // Import ref should have line=0 and a note.
        let import_item = items
            .iter()
            .find(|i| i.file_path.ends_with("bar.ts"))
            .expect("import ref for bar.ts expected");

        assert_eq!(import_item.line, 0, "import site should have line=0");
        assert!(
            import_item
                .note
                .as_deref()
                .unwrap_or("")
                .contains("import site"),
            "import site note expected"
        );
    }
}

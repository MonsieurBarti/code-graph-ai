use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;

/// Current cache format version. Bump when graph struct layout changes.
/// Bumped to 2 in Phase 8 when new SymbolKind variants (Struct, Trait, ImplMethod, Const,
/// Static, Macro), SymbolVisibility field, trait_impl field, and EdgeKind variants
/// (ReExport, RustImport) were added — bincode discriminant layout changed.
/// Bumped to 3 in Phase 9 when `GraphNode::Builtin { name }` variant was added,
/// `FileInfo.crate_name: Option<String>` field was added, and `builtin_index` field
/// was added to `CodeGraph` — all change bincode serialization layout.
pub const CACHE_VERSION: u32 = 3;

/// Cache directory name (created in project root).
pub const CACHE_DIR: &str = ".code-graph";
/// Cache file name within CACHE_DIR.
pub const CACHE_FILE: &str = "graph.bin";

/// Metadata for a cached file: mtime (seconds since epoch) + file size.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileMeta {
    pub mtime_secs: u64,
    pub size: u64,
}

/// Envelope wrapping the serialized graph with version and staleness metadata.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CacheEnvelope {
    pub version: u32,
    pub project_root: PathBuf,
    pub file_mtimes: HashMap<PathBuf, FileMeta>,
    pub graph: CodeGraph,
}

/// Build the cache file path for a project: `<project_root>/.code-graph/graph.bin`
pub fn cache_path(project_root: &Path) -> PathBuf {
    project_root.join(CACHE_DIR).join(CACHE_FILE)
}

/// Collect current filesystem metadata (mtime + size) for all files in the graph.
pub fn collect_file_mtimes(graph: &CodeGraph) -> HashMap<PathBuf, FileMeta> {
    let mut mtimes = HashMap::new();
    for path in graph.file_index.keys() {
        if let Ok(metadata) = std::fs::metadata(path) {
            let mtime_secs = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            mtimes.insert(
                path.clone(),
                FileMeta {
                    mtime_secs,
                    size: metadata.len(),
                },
            );
        }
    }
    mtimes
}

/// Save the graph to disk atomically using bincode serialization.
///
/// Writes to a temp file first, then renames to the final path.
/// Creates the `.code-graph/` directory if it doesn't exist.
pub fn save_cache(project_root: &Path, graph: &CodeGraph) -> anyhow::Result<()> {
    let cache_dir = project_root.join(CACHE_DIR);
    std::fs::create_dir_all(&cache_dir)?;

    let file_mtimes = collect_file_mtimes(graph);
    let envelope = CacheEnvelope {
        version: CACHE_VERSION,
        project_root: project_root.to_path_buf(),
        file_mtimes,
        graph: graph.clone(),
    };

    // Atomic write: temp file in same directory, then rename
    let target = cache_path(project_root);
    let mut tmp = tempfile::NamedTempFile::new_in(&cache_dir)?;
    bincode::serde::encode_into_std_write(&envelope, &mut tmp, bincode::config::standard())?;
    tmp.as_file().flush()?;
    tmp.persist(&target)?;

    Ok(())
}

/// Load the cached graph from disk. Returns None if:
/// - Cache file doesn't exist
/// - Cache version doesn't match CACHE_VERSION
/// - Deserialization fails (corrupt cache)
pub fn load_cache(project_root: &Path) -> Option<CacheEnvelope> {
    let target = cache_path(project_root);
    let bytes = std::fs::read(&target).ok()?;
    let result =
        bincode::serde::decode_from_slice::<CacheEnvelope, _>(&bytes, bincode::config::standard());
    match result {
        Ok((envelope, _)) if envelope.version == CACHE_VERSION => Some(envelope),
        _ => None, // version mismatch or corrupt — caller will do full rebuild
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{SymbolInfo, SymbolKind, SymbolVisibility};

    #[test]
    fn test_roundtrip_cache() {
        let mut graph = CodeGraph::new();
        let tmp_dir = tempfile::tempdir().unwrap();
        let fake_file = tmp_dir.path().join("test.ts");
        std::fs::write(&fake_file, "// test").unwrap();

        let f = graph.add_file(fake_file.clone(), "typescript");
        graph.add_symbol(
            f,
            SymbolInfo {
                name: "hello".into(),
                kind: SymbolKind::Function,
                line: 1,
                col: 0,
                is_exported: true,
                is_default: false,
                visibility: SymbolVisibility::Private,
                trait_impl: None,
            },
        );

        // Save
        save_cache(tmp_dir.path(), &graph).unwrap();

        // Load
        let loaded = load_cache(tmp_dir.path()).expect("cache should load");
        assert_eq!(loaded.version, CACHE_VERSION);
        assert_eq!(loaded.graph.file_count(), 1);
        assert_eq!(loaded.graph.symbol_count(), 1);
        assert!(loaded.file_mtimes.contains_key(&fake_file));
    }

    #[test]
    fn test_load_missing_cache_returns_none() {
        let tmp_dir = tempfile::tempdir().unwrap();
        assert!(load_cache(tmp_dir.path()).is_none());
    }
}

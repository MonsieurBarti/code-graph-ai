/// Vector store wrapping a usearch HNSW index for fast nearest-neighbor symbol search.
///
/// Provides:
/// - `add()`: insert an embedding with associated `SymbolMeta` metadata
/// - `search()`: find the top-k nearest neighbors for a query vector
/// - `save()`: persist the index + metadata map to disk (two files)
/// - `load()`: load from disk and reconstruct the full store
///
/// Persistence strategy:
/// - `vectors.usearch` — the binary HNSW index written by usearch
/// - `vectors_meta.bin` — bincode-serialized `HashMap<u64, SymbolMeta>`
///
/// Key design: usearch keys are sequential `u64` values managed by `next_key`.
/// This prevents key collisions when adding the same symbol name multiple times.
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Metadata associated with a single indexed symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolMeta {
    /// Absolute or project-relative path to the source file containing this symbol.
    pub file_path: String,
    /// The symbol's identifier name (e.g. "authenticate_user").
    pub symbol_name: String,
    /// 1-based line number where the symbol is defined.
    pub line_start: usize,
    /// Symbol kind string (e.g. "function", "struct", "class", "symbol").
    pub kind: String,
}

/// HNSW vector store backed by usearch.
///
/// Stores per-symbol embeddings with metadata mapping for retrieval.
/// Sequential `next_key` counter ensures unique keys for all symbols.
pub struct VectorStore {
    /// The underlying usearch HNSW index.
    index: Index,
    /// Maps each usearch key (u64) back to the symbol's metadata.
    key_to_symbol: HashMap<u64, SymbolMeta>,
    /// Auto-incrementing key counter for new embeddings.
    next_key: u64,
}

impl VectorStore {
    /// Create a new empty `VectorStore` with cosine similarity metric and F32 quantization.
    ///
    /// `dimensions` should match the embedding model's output size (384 for bge-small-en-v1.5).
    pub fn new(dimensions: usize) -> Result<Self> {
        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,     // auto (defaults to 16 for HNSW)
            expansion_add: 0,    // auto
            expansion_search: 0, // auto
            multi: false,
        };
        let index = Index::new(&options)
            .map_err(|e| anyhow::anyhow!("failed to create usearch index: {}", e))?;
        Ok(Self {
            index,
            key_to_symbol: HashMap::new(),
            next_key: 0,
        })
    }

    /// Pre-allocate capacity in the index for `capacity` embeddings.
    ///
    /// **Must be called before the first `add()`** — usearch HNSW indices require capacity
    /// reservation before insertion. Calling `add()` without `reserve()` will cause a SIGSEGV.
    pub fn reserve(&mut self, capacity: usize) -> Result<()> {
        self.index
            .reserve(capacity)
            .map_err(|e| anyhow::anyhow!("failed to reserve index capacity: {}", e))
    }

    /// Add a single embedding to the store with associated metadata.
    ///
    /// Returns the assigned key (sequential u64). The key is opaque to callers
    /// but is used internally to retrieve `SymbolMeta` from search results.
    pub fn add(&mut self, embedding: &[f32], meta: SymbolMeta) -> Result<u64> {
        let key = self.next_key;
        self.index
            .add(key, embedding)
            .map_err(|e| anyhow::anyhow!("failed to add embedding to index: {}", e))?;
        self.key_to_symbol.insert(key, meta);
        self.next_key += 1;
        Ok(key)
    }

    /// Search for the `top_k` nearest neighbors to `query`.
    ///
    /// Returns a list of `(SymbolMeta, distance)` pairs ordered by ascending distance
    /// (closest first). Distance is cosine distance (lower = more similar).
    pub fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<(SymbolMeta, f32)>> {
        let results = self
            .index
            .search(query, top_k)
            .map_err(|e| anyhow::anyhow!("failed to search index: {}", e))?;

        let mut matches = Vec::with_capacity(results.keys.len());
        for (key, distance) in results.keys.iter().zip(results.distances.iter()) {
            if let Some(meta) = self.key_to_symbol.get(key) {
                matches.push((meta.clone(), *distance));
            }
        }
        Ok(matches)
    }

    /// Save the vector store to disk.
    ///
    /// Writes two files to `dir`:
    /// - `vectors.usearch` — the HNSW index binary
    /// - `vectors_meta.bin` — bincode-serialized `HashMap<u64, SymbolMeta>`
    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;

        let index_path = dir.join("vectors.usearch");
        self.index
            .save(
                index_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("invalid index path"))?,
            )
            .map_err(|e| anyhow::anyhow!("failed to save usearch index: {}", e))?;

        let meta_path = dir.join("vectors_meta.bin");
        // Prepare serializable form: convert HashMap<u64, SymbolMeta> to Vec<(u64, SymbolMeta)>
        // because bincode 2.x works well with Vec of tuples.
        let meta_pairs: Vec<(u64, SymbolMeta)> = self
            .key_to_symbol
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        let encoded = bincode::serde::encode_to_vec(&meta_pairs, bincode::config::standard())?;
        std::fs::write(&meta_path, encoded)?;

        Ok(())
    }

    /// Load a `VectorStore` from disk.
    ///
    /// Expects `dir` to contain `vectors.usearch` and `vectors_meta.bin`.
    /// Returns an error if either file is missing or malformed.
    pub fn load(dir: &Path, dimensions: usize) -> Result<Self> {
        let index_path = dir.join("vectors.usearch");
        if !index_path.exists() {
            anyhow::bail!("vector index not found at {}", index_path.display());
        }

        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let index = Index::new(&options)
            .map_err(|e| anyhow::anyhow!("failed to create usearch index for load: {}", e))?;
        index
            .load(
                index_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("invalid index path"))?,
            )
            .map_err(|e| anyhow::anyhow!("failed to load usearch index: {}", e))?;

        let meta_path = dir.join("vectors_meta.bin");
        if !meta_path.exists() {
            anyhow::bail!("vector metadata not found at {}", meta_path.display());
        }
        let encoded = std::fs::read(&meta_path)?;
        let (meta_pairs, _): (Vec<(u64, SymbolMeta)>, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard())?;
        let key_to_symbol: HashMap<u64, SymbolMeta> = meta_pairs.into_iter().collect();
        let next_key = key_to_symbol
            .keys()
            .copied()
            .max()
            .map(|k| k + 1)
            .unwrap_or(0);

        Ok(Self {
            index,
            key_to_symbol,
            next_key,
        })
    }

    /// Returns true if no embeddings have been indexed.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.key_to_symbol.is_empty()
    }

    /// Returns the number of indexed embeddings.
    pub fn len(&self) -> usize {
        self.key_to_symbol.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper to create a simple test embedding with a given seed value.
    fn make_embedding(seed: f32, dims: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dims).map(|i| seed + i as f32 * 0.001).collect();
        // Normalize to unit length (cosine similarity works on unit vectors)
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }

    #[test]
    fn vector_store_new_creates_empty_store() {
        let store = VectorStore::new(384).expect("VectorStore::new should succeed");
        assert!(store.is_empty(), "new store should be empty");
        assert_eq!(store.len(), 0, "new store should have len 0");
    }

    #[test]
    fn symbol_meta_has_required_fields() {
        let meta = SymbolMeta {
            file_path: "src/auth.rs".to_string(),
            symbol_name: "authenticate_user".to_string(),
            line_start: 42,
            kind: "function".to_string(),
        };
        assert_eq!(meta.file_path, "src/auth.rs");
        assert_eq!(meta.symbol_name, "authenticate_user");
        assert_eq!(meta.line_start, 42);
        assert_eq!(meta.kind, "function");
    }

    #[test]
    fn vector_store_add_returns_sequential_keys() {
        // Use 384 dimensions to match actual embedding model output.
        // usearch requires reserve() before add() to avoid SIGSEGV.
        let dims = 384;
        let mut store = VectorStore::new(dims).expect("VectorStore::new should succeed");
        store.reserve(2).expect("reserve should succeed");

        let emb1 = make_embedding(0.9, dims);
        let emb2 = make_embedding(0.1, dims);

        let key1 = store
            .add(
                &emb1,
                SymbolMeta {
                    file_path: "a.rs".to_string(),
                    symbol_name: "foo".to_string(),
                    line_start: 1,
                    kind: "function".to_string(),
                },
            )
            .expect("add should succeed");

        let key2 = store
            .add(
                &emb2,
                SymbolMeta {
                    file_path: "b.rs".to_string(),
                    symbol_name: "bar".to_string(),
                    line_start: 10,
                    kind: "struct".to_string(),
                },
            )
            .expect("add should succeed");

        assert_eq!(key1, 0, "first key should be 0");
        assert_eq!(key2, 1, "second key should be 1");
        assert_eq!(store.len(), 2, "store should have 2 entries");
        assert!(!store.is_empty(), "store should not be empty");
    }

    #[test]
    fn vector_store_search_returns_nearest_neighbor() {
        let dims = 384;
        let mut store = VectorStore::new(dims).expect("VectorStore::new should succeed");
        store.reserve(3).expect("reserve should succeed");

        let emb_auth = make_embedding(0.9, dims);
        let emb_user = make_embedding(0.5, dims);
        let emb_log = make_embedding(0.1, dims);

        store
            .add(
                &emb_auth,
                SymbolMeta {
                    file_path: "auth.rs".to_string(),
                    symbol_name: "authenticate".to_string(),
                    line_start: 5,
                    kind: "function".to_string(),
                },
            )
            .expect("add auth should succeed");
        store
            .add(
                &emb_user,
                SymbolMeta {
                    file_path: "user.rs".to_string(),
                    symbol_name: "get_user".to_string(),
                    line_start: 10,
                    kind: "function".to_string(),
                },
            )
            .expect("add user should succeed");
        store
            .add(
                &emb_log,
                SymbolMeta {
                    file_path: "log.rs".to_string(),
                    symbol_name: "log_event".to_string(),
                    line_start: 15,
                    kind: "function".to_string(),
                },
            )
            .expect("add log should succeed");

        // Query with emb_auth — nearest should be "authenticate"
        let results = store.search(&emb_auth, 1).expect("search should succeed");
        assert_eq!(results.len(), 1, "should return 1 result");
        assert_eq!(
            results[0].0.symbol_name, "authenticate",
            "nearest to auth embedding should be authenticate"
        );
    }

    #[test]
    fn vector_store_search_returns_top_k_ordered_by_distance() {
        let dims = 384;
        let mut store = VectorStore::new(dims).expect("VectorStore::new should succeed");
        store.reserve(3).expect("reserve should succeed");

        // Add 3 vectors with increasing distance from seed 0.9
        let emb_a = make_embedding(0.9, dims);
        let emb_b = make_embedding(0.6, dims);
        let emb_c = make_embedding(0.1, dims);

        store
            .add(
                &emb_a,
                SymbolMeta {
                    file_path: "a.rs".to_string(),
                    symbol_name: "symbol_a".to_string(),
                    line_start: 1,
                    kind: "function".to_string(),
                },
            )
            .unwrap();
        store
            .add(
                &emb_b,
                SymbolMeta {
                    file_path: "b.rs".to_string(),
                    symbol_name: "symbol_b".to_string(),
                    line_start: 2,
                    kind: "function".to_string(),
                },
            )
            .unwrap();
        store
            .add(
                &emb_c,
                SymbolMeta {
                    file_path: "c.rs".to_string(),
                    symbol_name: "symbol_c".to_string(),
                    line_start: 3,
                    kind: "function".to_string(),
                },
            )
            .unwrap();

        // Search for top-2 nearest to emb_a
        let results = store.search(&emb_a, 2).expect("search should succeed");
        assert_eq!(results.len(), 2, "should return 2 results");

        // First result should be "symbol_a" (exact match, distance ~0)
        assert_eq!(
            results[0].0.symbol_name, "symbol_a",
            "first result should be symbol_a (closest)"
        );

        // Distances should be non-decreasing (sorted ascending)
        assert!(
            results[0].1 <= results[1].1,
            "results should be ordered by ascending distance: {} <= {}",
            results[0].1,
            results[1].1
        );
    }

    #[test]
    fn vector_store_save_and_load_round_trip() {
        let tmp = TempDir::new().expect("temp dir should create");
        let dir = tmp.path();
        let dims = 384;

        // Build store with known data
        let mut store = VectorStore::new(dims).expect("VectorStore::new should succeed");
        store.reserve(2).expect("reserve should succeed");

        let emb1 = make_embedding(0.8, dims);
        let emb2 = make_embedding(0.3, dims);

        store
            .add(
                &emb1,
                SymbolMeta {
                    file_path: "src/main.rs".to_string(),
                    symbol_name: "main_fn".to_string(),
                    line_start: 1,
                    kind: "function".to_string(),
                },
            )
            .expect("add emb1 should succeed");
        store
            .add(
                &emb2,
                SymbolMeta {
                    file_path: "src/lib.rs".to_string(),
                    symbol_name: "init_lib".to_string(),
                    line_start: 5,
                    kind: "function".to_string(),
                },
            )
            .expect("add emb2 should succeed");

        // Save
        store.save(dir).expect("save should succeed");

        // Verify files exist
        assert!(
            dir.join("vectors.usearch").exists(),
            "vectors.usearch should exist"
        );
        assert!(
            dir.join("vectors_meta.bin").exists(),
            "vectors_meta.bin should exist"
        );

        // Load
        let loaded = VectorStore::load(dir, dims).expect("load should succeed");

        assert_eq!(loaded.len(), 2, "loaded store should have 2 entries");
        assert!(!loaded.is_empty(), "loaded store should not be empty");

        // Search on loaded store should return the same nearest neighbor
        let results = loaded
            .search(&emb1, 1)
            .expect("search on loaded store should succeed");
        assert_eq!(results.len(), 1, "should return 1 result");
        assert_eq!(
            results[0].0.symbol_name, "main_fn",
            "nearest to emb1 should be main_fn"
        );
        assert_eq!(results[0].0.file_path, "src/main.rs");
        assert_eq!(results[0].0.line_start, 1);
    }

    #[test]
    fn vector_store_load_missing_index_returns_error() {
        let tmp = TempDir::new().expect("temp dir should create");
        let result = VectorStore::load(tmp.path(), 384);
        assert!(result.is_err(), "load from empty dir should fail");
    }
}

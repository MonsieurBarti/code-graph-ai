/// Embedding engine wrapping fastembed's TextEmbedding model.
///
/// Uses BAAI/bge-small-en-v1.5 (384 dimensions) via ONNX inference.
/// The model is downloaded automatically to `~/.cache/fastembed/` on first use.
///
/// `TextEmbedding` is synchronous and CPU-bound; all embedding calls use
/// `tokio::task::spawn_blocking` to avoid blocking the async runtime.
///
/// Thread safety note: `TextEmbedding` may not implement `Send`/`Sync` directly.
/// If wrapping in `Arc<tokio::sync::Mutex<TextEmbedding>>` fails to compile,
/// we fall back to a dedicated embedding thread communicating via `std::sync::mpsc`.
use std::sync::Arc;

use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokio::sync::Mutex;

/// Number of embedding dimensions produced by BAAI/bge-small-en-v1.5.
#[allow(dead_code)]
pub const EMBEDDING_DIMENSIONS: usize = 384;

/// Wraps a fastembed `TextEmbedding` model behind an async interface.
///
/// Internally, the model is wrapped in `Arc<Mutex<TextEmbedding>>` and all
/// embedding work is dispatched via `tokio::task::spawn_blocking` to avoid
/// blocking the async runtime.
pub struct EmbeddingEngine {
    inner: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingEngine {
    /// Create a new `EmbeddingEngine` using the BAAI/bge-small-en-v1.5 model.
    ///
    /// Downloads the model to `~/.cache/fastembed/` if not already cached.
    /// This is a blocking operation — call from a sync context or via `spawn_blocking`.
    pub fn try_new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
        )?;
        Ok(Self {
            inner: Arc::new(Mutex::new(model)),
        })
    }

    /// Embed a batch of text strings, returning a vector of 384-dimensional embeddings.
    ///
    /// Dispatches the CPU-bound embedding work to a blocking thread pool via
    /// `spawn_blocking` to keep the async runtime unblocked.
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let mut guard = model.blocking_lock();
            guard
                .embed(texts, None)
                .map_err(|e| anyhow::anyhow!("embedding failed: {}", e))
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))?
    }
}

/// Embed a slice of symbol descriptors using the provided engine.
///
/// Each symbol is described as `"{name} in {file_path}:{line}"` to give the model
/// enough context for semantic matching. Returns one embedding vector per symbol.
///
/// `symbols` is a slice of `(name, file_path, line_start)` tuples.
pub async fn embed_symbols(
    engine: &EmbeddingEngine,
    symbols: &[(String, String, usize)],
) -> Result<Vec<Vec<f32>>> {
    let texts: Vec<String> = symbols
        .iter()
        .map(|(name, path, line)| format!("{} in {}:{}", name, path, line))
        .collect();
    engine.embed_batch(texts).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that `EmbeddingEngine::try_new()` creates an engine successfully.
    ///
    /// Marked `#[ignore]` because it downloads a ~23MB ONNX model on first run.
    /// Run explicitly with: `cargo test --features rag -- --ignored embedding_engine_new`
    #[test]
    #[ignore = "requires model download (~23MB); run explicitly with --ignored"]
    fn embedding_engine_new() {
        let engine = EmbeddingEngine::try_new();
        assert!(engine.is_ok(), "EmbeddingEngine::try_new() should succeed");
    }

    /// Verifies that `embed_batch` produces vectors with the correct dimensionality (384).
    ///
    /// Marked `#[ignore]` because it requires the ONNX model to be downloaded.
    #[tokio::test]
    #[ignore = "requires model download (~23MB); run explicitly with --ignored"]
    async fn embed_batch_produces_384_dim_vectors() {
        let engine = EmbeddingEngine::try_new().expect("engine should initialize");
        let texts = vec![
            "authenticate_user in src/auth.rs:42".to_string(),
            "UserService in src/services/user.ts:10".to_string(),
        ];
        let embeddings = engine
            .embed_batch(texts)
            .await
            .expect("embed_batch should succeed");
        assert_eq!(embeddings.len(), 2, "should return one embedding per text");
        for emb in &embeddings {
            assert_eq!(
                emb.len(),
                EMBEDDING_DIMENSIONS,
                "each embedding should have {} dimensions",
                EMBEDDING_DIMENSIONS
            );
        }
    }

    /// Verifies the `embed_symbols` function formats symbol descriptors correctly.
    ///
    /// Marked `#[ignore]` because it requires the ONNX model to be downloaded.
    #[tokio::test]
    #[ignore = "requires model download (~23MB); run explicitly with --ignored"]
    async fn embed_symbols_produces_correct_count() {
        let engine = EmbeddingEngine::try_new().expect("engine should initialize");
        let symbols = vec![
            ("my_fn".to_string(), "src/lib.rs".to_string(), 5usize),
            ("MyStruct".to_string(), "src/types.rs".to_string(), 20usize),
            ("run_loop".to_string(), "src/main.rs".to_string(), 100usize),
        ];
        let embeddings = embed_symbols(&engine, &symbols)
            .await
            .expect("embed_symbols should succeed");
        assert_eq!(
            embeddings.len(),
            symbols.len(),
            "should return one embedding per symbol"
        );
        for emb in &embeddings {
            assert_eq!(
                emb.len(),
                EMBEDDING_DIMENSIONS,
                "each embedding should have {} dimensions",
                EMBEDDING_DIMENSIONS
            );
        }
    }
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fastembed::{TextEmbedding, TextInitOptions};
use tracing::info;

use anyhow::{Result, anyhow};

/// Async-safe wrapper around a fastembed TextEmbedding model.
///
/// `TextEmbedding::embed` takes `&mut self` and is CPU-bound, so calls are
/// dispatched via `spawn_blocking` to avoid stalling the Tokio reactor.
#[derive(Clone)]
pub struct EmbeddingService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingService {
    /// Loads the specified embedding model, caching files under `cache_dir`.
    ///
    /// Downloads on first run; subsequent runs reuse the local cache.
    pub fn new(model: fastembed::EmbeddingModel, cache_dir: PathBuf) -> Result<Self> {
        info!(
            "loading embedding model {:?} (cache: {:?})",
            model, cache_dir
        );
        let opts = TextInitOptions::new(model).with_cache_dir(cache_dir);
        let model = TextEmbedding::try_new(opts).map_err(|e| anyhow!("embedding: {e}"))?;
        Ok(EmbeddingService {
            model: Arc::new(Mutex::new(model)),
        })
    }

    /// Embeds a slice of strings in one ONNX inference pass, returning one vector per input.
    ///
    /// Offloads blocking ONNX inference to a Tokio blocking thread.
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = Arc::clone(&self.model);
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().expect("embedding mutex poisoned");
            guard
                .embed(texts, None)
                .map_err(|e| anyhow!("embedding: {e}"))
        })
        .await
        .map_err(|e| anyhow!("embedding join: {e}"))?
    }

    /// Embeds a single string and returns the raw float vector.
    ///
    /// Offloads blocking ONNX inference to a Tokio blocking thread.
    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let model = Arc::clone(&self.model);
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().expect("embedding mutex poisoned");
            let mut batches = guard
                .embed(vec![text], None)
                .map_err(|e| anyhow!("embedding: {e}"))?;
            Ok(batches.pop().unwrap_or_default())
        })
        .await
        .map_err(|e| anyhow!("embedding join: {e}"))?
    }
}

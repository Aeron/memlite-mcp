use std::sync::{Arc, Mutex};

use fastembed::{TextEmbedding, TextInitOptions};
use tracing::info;

use crate::error::{Error, Result};

/// Async-safe wrapper around a fastembed TextEmbedding model.
///
/// `TextEmbedding::embed` takes `&mut self` and is CPU-bound, so calls are
/// dispatched via `spawn_blocking` to avoid stalling the Tokio reactor.
#[derive(Clone)]
pub struct EmbeddingService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingService {
    /// Loads the model identified by `model_str` (e.g. "bge-small-en-v1.5").
    ///
    /// Downloads model files on first run; subsequent runs use the local cache.
    /// Unknown strings fall back to `DEFAULT_MODEL`.
    pub fn new(model: fastembed::EmbeddingModel) -> Result<Self> {
        info!("loading embedding model {:?}", model);
        let model = TextEmbedding::try_new(TextInitOptions::new(model))
            .map_err(|e| Error::Embedding(e.to_string()))?;
        Ok(EmbeddingService {
            model: Arc::new(Mutex::new(model)),
        })
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
                .map_err(|e| Error::Embedding(e.to_string()))?;
            Ok(batches.pop().unwrap_or_default())
        })
        .await
        .map_err(|e| Error::Embedding(e.to_string()))?
    }
}

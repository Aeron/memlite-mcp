use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use tracing::info;

use crate::error::{Error, Result};

/// Default embedding model: BAAI/bge-small-en-v1.5, 384 dimensions.
/// Downloaded to the fastembed cache (~/.cache/huggingface) on first use.
pub const DEFAULT_MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15;

/// Async-safe wrapper around a fastembed TextEmbedding model.
///
/// `TextEmbedding::embed` takes `&mut self` and is CPU-bound, so calls are
/// dispatched via `spawn_blocking` to avoid stalling the Tokio reactor.
#[derive(Clone)]
pub struct EmbeddingService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingService {
    /// Initializes the model specified by `MEMLITE_EMBEDDING_MODEL` env var,
    /// falling back to `DEFAULT_MODEL`.
    ///
    /// Blocks the calling thread until the model files are loaded (downloading
    /// them from HuggingFace on first run; cached afterward).
    pub fn from_env() -> Result<Self> {
        let model_name = Self::model_from_env();
        info!("loading embedding model {:?}", model_name);
        let model = TextEmbedding::try_new(TextInitOptions::new(model_name))
            .map_err(|e| Error::Embedding(e.to_string()))?;
        Ok(EmbeddingService { model: Arc::new(Mutex::new(model)) })
    }

    /// Embeds a single text string and returns the raw float vector.
    ///
    /// Offloads the blocking ONNX inference to a Tokio blocking thread.
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

    fn model_from_env() -> EmbeddingModel {
        match std::env::var("MEMLITE_EMBEDDING_MODEL").as_deref() {
            Ok("bge-small-en-v1.5") | Ok("BGESmallENV15") => EmbeddingModel::BGESmallENV15,
            Ok("bge-base-en-v1.5") | Ok("BGEBaseENV15") => EmbeddingModel::BGEBaseENV15,
            Ok("bge-large-en-v1.5") | Ok("BGELargeENV15") => EmbeddingModel::BGELargeENV15,
            _ => DEFAULT_MODEL,
        }
    }
}

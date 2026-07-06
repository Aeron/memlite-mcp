use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use tokio::sync::OnceCell;

use anyhow::{Result, anyhow};

/// Async-safe wrapper around a fastembed TextEmbedding model.
///
/// Loads the model lazily on the first embed call, so MCP server startup
/// completes without blocking on a potentially large model download.
/// `TextEmbedding::embed` takes `&mut self` and is CPU-bound, so calls are
/// dispatched via `spawn_blocking` to avoid stalling the Tokio reactor.
#[derive(Clone)]
pub struct EmbeddingService {
    model_kind: EmbeddingModel,
    cache_dir: PathBuf,
    cell: Arc<OnceCell<Arc<Mutex<TextEmbedding>>>>,
}

impl EmbeddingService {
    /// Creates a service configured to load `model` from `cache_dir` on first use.
    ///
    /// Does not download or initialize anything; the model loads on the first call to
    /// [`embed`] or [`embed_batch`].
    pub fn new(model: EmbeddingModel, cache_dir: PathBuf) -> Self {
        EmbeddingService {
            model_kind: model,
            cache_dir,
            cell: Arc::new(OnceCell::new()),
        }
    }

    /// Returns the initialized model, loading and caching it on the first call.
    async fn model(&self) -> Result<Arc<Mutex<TextEmbedding>>> {
        self.cell
            .get_or_try_init(|| {
                let model_kind = self.model_kind.clone();
                let cache_dir = self.cache_dir.clone();
                async move {
                    tokio::task::spawn_blocking(move || {
                        tracing::info!(
                            "loading embedding model {:?} (cache: {:?})",
                            model_kind,
                            cache_dir
                        );
                        let opts = TextInitOptions::new(model_kind).with_cache_dir(cache_dir);
                        TextEmbedding::try_new(opts)
                            .map(|m| Arc::new(Mutex::new(m)))
                            .map_err(|e| anyhow!("embedding: {e}"))
                    })
                    .await
                    .map_err(|e| anyhow!("embedding join: {e}"))?
                }
            })
            .await
            .map(Arc::clone)
    }

    /// Embeds a slice of strings in one ONNX inference pass, returning one vector per input.
    ///
    /// Offloads blocking ONNX inference to a Tokio blocking thread. Loads the model on first call.
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let expected = texts.len();
        let model = self.model().await?;
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().expect("embedding mutex poisoned");
            let embeddings = guard
                .embed(texts, None)
                .map_err(|e| anyhow!("embedding: {e}"))?;
            if embeddings.len() != expected {
                return Err(anyhow!(
                    "embedding returned {} vectors for {} inputs",
                    embeddings.len(),
                    expected
                ));
            }
            Ok(embeddings)
        })
        .await
        .map_err(|e| anyhow!("embedding join: {e}"))?
    }

    /// Embeds a single string and returns the raw float vector.
    ///
    /// Delegates to [`embed_batch`] with a one-element vec.
    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        self.embed_batch(vec![text])
            .await?
            .pop()
            .ok_or_else(|| anyhow!("embedding returned no vector"))
    }
}

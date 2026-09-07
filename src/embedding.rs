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
pub struct EmbeddingService {
    model_kind: EmbeddingModel,
    cache_dir: PathBuf,
    cell: OnceCell<Arc<Mutex<TextEmbedding>>>,
}

impl EmbeddingService {
    /// Creates a service configured to load `model` from `cache_dir` on first use.
    ///
    /// Does not download or initialize anything; the model loads on the first call to
    /// [`embed_owned`] or [`embed_batch_owned`].
    pub fn new(model: EmbeddingModel, cache_dir: PathBuf) -> Self {
        EmbeddingService {
            model_kind: model,
            cache_dir,
            cell: OnceCell::new(),
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
                        let opts = TextInitOptions::new(model_kind)
                            .with_cache_dir(cache_dir)
                            .with_show_download_progress(false);
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

    /// Embeds owned strings in one ONNX inference pass and returns them with their vectors.
    ///
    /// Keeping ownership in the blocking task lets callers retain stored content without
    /// cloning every string before inference.
    pub async fn embed_batch_owned(
        &self,
        texts: Vec<String>,
    ) -> Result<(Vec<String>, Vec<Vec<f32>>)> {
        let expected = texts.len();
        let model = self.model().await?;
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().expect("embedding mutex poisoned");
            let embeddings = guard
                .embed(&texts, None)
                .map_err(|e| anyhow!("embedding: {e}"))?;
            if embeddings.len() != expected {
                return Err(anyhow!(
                    "embedding returned {} vectors for {} inputs",
                    embeddings.len(),
                    expected
                ));
            }
            Ok((texts, embeddings))
        })
        .await
        .map_err(|e| anyhow!("embedding join: {e}"))?
    }

    /// Embeds one owned string and returns it with the raw float vector.
    pub async fn embed_owned(&self, text: String) -> Result<(String, Vec<f32>)> {
        let (mut texts, mut embeddings) = self.embed_batch_owned(vec![text]).await?;
        let text = texts
            .pop()
            .ok_or_else(|| anyhow!("embedding returned no input text"))?;
        let embedding = embeddings
            .pop()
            .ok_or_else(|| anyhow!("embedding returned no vector"))?;
        Ok((text, embedding))
    }
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel as FastEmbeddingModel, TextEmbedding, TextInitOptions};
use tokio::sync::OnceCell;

use anyhow::{Result, anyhow};

use crate::config::EmbeddingModel;

/// Async-safe wrapper around a fastembed TextEmbedding model.
///
/// Loads the model lazily on the first embed call, so MCP server startup
/// completes without blocking on a potentially large model download.
/// `TextEmbedding::embed` takes `&mut self` and is CPU-bound, so calls are
/// dispatched via `spawn_blocking` to avoid stalling the Tokio reactor.
pub struct EmbeddingService {
    model_kind: FastEmbeddingModel,
    cache_dir: PathBuf,
    batch_size: usize,
    cell: OnceCell<Arc<Mutex<TextEmbedding>>>,
}

impl EmbeddingService {
    /// Creates lazy service with model-aware batch cap.
    pub fn new(model: EmbeddingModel, cache_dir: PathBuf, max_batch_size: usize) -> Self {
        EmbeddingService {
            model_kind: model.into(),
            cache_dir,
            batch_size: model.max_batch_size().min(max_batch_size),
            cell: OnceCell::new(),
        }
    }

    /// Returns effective inference batch size.
    pub const fn batch_size(&self) -> usize {
        self.batch_size
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

    /// Embeds owned strings in one batched FastEmbed call and returns them with their vectors.
    ///
    /// Keeping ownership in the blocking task lets callers retain stored content without
    /// cloning every string before inference.
    pub async fn embed_batch_owned(
        &self,
        texts: Vec<String>,
    ) -> Result<(Vec<String>, Vec<Vec<f32>>)> {
        let expected = texts.len();
        let batch_size = self.batch_size;
        let model = self.model().await?;
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().expect("embedding mutex poisoned");
            let embeddings = guard
                .embed(&texts, Some(batch_size))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_batch_size_uses_model_tier() {
        let service = EmbeddingService::new(EmbeddingModel::M3, PathBuf::new(), 128);
        assert_eq!(service.batch_size(), 8);
    }

    #[test]
    fn effective_batch_size_uses_user_cap() {
        let service = EmbeddingService::new(EmbeddingModel::Small, PathBuf::new(), 16);
        assert_eq!(service.batch_size(), 16);
    }
}

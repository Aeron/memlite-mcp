use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database: {0}")]
    Database(#[from] libsql::Error),

    /// Wraps fastembed/ONNX errors as a string to avoid pulling anyhow into
    /// the public API surface.
    #[error("embedding: {0}")]
    Embedding(String),

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

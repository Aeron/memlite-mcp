use clap::Parser;
use rmcp::ServiceExt;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

mod config;
mod db;
mod embedding;
mod error;
mod server;

use config::Config;
use db::MemoryStore;
use embedding::EmbeddingService;
use server::MemServer;

// Replaces the default system allocator with mimalloc for better throughput
// when allocating the many small buffers produced by embedding inference.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Structured logging; defaults to INFO unless RUST_LOG is set.
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let config = Config::parse();
    info!(
        project = %config.project_path,
        agent   = %config.agent,
        in_mem  = config.in_memory,
        "starting memlite-mcp"
    );

    let db_path = config.db_path();
    info!(path = ?db_path, "opening database");
    let store = MemoryStore::open(db_path.as_deref()).await?;

    // Load the embedding model (may download on first run).
    let embedding = EmbeddingService::new(config.embedding_model.into())?;

    let server = MemServer::new(config, store, embedding);

    let service = server.serve(rmcp::transport::stdio()).await?;
    info!("memlite-mcp ready");
    service.waiting().await?;
    Ok(())
}

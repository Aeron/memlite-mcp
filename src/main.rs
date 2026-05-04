use clap::Parser;
use rmcp::ServiceExt;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

mod config;
mod db;
mod embedding;
mod mcp;

use config::Config;
use db::MemoryStore;
use embedding::EmbeddingService;
use mcp::MemServer;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Setting up structured logging; defaults to INFO unless RUST_LOG is set
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // Parsing the configuration
    let config = Config::parse();
    info!(
        project = %config.project_path,
        agent   = %config.agent,
        in_mem  = config.in_memory,
        "starting memlite-mcp"
    );

    // Resolving the database path and opening the store
    let db_path = config.db_path();
    info!(path = ?db_path, "opening database");
    let store = MemoryStore::open(db_path.as_deref()).await?;

    // Loading the embedding model (may download on first run)
    info!("loading embedding model");
    let embedding = EmbeddingService::new(config.embedding_model.into())?;

    // Creating the server and starting the service
    let server = MemServer::new(config, store, embedding);
    let service = server.serve(rmcp::transport::stdio()).await?;
    info!("memlite-mcp ready");

    service.waiting().await?;

    Ok(())
}

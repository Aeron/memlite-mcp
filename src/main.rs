use std::sync::{Arc, OnceLock};

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
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let config = Config::parse();
    info!(
        project = config.explicit_project_path().unwrap_or("<auto>"),
        agent   = %config.agent,
        in_mem  = config.in_memory,
        "starting memlite-mcp"
    );

    let store_lock: Arc<OnceLock<MemoryStore>> = Arc::new(OnceLock::new());

    // Open DB eagerly only when the project path was given explicitly;
    // otherwise on_initialized will call list_roots and open it then.
    if let Some(explicit) = config.explicit_project_path() {
        let project_path = Config::resolve_project_path(Some(explicit), &[]);
        let db_path = config.db_path_for(&project_path);
        info!(path = ?db_path, project = %project_path, "opening database eagerly");
        let store = MemoryStore::open(db_path.as_deref()).await?;
        let _ = store_lock.set(store);
    }

    info!("loading embedding model");
    let embedding =
        EmbeddingService::new(config.embedding_model.into(), config.embedding_cache_dir())?;

    let server = MemServer::new(config, store_lock, embedding);
    let service = server.serve(rmcp::transport::stdio()).await?;
    info!("memlite-mcp ready");

    service.waiting().await?;

    Ok(())
}

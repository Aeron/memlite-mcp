use std::{
    fs::OpenOptions,
    sync::{Arc, Mutex, OnceLock},
};

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
    let config = Config::parse();

    if let Some(log_path) = &config.log_file {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
            )
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .init();
    } else {
        fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }

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
        let project_path = Config::resolve_project_path(Some(explicit), &[])
            .expect("explicit project path must resolve");
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

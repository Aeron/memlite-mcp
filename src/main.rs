use clap::Parser;
use rmcp::ServiceExt;
use tracing::info;

mod config;
mod db;
mod embedding;
mod mcp;

use config::Config;

use embedding::EmbeddingService;
use mcp::MemServer;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

    config.init_logging()?;

    info!(
        project = config.explicit_project_path().unwrap_or("<auto>"),
        agent   = %config.agent,
        in_mem  = config.in_memory,
        "starting memlite-mcp"
    );

    let embedding =
        EmbeddingService::new(config.embedding_model.into(), config.embedding_cache_dir())?;

    let server = MemServer::new(config, embedding);
    let service = server.serve(rmcp::transport::stdio()).await?;
    info!("memlite-mcp ready");

    service.waiting().await?;

    Ok(())
}

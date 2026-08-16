use clap::Parser;
use tracing::info;

mod cli;
mod config;
mod db;
mod embedding;
mod errors;
mod mcp;
mod output;

use cli::{Cli, Command};
use embedding::EmbeddingService;
use mcp::MemServer;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Cli { config, command } = Cli::parse();
    config.init_logging(matches!(&command, Command::Serve))?;

    match command {
        Command::Serve => {
            info!(
                project = config.explicit_project_path().unwrap_or("<auto>"),
                "starting memlite-mcp"
            );
            let embedding =
                EmbeddingService::new(config.embedding_model.into(), config.embedding_cache_dir());
            MemServer::new(config, embedding).run().await
        }
        Command::Tool(args) => cli::run(&config, *args).await,
    }
}

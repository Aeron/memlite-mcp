use clap::Parser;
use tracing::info;

mod cli;
mod config;
mod db;
mod embedding;
mod mcp;
mod output;
mod worker;

use cli::{Cli, Command};
use mcp::ProxyServer;

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
            ProxyServer::new(config).run().await
        }
        Command::Tool(args) => cli::run(&config, *args).await,
    }
}

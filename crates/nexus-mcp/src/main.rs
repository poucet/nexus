//! Standalone `nexus-mcp` binary: serves the Nexus MCP tools over stdio or Streamable HTTP.

use clap::Parser;
use nexus_mcp::server::{serve_http, serve_stdio, DEFAULT_PORT};
use nexus_mcp::standalone::{create_standalone_registry, spawn_standalone_worker};

#[derive(Parser, Debug)]
#[command(name = "nexus-mcp")]
#[command(about = "Nexus Model Context Protocol (MCP) server for audio plugin sampling, control, and state exploration")]
struct Cli {
    /// Port for the streamable HTTP server
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Serve over stdio instead of streamable HTTP (for Claude Desktop / IDE integration)
    #[arg(long)]
    stdio: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    let (registry, rx) = create_standalone_registry();
    spawn_standalone_worker(rx);

    if cli.stdio {
        log::info!("nexus-mcp: serving MCP over stdio");
        serve_stdio(registry).await?;
    } else {
        serve_http(registry, cli.port).await?;
    }

    Ok(())
}

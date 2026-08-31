//! Background loopback MCP & REST servers for nexus-wrap.
//!
//! Exposes the inner plugin session to AI coding agents via:
//! - REST debug API on port 9996 (default)
//! - Model Context Protocol (MCP) on port 9997 (default)

use std::sync::{Arc, OnceLock};
use nexus_tools::ToolRegistry;

static SERVERS_STARTED: OnceLock<()> = OnceLock::new();

/// Start the loopback REST and MCP servers on a background worker thread.
/// Idempotent — multiple wrapper instances share the same loopback background servers.
pub fn start_servers(registry: Arc<ToolRegistry>) {
    SERVERS_STARTED.get_or_init(|| {
        let rest_port = std::env::var("NEXUS_REST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(nexus_rest::DEFAULT_PORT);

        let mcp_port = std::env::var("NEXUS_MCP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(nexus_mcp::DEFAULT_PORT);

        // 1. Start the REST frontend (axum on port 9996)
        nexus_rest::start_server(registry.clone(), rest_port);

        // 2. Start the MCP server (rmcp streamable HTTP on port 9997)
        let reg = registry.clone();
        std::thread::Builder::new()
            .name("nexus-wrap-mcp".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("nexus-wrap: failed to build tokio runtime for MCP: {e}");
                        return;
                    }
                };

                runtime.block_on(async move {
                    log::info!("nexus-wrap: starting loopback MCP server on port {mcp_port}");
                    if let Err(e) = nexus_mcp::serve_http(reg, mcp_port).await {
                        log::error!("nexus-wrap: MCP serve_http error: {e}");
                    }
                });
            })
            .expect("spawn nexus-wrap-mcp thread");
    });
}

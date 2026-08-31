//! MCP Server implementation for Nexus.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use nexus_tools::ToolRegistry;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

pub const DEFAULT_PORT: u16 = 9997;
pub const MCP_PATH: &str = "/mcp";
pub const INSTRUCTIONS: &str = include_str!("instructions.md");

#[derive(Clone)]
pub struct NexusMcpHandler {
    registry: Arc<ToolRegistry>,
}

impl NexusMcpHandler {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

impl ServerHandler for NexusMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "nexus-mcp".into(),
                version: "0.1.0".into(),
                icons: None,
                title: None,
                website_url: None,
            },
            instructions: Some(INSTRUCTIONS.into()),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools = Vec::new();
        for t in self.registry.tools() {
            let schema_val = serde_json::to_value(&t.input_schema).unwrap_or(Value::Object(Default::default()));
            let schema_obj = match schema_val {
                Value::Object(map) => map,
                _ => Default::default(),
            };
            tools.push(Tool {
                name: t.name.to_string().into(),
                description: Some(t.description.to_string().into()),
                input_schema: Arc::new(schema_obj),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            });
        }
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let Some(tool) = self.registry.get(&request.name) else {
            return Err(McpError::invalid_params(format!("unknown tool: {}", request.name), None));
        };

        let input_val = request
            .arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(Default::default()));

        match tool.invoke(input_val).await {
            Ok(val) => Ok(CallToolResult {
                content: vec![Content::text(
                    serde_json::to_string_pretty(&val).unwrap_or_default(),
                )],
                is_error: None,
                meta: None,
                structured_content: None,
            }),
            Err(e) => Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Error {:?}: {}",
                    e.code,
                    e.message
                ))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
        }
    }
}

pub fn app_with(registry: Arc<ToolRegistry>) -> Router {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig {
        stateful_mode: true,
        sse_keep_alive: Some(Duration::from_secs(30)),
    };

    let reg = registry.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(NexusMcpHandler::new(reg.clone())),
        session_manager,
        config,
    );

    Router::new().route(
        "/mcp",
        axum::routing::any(move |req: axum::http::Request<axum::body::Body>| {
            let service = mcp_service.clone();
            async move { service.handle(req).await }
        }),
    )
}

pub async fn serve_http(registry: Arc<ToolRegistry>, port: u16) -> std::io::Result<()> {
    let v4_addr = SocketAddr::from(([127, 0, 0, 1], port));
    let v6_addr = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));

    let app = app_with(registry);

    let v4 = tokio::net::TcpListener::bind(v4_addr).await;
    let v6 = tokio::net::TcpListener::bind(v6_addr).await;

    match (v4, v6) {
        (Ok(v4), Ok(v6)) => {
            log::info!("nexus-mcp: serving MCP at http://{v4_addr}{MCP_PATH} and [::1]:{port}{MCP_PATH}");
            let _ = tokio::join!(
                axum::serve(v4, app.clone()),
                axum::serve(v6, app)
            );
        }
        (Ok(v4), Err(e)) => {
            log::warn!("nexus-mcp: [::1]:{port} bind failed ({e}); serving IPv4 only");
            log::info!("nexus-mcp: serving MCP at http://{v4_addr}{MCP_PATH}");
            let _ = axum::serve(v4, app).await;
        }
        (Err(e4), Ok(v6)) => {
            log::warn!("nexus-mcp: 127.0.0.1:{port} bind failed ({e4}); serving IPv6 only");
            let _ = axum::serve(v6, app).await;
        }
        (Err(e4), Err(e6)) => {
            log::error!("nexus-mcp: could not bind port {port} (v4: {e4}, v6: {e6})");
        }
    }

    Ok(())
}

pub async fn serve_stdio(registry: Arc<ToolRegistry>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let handler = NexusMcpHandler::new(registry);
    let transport = rmcp::transport::io::stdio();
    let running = handler.serve(transport).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_tools::bridge::{HostBridge, HostSession, MainThreadHost};
    use nexus_tools::types::*;
    use nexus_tools::{ToolError, ToolsService};

    struct TestHost;

    impl MainThreadHost for TestHost {
        fn session(&mut self) -> HostSession {
            HostSession {
                loaded: Some(LoadedPlugin {
                    plugin_id: "test.synth".into(),
                    name: Some("Test Synth".into()),
                    vendor: Some("Test Vendor".into()),
                    format: "clap".into(),
                }),
                sample_rate: Some(48000.0),
            }
        }
        fn descriptor(&mut self) -> Option<nexus_host::host::PluginDescriptor> {
            None
        }
        fn save_state(&mut self) -> Result<Vec<u8>, ToolError> {
            Ok(vec![1, 2, 3])
        }
        fn load_state(&mut self, _data: &[u8]) -> Result<(), ToolError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn server_handler_info_and_tools() {
        let (bridge, rx) = HostBridge::channel(Arc::new(|| {}));
        let service = Arc::new(ToolsService::new(bridge));
        let registry = Arc::new(ToolRegistry::standard(service));
        let handler = NexusMcpHandler::new(registry.clone());

        // Spawn mock host worker
        std::thread::spawn(move || {
            let mut host = TestHost;
            while let Ok(req) = rx.recv() {
                nexus_tools::bridge::handle(req, &mut host);
            }
        });

        let info = handler.get_info();
        assert_eq!(info.server_info.name, "nexus-mcp");
        assert!(info.instructions.is_some());

        let (_client_io, server_io) = tokio::io::duplex(4096);
        let running = rmcp::service::serve_directly(handler.clone(), server_io, None);
        let ctx = RequestContext {
            ct: Default::default(),
            id: RequestId::Number(1),
            meta: Default::default(),
            extensions: Default::default(),
            peer: running.peer().clone(),
        };

        let tools = handler.list_tools(None, ctx.clone()).await.unwrap();
        assert_eq!(tools.tools.len(), registry.tools().len());
        assert!(tools.tools.iter().any(|t| t.name == "get_session"));
        assert!(tools.tools.iter().any(|t| t.name == "render_audio"));
        assert!(tools.tools.iter().any(|t| t.name == "play"));
        assert!(tools.tools.iter().any(|t| t.name == "list_plugins"));

        let res = handler
            .call_tool(
                CallToolRequestParam {
                    name: "get_session".into(),
                    arguments: Some(Default::default()),
                },
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(res.is_error, None);
        assert!(!res.content.is_empty());
    }
}

//! Nexus MCP Server: exposes Nexus plugin hosting, sampling, and state tools
//! over the Model Context Protocol (MCP) to AI coding agents.

pub mod server;
pub mod standalone;

pub use server::{NexusMcpHandler, serve_http, serve_stdio, DEFAULT_PORT};
pub use standalone::{create_standalone_registry, spawn_standalone_worker, StandaloneHost};

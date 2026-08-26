//! nexus-tools: the transport-agnostic tool surface for structured access to a
//! hosted plugin's stored state (design: docs/designs/STATE_TOOLS.md).
//!
//! - [`NexusTools`] — the async trait every frontend (REST, later MCP) consumes.
//! - [`ToolsService`] — its canonical implementation: main-thread bridge +
//!   codec registry + snapshot store.
//! - [`ToolRegistry`] — each tool declared once (name/description/schemas/
//!   handler); frontends stay thin.
//! - [`codec::StateCodec`] — the per-plugin extensibility seam for making
//!   opaque state blobs legible (and, once understood, editable).
//!
//! This crate has **no async runtime** — frontends bring their own and enforce
//! per-tool timeouts (see [`registry::TimeoutClass`]).

pub mod bridge;
pub mod codec;
pub mod codecs;
pub mod diff;
pub mod error;
pub mod registry;
pub mod service;
pub mod snapshots;
pub mod tools;
pub mod types;

pub use bridge::{HostBridge, HostRequest, HostSession, MainThreadHost, drain};
pub use error::{ErrorCode, ToolError};
pub use registry::{TimeoutClass, ToolDef, ToolRegistry};
pub use service::ToolsService;
pub use tools::NexusTools;

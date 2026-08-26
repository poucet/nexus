//! The single error type every tool returns.
//!
//! Codes are stable identifiers a client (human debug UI, LLM) can branch on;
//! messages are written to be actionable ("call X first"), not just descriptive.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No plugin is loaded in the session.
    NoPlugin,
    /// The request addressed an instance other than "default" (v1 limitation).
    InvalidInstance,
    /// A named thing (snapshot, path, …) does not exist.
    NotFound,
    /// The operation lost a race (e.g. state changed since decode).
    Conflict,
    /// The operation isn't available here (e.g. codec is decode-only).
    Unsupported,
    /// The request itself was malformed.
    InvalidInput,
    /// The plugin rejected or failed the operation.
    Plugin,
    /// Internal plumbing failure (bridge closed, host gone).
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn no_plugin() -> Self {
        Self::new(
            ErrorCode::NoPlugin,
            "no plugin loaded — load one via the app's Plugin menu (load_plugin tool arrives later)",
        )
    }

    pub fn invalid_instance(got: &str) -> Self {
        Self::new(
            ErrorCode::InvalidInstance,
            format!("unknown instance {got:?} — only \"default\" exists in this version"),
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn plugin(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Plugin, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

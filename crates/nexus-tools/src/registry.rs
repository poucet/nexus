//! The tool registry: every tool declared once — name, description, schemas,
//! handler — and every frontend (REST now, MCP later) a thin consumer.
//!
//! Deliberately hand-written (no proc-macro): with a dozen tools and one
//! frontend the entries are cheaper than machinery. Extract a macro when MCP
//! becomes the second consumer and the boilerplate is real.

use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;

use crate::error::ToolError;
use crate::tools::NexusTools;
use crate::types::*;

/// How long a frontend should give this tool. This crate has no runtime, so
/// enforcing the timeout is the frontend's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutClass {
    /// Ordinary calls: ~5 s.
    Default,
    /// Loads plugin state or binaries; sample libraries can take a while: ~30 s.
    Load,
}

type Handler = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value, ToolError>> + Send + Sync>;

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub timeout_class: TimeoutClass,
    pub input_schema: schemars::Schema,
    pub output_schema: schemars::Schema,
    handler: Handler,
}

impl ToolDef {
    pub async fn invoke(&self, input: Value) -> Result<Value, ToolError> {
        (self.handler)(input).await
    }
}

pub struct ToolRegistry {
    tools: Vec<ToolDef>,
}

macro_rules! tool {
    ($service:expr, $name:literal, $class:expr, $req:ty => $out:ty, $method:ident, $desc:literal) => {{
        let service = $service.clone();
        ToolDef {
            name: $name,
            description: $desc,
            timeout_class: $class,
            input_schema: schemars::schema_for!($req),
            output_schema: schemars::schema_for!($out),
            handler: Arc::new(move |input: Value| {
                let service = service.clone();
                Box::pin(async move {
                    let req: $req = serde_json::from_value(input)
                        .map_err(|e| ToolError::invalid_input(format!("invalid arguments: {e}")))?;
                    let out = service.$method(req).await?;
                    serde_json::to_value(out)
                        .map_err(|e| ToolError::internal(format!("serialize response: {e}")))
                }) as BoxFuture<'static, Result<Value, ToolError>>
            }),
        }
    }};
}

impl ToolRegistry {
    /// The v1 tool set over any [`NexusTools`] implementation.
    pub fn standard(service: Arc<dyn NexusTools>) -> Self {
        let tools = vec![
            tool!(service, "get_session", TimeoutClass::Default, GetSessionRequest => SessionInfo, get_session,
                "What's loaded right now: plugin, sample rate, and the on-disk corpus directory. Call this first."),
            tool!(service, "decode_state", TimeoutClass::Load, DecodeStateRequest => DecodedState, decode_state,
                "Decode the plugin's stored state (live, or a snapshot) into a structured document via the best codec. Use `path` (JSON pointer) + `depth` to fetch just a subtree; the response's `base_sha256` feeds edit_state."),
            tool!(service, "edit_state", TimeoutClass::Load, EditStateRequest => EditResult, edit_state,
                "THE write path: targeted edits (set/remove at JSON-pointer paths) applied to the decoded state and re-encoded. Requires `base_sha256` from the decode the edits were computed against; `apply:false` dry-runs. Applying auto-snapshots the prior state and verifies through the plugin."),
            tool!(service, "get_state", TimeoutClass::Load, GetStateRequest => StateBlobInfo, get_state,
                "The raw state blob's metadata (size, sha256), optionally with the base64 data under a size cap. Prefer decode_state for anything human/LLM-facing."),
            tool!(service, "set_state", TimeoutClass::Load, SetStateRequest => StateBlobInfo, set_state,
                "Load a full state blob (base64) into the plugin. Escape hatch — prefer edit_state. Auto-snapshots the prior state first."),
            tool!(service, "snapshot_state", TimeoutClass::Load, SnapshotStateRequest => SnapshotInfo, snapshot_state,
                "Capture the live state as a named snapshot (auto-named when omitted); `export:true` also writes it to the on-disk state corpus."),
            tool!(service, "restore_snapshot", TimeoutClass::Load, RestoreSnapshotRequest => RestoreResult, restore_snapshot,
                "Load a snapshot back into the plugin (undo). The pre-restore state is auto-snapshotted."),
            tool!(service, "list_snapshots", TimeoutClass::Default, ListSnapshotsRequest => SnapshotList, list_snapshots,
                "All snapshots in this session, oldest first."),
            tool!(service, "diff_state", TimeoutClass::Load, DiffStateRequest => StateDiff, diff_state,
                "Structural diff between snapshot `a` and `b` (omit `b` = the live state): the 'what did that knob change?' call. Changed paths with before/after values, at the decoded-document level."),
            tool!(service, "list_codecs", TimeoutClass::Default, ListCodecsRequest => CodecList, list_codecs,
                "The registered state codecs and whether each can re-encode (edit_state support)."),
        ];
        Self { tools }
    }

    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// JSON descriptor list for `GET /tools` (and later MCP `list_tools`).
    pub fn descriptors(&self) -> Value {
        Value::Array(
            self.tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                        "output_schema": t.output_schema,
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use async_trait::async_trait;
    use futures::executor::block_on;
    use serde_json::json;

    /// Minimal NexusTools stub — the registry test only needs dispatch.
    struct Stub;

    #[async_trait]
    impl NexusTools for Stub {
        async fn get_session(&self, _req: GetSessionRequest) -> Result<SessionInfo, ToolError> {
            Ok(SessionInfo { loaded: None, sample_rate: None, corpus_directory: "/tmp".into() })
        }
        async fn decode_state(&self, _req: DecodeStateRequest) -> Result<DecodedState, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn edit_state(&self, _req: EditStateRequest) -> Result<EditResult, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn get_state(&self, _req: GetStateRequest) -> Result<StateBlobInfo, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn set_state(&self, _req: SetStateRequest) -> Result<StateBlobInfo, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn snapshot_state(&self, _req: SnapshotStateRequest) -> Result<SnapshotInfo, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn restore_snapshot(&self, _req: RestoreSnapshotRequest) -> Result<RestoreResult, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn list_snapshots(&self, _req: ListSnapshotsRequest) -> Result<SnapshotList, ToolError> {
            Ok(SnapshotList { snapshots: vec![] })
        }
        async fn diff_state(&self, _req: DiffStateRequest) -> Result<StateDiff, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn list_codecs(&self, _req: ListCodecsRequest) -> Result<CodecList, ToolError> {
            Ok(CodecList { codecs: vec![] })
        }
    }

    #[test]
    fn dispatches_by_name_with_defaults() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let out = block_on(registry.get("get_session").unwrap().invoke(json!({}))).unwrap();
        assert_eq!(out["corpus_directory"], "/tmp");
        assert!(registry.get("no_such_tool").is_none());
    }

    #[test]
    fn invalid_arguments_are_invalid_input() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let err = block_on(
            registry.get("diff_state").unwrap().invoke(json!({"a": 42})), // wrong type
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn tool_errors_pass_through() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let err = block_on(registry.get("get_state").unwrap().invoke(json!({}))).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoPlugin);
    }

    #[test]
    fn descriptors_cover_every_tool_with_schemas() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let descriptors = registry.descriptors();
        let list = descriptors.as_array().unwrap();
        assert_eq!(list.len(), registry.tools().len());
        for d in list {
            assert!(d["name"].is_string());
            assert!(!d["description"].as_str().unwrap().is_empty());
            assert!(d["input_schema"].is_object());
            assert!(d["output_schema"].is_object());
        }
    }
}

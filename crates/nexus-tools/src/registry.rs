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
            tool!(service, "list_plugins", TimeoutClass::Default, ListPluginsRequest => ListPluginsResponse, list_plugins,
                "List installed CLAP and VST3 plugins found on the system catalog, optionally filtered by query."),
            tool!(service, "load_plugin", TimeoutClass::Load, LoadPluginRequest => LoadPluginResponse, load_plugin,
                "Load a plugin by ID or fuzzy name into the session."),
            tool!(service, "list_presets", TimeoutClass::Default, ListPresetsRequest => ListPresetsResponse, list_presets,
                "List available .nexuspreset files, optionally filtered by plugin ID."),
            tool!(service, "load_preset", TimeoutClass::Load, LoadPresetRequest => LoadPresetResponse, load_preset,
                "Load a preset file into the active plugin instance."),
            tool!(service, "save_preset", TimeoutClass::Load, SavePresetRequest => SavePresetResponse, save_preset,
                "Save the current plugin state and metadata as a .nexuspreset file."),
            tool!(service, "list_params", TimeoutClass::Default, ListParamsRequest => ListParamsResponse, list_params,
                "List and search parameters exposed by the loaded plugin, with pagination and filtering."),
            tool!(service, "get_params", TimeoutClass::Default, GetParamsRequest => GetParamsResponse, get_params,
                "Get current values and display strings for a list of parameter IDs."),
            tool!(service, "set_params", TimeoutClass::Default, SetParamsRequest => SetParamsResponse, set_params,
                "Set values for one or more plugin parameters."),
            tool!(service, "convert_param_value", TimeoutClass::Default, ConvertParamValueRequest => ConvertParamValueResponse, convert_param_value,
                "Convert a parameter value to display text, or parse display text into a plain value."),
            tool!(service, "play", TimeoutClass::Default, PlayRequest => PlayResponse, play,
                "Queue a musical performance (Droplets Fugue format: notes, CCs, automation ramps) for live transport-synced playback."),
            tool!(service, "render_audio", TimeoutClass::Load, RenderAudioRequest => RenderAudioResponse, render_audio,
                "Render a fugue offline through the loaded plugin at max CPU speed, without an audio driver. \
                 Returns audio metrics and 32-bit float WAV. \
                 LENGTH: the fugue's own duration_beats at tempo_bpm (default 120), plus a fixed 1.0 s release tail \
                 so a note ending on the last beat keeps its decay; frames and duration_s include the tail. \
                 LOOP: `once` renders one pass; {\"times\": n} renders n passes with every event re-emitted a pass later, \
                 and the tail is added once, after the last; `loop` and `ping_pong` are endless, have no length, and are \
                 refused here — they apply to live playback via `play`. \
                 OUTPUT: output_format is \"file\" (the DEFAULT — returns file_path, written to output_path if given) \
                 or \"wav_base64\" (inline; a minute of stereo is ~30 MB). Any other value is rejected. \
                 CONTROL SPACE: a `ccs` lane is MIDI bytes 0-127, and that IS the normalized (travel) space the plugin's \
                 own knob moves in, so a CC sweep sounds exactly like turning that knob — on a log-tapered cutoff, linear \
                 in octaves. A `params` lane interpolates in the plugin's plain value, which is affine in its normalized \
                 range, so linear-in-plain is linear-in-travel too. Curves: none (stepped, the default) / linear / exp \
                 (t^2) / log (1-(1-t)^2), sampled every 64 frames for every lane kind. \
                 WARNINGS: the response's `warnings` names everything decided on your behalf — an event kind this plugin \
                 path cannot carry (with a count), a floored block_size, an assumed tempo. \
                 Flux exposes render_audio taking the SAME argument, but renders through one patch rather than a hosted \
                 plugin: comparable audio, not equal. Flux reports errors as prose; nexus returns {code, message}."),
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
        async fn list_plugins(&self, _req: ListPluginsRequest) -> Result<ListPluginsResponse, ToolError> {
            Ok(ListPluginsResponse { plugins: vec![] })
        }
        async fn load_plugin(&self, _req: LoadPluginRequest) -> Result<LoadPluginResponse, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn list_presets(&self, _req: ListPresetsRequest) -> Result<ListPresetsResponse, ToolError> {
            Ok(ListPresetsResponse { presets: vec![] })
        }
        async fn load_preset(&self, _req: LoadPresetRequest) -> Result<LoadPresetResponse, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn save_preset(&self, _req: SavePresetRequest) -> Result<SavePresetResponse, ToolError> {
            Err(ToolError::no_plugin())
        }
        async fn list_params(&self, _req: ListParamsRequest) -> Result<ListParamsResponse, ToolError> {
            Ok(ListParamsResponse { total: 0, offset: 0, params: vec![] })
        }
        async fn get_params(&self, _req: GetParamsRequest) -> Result<GetParamsResponse, ToolError> {
            Ok(GetParamsResponse { params: vec![] })
        }
        async fn set_params(&self, _req: SetParamsRequest) -> Result<SetParamsResponse, ToolError> {
            Ok(SetParamsResponse { updated: vec![] })
        }
        async fn convert_param_value(&self, req: ConvertParamValueRequest) -> Result<ConvertParamValueResponse, ToolError> {
            Ok(ConvertParamValueResponse { id: req.id, value: req.value, display_text: req.text })
        }
        async fn play(&self, _req: PlayRequest) -> Result<PlayResponse, ToolError> {
            Ok(PlayResponse { queued: true, duration_beats: 4.0, total_events: 0 })
        }
        async fn render_audio(&self, _req: RenderAudioRequest) -> Result<RenderAudioResponse, ToolError> {
            Err(ToolError::no_plugin())
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

    /// **A typo'd `output_format` is refused, and the message names both
    /// values.** It used to fall silently through to base64 and hand back tens
    /// of megabytes of inline audio the caller never asked for — the one place
    /// FUGUE-API's compatibility rule is knowingly broken (§3.5, ruled §6.6).
    #[test]
    fn an_unknown_output_format_is_rejected_naming_both_values() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let err = block_on(
            registry
                .get("render_audio")
                .unwrap()
                .invoke(json!({"fugue": {}, "output_format": "wav"})),
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(err.message.contains("file"), "{}", err.message);
        assert!(err.message.contains("wav_base64"), "{}", err.message);

        // Both real spellings still reach the service (which has no plugin).
        for spelling in ["file", "wav_base64"] {
            let err = block_on(
                registry
                    .get("render_audio")
                    .unwrap()
                    .invoke(json!({"fugue": {}, "output_format": spelling})),
            )
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::NoPlugin, "{spelling} must still parse");
        }
    }

    /// The tool description is the only place an agent learns the policies that
    /// have no field: the tail, the loop refusal, the CC value space.
    #[test]
    fn the_render_audio_description_states_the_policies_with_no_field() {
        let registry = ToolRegistry::standard(Arc::new(Stub));
        let description = registry.get("render_audio").unwrap().description;
        for claim in ["1.0 s release tail", "ping_pong", "\"file\" (the DEFAULT", "travel"] {
            assert!(description.contains(claim), "tool description must say {claim:?}");
        }
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

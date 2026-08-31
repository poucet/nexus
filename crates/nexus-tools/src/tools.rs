//! The transport-agnostic tool surface (v1: the state-centric core).
//!
//! Frontends (REST now, MCP later, gRPC if wanted) are thin consumers of this
//! trait via the [`crate::registry::ToolRegistry`] — none of them defines the
//! contract. See docs/design/STATE_TOOLS.md for what is deliberately NOT here yet
//! (params, presets, catalog): those land once manual investigation through
//! the debug UI demands them.

use async_trait::async_trait;

use crate::error::ToolError;
use crate::types::*;

#[async_trait]
pub trait NexusTools: Send + Sync {
    async fn get_session(&self, req: GetSessionRequest) -> Result<SessionInfo, ToolError>;

    /// Path-scoped read of the decoded state: `path` + `depth` return just a
    /// subtree — never the whole multi-MB document by accident.
    async fn decode_state(&self, req: DecodeStateRequest) -> Result<DecodedState, ToolError>;

    /// Path-scoped write — the PRIMARY write path. Guarded by `base_sha256`
    /// and verified through the plugin (load → re-save → re-decode).
    async fn edit_state(&self, req: EditStateRequest) -> Result<EditResult, ToolError>;

    async fn get_state(&self, req: GetStateRequest) -> Result<StateBlobInfo, ToolError>;

    /// Full-blob write. Escape hatch / restore only — prefer `edit_state`.
    async fn set_state(&self, req: SetStateRequest) -> Result<StateBlobInfo, ToolError>;

    async fn snapshot_state(&self, req: SnapshotStateRequest) -> Result<SnapshotInfo, ToolError>;
    async fn restore_snapshot(&self, req: RestoreSnapshotRequest) -> Result<RestoreResult, ToolError>;
    async fn list_snapshots(&self, req: ListSnapshotsRequest) -> Result<SnapshotList, ToolError>;

    /// Doc-level structural diff; `b` omitted = live state ("what did that
    /// knob change?").
    async fn diff_state(&self, req: DiffStateRequest) -> Result<StateDiff, ToolError>;

    async fn list_codecs(&self, req: ListCodecsRequest) -> Result<CodecList, ToolError>;

    // -----------------------------------------------------------------------
    // Catalog & Presets

    async fn list_plugins(&self, req: ListPluginsRequest) -> Result<ListPluginsResponse, ToolError>;
    async fn load_plugin(&self, req: LoadPluginRequest) -> Result<LoadPluginResponse, ToolError>;

    async fn list_presets(&self, req: ListPresetsRequest) -> Result<ListPresetsResponse, ToolError>;
    async fn load_preset(&self, req: LoadPresetRequest) -> Result<LoadPresetResponse, ToolError>;
    async fn save_preset(&self, req: SavePresetRequest) -> Result<SavePresetResponse, ToolError>;

    // -----------------------------------------------------------------------
    // Parameters

    async fn list_params(&self, req: ListParamsRequest) -> Result<ListParamsResponse, ToolError>;
    async fn get_params(&self, req: GetParamsRequest) -> Result<GetParamsResponse, ToolError>;
    async fn set_params(&self, req: SetParamsRequest) -> Result<SetParamsResponse, ToolError>;

    // -----------------------------------------------------------------------
    // Performance & Sampling

    /// Queue a transport-synchronized musical performance into the host (Droplets fugue model).
    async fn play(&self, req: PlayRequest) -> Result<PlayResponse, ToolError>;

    /// Render audio offline through the plugin at max CPU speed without an audio driver.
    async fn render_audio(&self, req: RenderAudioRequest) -> Result<RenderAudioResponse, ToolError>;
}

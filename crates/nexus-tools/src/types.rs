//! Request/response DTOs for the tool surface.
//!
//! Conventions (see designs/STATE_TOOLS.md): every request carries
//! `instance: "default"` so a multi-instance future doesn't break the contract;
//! every response is an object wrapper (never a bare array); values are JSON.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn default_instance() -> String {
    "default".into()
}

fn is_default_instance(s: &str) -> bool {
    s == "default"
}

// ---------------------------------------------------------------------------
// Session

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSessionRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
}

impl Default for GetSessionRequest {
    fn default() -> Self {
        Self { instance: default_instance() }
    }
}

/// The plugin currently loaded in the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LoadedPlugin {
    pub plugin_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// "clap" or "vst3".
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionInfo {
    /// `null` when no plugin is loaded.
    pub loaded: Option<LoadedPlugin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<f64>,
    /// Where exported snapshots (the state corpus) land on disk.
    pub corpus_directory: String,
}

// ---------------------------------------------------------------------------
// State blobs

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// Include the blob itself (base64) when it fits under `max_bytes`.
    #[serde(default)]
    pub include_data: bool,
    /// Cap on returned blob size; larger states return metadata only.
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
}

fn default_max_bytes() -> u64 {
    262_144
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateBlobInfo {
    pub plugin_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    /// Why `base64` is absent when data was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_omitted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// The full state blob. Escape hatch — prefer `edit_state` for targeted changes.
    pub base64: String,
}

// ---------------------------------------------------------------------------
// Snapshots

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// Auto-generated ("snap-N") when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Also write the blob to the on-disk state corpus.
    #[serde(default)]
    pub export: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotInfo {
    pub name: String,
    pub plugin_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_unix: u64,
    /// Set when the snapshot was exported to the corpus directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exported_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSnapshotRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreResult {
    /// The snapshot that was loaded into the plugin.
    pub restored: SnapshotInfo,
    /// Automatic snapshot of the state as it was just before the restore.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSnapshotsRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
}

impl Default for ListSnapshotsRequest {
    fn default() -> Self {
        Self { instance: default_instance() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotList {
    pub snapshots: Vec<SnapshotInfo>,
}

// ---------------------------------------------------------------------------
// Decode / edit

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecodeStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// Decode this snapshot instead of the live state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// JSON pointer into the decoded document; returns just that subtree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Depth to which the returned subtree is expanded; deeper levels are
    /// summarized. Fetch a deeper `path` to drill in.
    #[serde(default = "default_depth")]
    pub depth: u32,
}

fn default_depth() -> u32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DecodedState {
    /// Codec that produced the document.
    pub codec: String,
    /// The codec's confidence for this (plugin, blob): 1 = generic fallback,
    /// 255 = authored for exactly this plugin.
    pub confidence: u8,
    /// Whether this codec can re-encode edits (`edit_state` works).
    pub encode_capable: bool,
    /// sha256 of the blob this document was decoded from — pass to
    /// `edit_state` as `base_sha256`.
    pub base_sha256: String,
    pub size_bytes: u64,
    /// The JSON pointer this subtree was fetched at ("" = root).
    pub path: String,
    /// The decoded (sub)document; levels below `depth` appear summarized.
    pub doc: Value,
}

/// One targeted edit at a JSON-pointer path in the decoded document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Set the value at `path` (creates a missing final object key).
    Set { value: Value },
    /// Remove the value at `path`.
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateEdit {
    /// JSON pointer into the decoded document, e.g. "/settings/osc_1_level".
    pub path: String,
    #[serde(flatten)]
    pub op: EditOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// The `base_sha256` from the `decode_state` these edits were computed
    /// against. Guards against the state changing underneath (e.g. a knob
    /// turned in the plugin's editor since the decode).
    pub base_sha256: String,
    pub edits: Vec<StateEdit>,
    /// false = dry-run: report what would happen without touching the plugin.
    #[serde(default)]
    pub apply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VerifyMismatch {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Verification {
    /// The plugin accepted the blob and every edited path reads back as edited.
    pub verified: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mismatches: Vec<VerifyMismatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditResult {
    /// Whether the edited state was loaded into the plugin (false = dry-run).
    pub applied: bool,
    /// sha256 of the resulting blob. After an apply this is the plugin's own
    /// re-saved state (plugins may normalize), after a dry-run the encoded blob.
    pub sha256: String,
    pub size_bytes: u64,
    /// Snapshot of the pre-edit state, taken automatically before an apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_snapshot: Option<String>,
    /// Through-the-plugin verification (apply only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
}

// ---------------------------------------------------------------------------
// Diff

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffStateRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
    /// Baseline snapshot name.
    pub a: String,
    /// Other snapshot name; omitted = the live state ("what did that knob change?").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffChange {
    pub path: String,
    pub kind: DiffKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateDiff {
    /// Codec whose documents were compared.
    pub codec: String,
    pub changes: Vec<DiffChange>,
    /// True when the change list was cut off at the cap.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Codecs

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListCodecsRequest {
    #[serde(default = "default_instance", skip_serializing_if = "is_default_instance")]
    pub instance: String,
}

impl Default for ListCodecsRequest {
    fn default() -> Self {
        Self { instance: default_instance() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodecInfo {
    pub id: String,
    pub description: String,
    pub encode_capable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodecList {
    pub codecs: Vec<CodecInfo>,
}

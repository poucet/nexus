//! `ToolsService`: the canonical [`NexusTools`] implementation, composing the
//! main-thread bridge (plugin access), the codec registry (state legibility),
//! and the snapshot store (investigation working set + undo).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use nexus_host::host::PluginDescriptor;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::bridge::HostBridge;
use crate::codec::{CodecRegistry, StateCodec, apply_edits, prune};
use crate::diff::diff_docs;
use crate::error::ToolError;
use crate::snapshots::{SnapshotStore, corpus_directory};
use crate::tools::NexusTools;
use crate::types::*;

pub struct ToolsService {
    bridge: HostBridge,
    snapshots: Arc<SnapshotStore>,
    codecs: Arc<CodecRegistry>,
}

impl ToolsService {
    pub fn new(bridge: HostBridge) -> Self {
        Self {
            bridge,
            snapshots: Arc::new(SnapshotStore::new()),
            codecs: Arc::new(CodecRegistry::standard()),
        }
    }

    fn check_instance(instance: &str) -> Result<(), ToolError> {
        if instance == "default" { Ok(()) } else { Err(ToolError::invalid_instance(instance)) }
    }

    /// Current plugin id, for snapshot bookkeeping and codec selection.
    async fn plugin_id(&self) -> Result<String, ToolError> {
        match self.bridge.session().await?.loaded {
            Some(loaded) => Ok(loaded.plugin_id),
            None => Err(ToolError::no_plugin()),
        }
    }

    /// The blob a state-reading tool operates on: a snapshot, or the live state.
    async fn source_blob(&self, snapshot: Option<&str>) -> Result<(Arc<Vec<u8>>, String), ToolError> {
        match snapshot {
            Some(name) => {
                let snapshot = self.snapshots.get(name)?;
                Ok((snapshot.data, snapshot.info.plugin_id))
            }
            None => {
                let blob = self.bridge.save_state().await?;
                let plugin_id = self.plugin_id().await?;
                Ok((Arc::new(blob), plugin_id))
            }
        }
    }

    /// Descriptor for codec selection: the live plugin's, or a synthetic one
    /// carrying the snapshot's plugin id when nothing (matching) is loaded.
    async fn descriptor_for(&self, plugin_id: &str) -> Result<Option<PluginDescriptor>, ToolError> {
        let live = self.bridge.descriptor().await?;
        Ok(match live {
            Some(d) if d.id == plugin_id => Some(d),
            _ => Some(PluginDescriptor {
                id: plugin_id.to_string(),
                name: None,
                vendor: None,
                version: None,
            }),
        })
    }

    async fn auto_snapshot(&self, note: &str) -> Result<String, ToolError> {
        let blob = self.bridge.save_state().await?;
        let plugin_id = self.plugin_id().await?;
        let sha = sha256_hex(&blob);
        let info =
            self.snapshots.insert(None, Some(note.to_string()), plugin_id, Arc::new(blob), sha);
        Ok(info.name)
    }

    fn select_codec(
        &self,
        descriptor: Option<&PluginDescriptor>,
        blob: &[u8],
    ) -> Result<(Arc<dyn StateCodec>, u8), ToolError> {
        self.codecs.select(descriptor, blob)
    }
}

#[async_trait]
impl NexusTools for ToolsService {
    async fn get_session(&self, req: GetSessionRequest) -> Result<SessionInfo, ToolError> {
        Self::check_instance(&req.instance)?;
        let host = self.bridge.session().await?;
        Ok(SessionInfo::from_host(host, corpus_directory().display().to_string()))
    }

    async fn decode_state(&self, req: DecodeStateRequest) -> Result<DecodedState, ToolError> {
        Self::check_instance(&req.instance)?;
        let (blob, plugin_id) = self.source_blob(req.snapshot.as_deref()).await?;
        let descriptor = self.descriptor_for(&plugin_id).await?;
        let (codec, confidence) = self.select_codec(descriptor.as_ref(), &blob)?;
        let doc = codec.decode(descriptor.as_ref(), &blob)?;
        let path = req.path.unwrap_or_default();
        let subtree = if path.is_empty() {
            &doc
        } else {
            doc.pointer(&path).ok_or_else(|| {
                ToolError::not_found(format!(
                    "path {path:?} does not exist in the decoded document (codec {})",
                    codec.id()
                ))
            })?
        };
        Ok(DecodedState {
            codec: codec.id().to_string(),
            confidence,
            encode_capable: codec.encode_capable(),
            base_sha256: sha256_hex(&blob),
            size_bytes: blob.len() as u64,
            path,
            doc: prune(subtree, req.depth),
        })
    }

    async fn edit_state(&self, req: EditStateRequest) -> Result<EditResult, ToolError> {
        Self::check_instance(&req.instance)?;
        if req.edits.is_empty() {
            return Err(ToolError::invalid_input("no edits given"));
        }
        // Baseline guard: the state may have changed since the decode the
        // edits were computed against (a knob turned in the plugin's editor).
        let live = self.bridge.save_state().await?;
        let live_sha = sha256_hex(&live);
        if live_sha != req.base_sha256 {
            return Err(ToolError::conflict(format!(
                "state changed since decode (base {}, live {live_sha}) — re-run decode_state and recompute the edits",
                req.base_sha256
            )));
        }

        let plugin_id = self.plugin_id().await?;
        let descriptor = self.descriptor_for(&plugin_id).await?;
        let (codec, _) = self.select_codec(descriptor.as_ref(), &live)?;
        let mut doc = codec.decode(descriptor.as_ref(), &live)?;
        apply_edits(&mut doc, &req.edits)?;
        let encoded = codec
            .encode(descriptor.as_ref(), &doc)
            .ok_or_else(|| {
                ToolError::unsupported(format!(
                    "codec {:?} is decode-only — it can show this plugin's state but not rewrite it yet",
                    codec.id()
                ))
            })??;

        if !req.apply {
            return Ok(EditResult {
                applied: false,
                sha256: sha256_hex(&encoded),
                size_bytes: encoded.len() as u64,
                auto_snapshot: None,
                verification: None,
            });
        }

        let auto = self.auto_snapshot("auto: before edit_state").await?;
        self.bridge.load_state(encoded).await?;

        // Verify through the plugin: re-save, re-decode, compare edited paths.
        let resaved = self.bridge.save_state().await?;
        let redoc = codec.decode(descriptor.as_ref(), &resaved)?;
        let mut mismatches = Vec::new();
        for edit in &req.edits {
            let actual = redoc.pointer(&edit.path);
            match &edit.op {
                EditOp::Set { value } => {
                    if !actual.is_some_and(|a| values_match(a, value)) {
                        mismatches.push(VerifyMismatch {
                            path: edit.path.clone(),
                            expected: Some(value.clone()),
                            actual: actual.cloned(),
                        });
                    }
                }
                EditOp::Remove => {
                    if let Some(actual) = actual {
                        mismatches.push(VerifyMismatch {
                            path: edit.path.clone(),
                            expected: None,
                            actual: Some(actual.clone()),
                        });
                    }
                }
            }
        }
        Ok(EditResult {
            applied: true,
            sha256: sha256_hex(&resaved),
            size_bytes: resaved.len() as u64,
            auto_snapshot: Some(auto),
            verification: Some(Verification { verified: mismatches.is_empty(), mismatches }),
        })
    }

    async fn get_state(&self, req: GetStateRequest) -> Result<StateBlobInfo, ToolError> {
        Self::check_instance(&req.instance)?;
        let (blob, plugin_id) = self.source_blob(None).await?;
        let size = blob.len() as u64;
        let (base64, data_omitted) = if !req.include_data {
            (None, None)
        } else if size <= req.max_bytes {
            (Some(BASE64.encode(blob.as_slice())), None)
        } else {
            (None, Some(format!("state is {size} bytes > max_bytes {} — raise max_bytes or use decode_state/snapshots instead", req.max_bytes)))
        };
        Ok(StateBlobInfo { plugin_id, size_bytes: size, sha256: sha256_hex(&blob), base64, data_omitted })
    }

    async fn set_state(&self, req: SetStateRequest) -> Result<StateBlobInfo, ToolError> {
        Self::check_instance(&req.instance)?;
        let data = BASE64
            .decode(req.base64.as_bytes())
            .map_err(|e| ToolError::invalid_input(format!("base64: {e}")))?;
        let _auto = self.auto_snapshot("auto: before set_state").await?;
        self.bridge.load_state(data).await?;
        // Re-save: proves the plugin accepted it and reports the state as the
        // plugin now holds it (plugins may normalize).
        let resaved = self.bridge.save_state().await?;
        let plugin_id = self.plugin_id().await?;
        Ok(StateBlobInfo {
            plugin_id,
            size_bytes: resaved.len() as u64,
            sha256: sha256_hex(&resaved),
            base64: None,
            data_omitted: None,
        })
    }

    async fn snapshot_state(&self, req: SnapshotStateRequest) -> Result<SnapshotInfo, ToolError> {
        Self::check_instance(&req.instance)?;
        let blob = self.bridge.save_state().await?;
        let plugin_id = self.plugin_id().await?;
        let sha = sha256_hex(&blob);
        let mut info = self.snapshots.insert(req.name, req.note, plugin_id, Arc::new(blob), sha);
        if req.export {
            let path = self.snapshots.export(&info.name)?;
            info.exported_path = Some(path.display().to_string());
        }
        Ok(info)
    }

    async fn restore_snapshot(&self, req: RestoreSnapshotRequest) -> Result<RestoreResult, ToolError> {
        Self::check_instance(&req.instance)?;
        let snapshot = self.snapshots.get(&req.name)?;
        let plugin_id = self.plugin_id().await?;
        if snapshot.info.plugin_id != plugin_id {
            return Err(ToolError::conflict(format!(
                "snapshot {:?} belongs to plugin {:?} but {plugin_id:?} is loaded",
                req.name, snapshot.info.plugin_id
            )));
        }
        let auto = self.auto_snapshot(&format!("auto: before restoring {:?}", req.name)).await?;
        self.bridge.load_state(snapshot.data.as_ref().clone()).await?;
        Ok(RestoreResult { restored: snapshot.info, auto_snapshot: Some(auto) })
    }

    async fn list_snapshots(&self, req: ListSnapshotsRequest) -> Result<SnapshotList, ToolError> {
        Self::check_instance(&req.instance)?;
        Ok(SnapshotList { snapshots: self.snapshots.list() })
    }

    async fn diff_state(&self, req: DiffStateRequest) -> Result<StateDiff, ToolError> {
        Self::check_instance(&req.instance)?;
        let a = self.snapshots.get(&req.a)?;
        let (b_blob, b_plugin) = self.source_blob(req.b.as_deref()).await?;
        let descriptor = self.descriptor_for(&b_plugin).await?;
        // One codec for both sides — comparing documents from different
        // decoders would diff the decoders, not the states.
        let (codec, _) = self.select_codec(descriptor.as_ref(), &b_blob)?;
        let doc_a = codec.decode(descriptor.as_ref(), &a.data)?;
        let doc_b = codec.decode(descriptor.as_ref(), &b_blob)?;
        let outcome = diff_docs(&doc_a, &doc_b);
        Ok(StateDiff { codec: codec.id().to_string(), changes: outcome.changes, truncated: outcome.truncated })
    }

    async fn list_codecs(&self, req: ListCodecsRequest) -> Result<CodecList, ToolError> {
        Self::check_instance(&req.instance)?;
        Ok(CodecList { codecs: self.codecs.list() })
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Value equality with float tolerance — plugins may reformat numbers when
/// re-saving (0.9 vs 0.9000000001 is a pass, not a mismatch).
fn values_match(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{self, HostBridge, HostSession, MainThreadHost};
    use crate::error::ErrorCode;
    use futures::executor::block_on;
    use serde_json::json;

    /// A fake app: state is an in-memory blob, "loaded plugin" is Vital-like
    /// (JSON state), so the vital-json codec drives the full edit flow.
    struct MockHost {
        loaded: bool,
        descriptor: PluginDescriptor,
        state: Vec<u8>,
    }

    impl MockHost {
        fn vital() -> Self {
            Self {
                loaded: true,
                descriptor: PluginDescriptor {
                    id: "audio.vital.synth".into(),
                    name: Some("Vital".into()),
                    vendor: None,
                    version: None,
                },
                state: serde_json::to_vec(&json!({
                    "settings": { "osc_1_level": 0.7, "filter_1_cutoff": 0.42 }
                }))
                .unwrap(),
            }
        }

        fn binary() -> Self {
            Self {
                loaded: true,
                descriptor: PluginDescriptor {
                    id: "com.example.mystery".into(),
                    name: Some("Mystery".into()),
                    vendor: None,
                    version: None,
                },
                state: vec![0x00, 0x01, 0x02, 0xff, 0xfe, 0x00, 0x10],
            }
        }
    }

    impl MainThreadHost for MockHost {
        fn session(&mut self) -> HostSession {
            HostSession {
                loaded: self.loaded.then(|| LoadedPlugin {
                    plugin_id: self.descriptor.id.clone(),
                    name: self.descriptor.name.clone(),
                    vendor: None,
                    format: "clap".into(),
                }),
                sample_rate: Some(48_000.0),
            }
        }

        fn descriptor(&mut self) -> Option<PluginDescriptor> {
            self.loaded.then(|| self.descriptor.clone())
        }

        fn save_state(&mut self) -> Result<Vec<u8>, ToolError> {
            if !self.loaded {
                return Err(ToolError::no_plugin());
            }
            Ok(self.state.clone())
        }

        fn load_state(&mut self, data: &[u8]) -> Result<(), ToolError> {
            if !self.loaded {
                return Err(ToolError::no_plugin());
            }
            self.state = data.to_vec();
            Ok(())
        }
    }

    /// Service over a mock host serviced by a background thread — the same
    /// marshalling the real app does, minus egui.
    fn service(mock: MockHost) -> (Arc<ToolsService>, std::thread::JoinHandle<()>) {
        let (host_bridge, rx) = HostBridge::channel(Arc::new(|| {}));
        let handle = std::thread::spawn(move || {
            let mut mock = mock;
            while let Ok(request) = rx.recv() {
                bridge::handle(request, &mut mock);
            }
        });
        (Arc::new(ToolsService::new(host_bridge)), handle)
    }

    fn done(service: Arc<ToolsService>, handle: std::thread::JoinHandle<()>) {
        drop(service);
        handle.join().unwrap();
    }

    #[test]
    fn session_reports_loaded_plugin() {
        let (svc, handle) = service(MockHost::vital());
        let session = block_on(svc.get_session(GetSessionRequest::default())).unwrap();
        assert_eq!(session.loaded.unwrap().plugin_id, "audio.vital.synth");
        done(svc, handle);
    }

    #[test]
    fn unknown_instance_is_rejected() {
        let (svc, handle) = service(MockHost::vital());
        let err = block_on(svc.get_session(GetSessionRequest { instance: "other".into() }))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInstance);
        done(svc, handle);
    }

    #[test]
    fn get_state_data_gating() {
        let (svc, handle) = service(MockHost::vital());
        let no_data = block_on(svc.get_state(GetStateRequest {
            instance: "default".into(),
            include_data: false,
            max_bytes: 1 << 20,
        }))
        .unwrap();
        assert!(no_data.base64.is_none());

        let with_data = block_on(svc.get_state(GetStateRequest {
            instance: "default".into(),
            include_data: true,
            max_bytes: 1 << 20,
        }))
        .unwrap();
        assert!(with_data.base64.is_some());

        let capped = block_on(svc.get_state(GetStateRequest {
            instance: "default".into(),
            include_data: true,
            max_bytes: 4,
        }))
        .unwrap();
        assert!(capped.base64.is_none());
        assert!(capped.data_omitted.is_some());
        done(svc, handle);
    }

    #[test]
    fn decode_uses_vital_codec_and_paths() {
        let (svc, handle) = service(MockHost::vital());
        let decoded = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: Some("/settings".into()),
            depth: 4,
        }))
        .unwrap();
        assert_eq!(decoded.codec, "vital-json");
        assert!(decoded.encode_capable);
        assert_eq!(decoded.doc["osc_1_level"], 0.7);

        let missing = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: Some("/nope".into()),
            depth: 4,
        }));
        assert_eq!(missing.unwrap_err().code, ErrorCode::NotFound);
        done(svc, handle);
    }

    #[test]
    fn decode_falls_back_to_generic_for_unknown_binary() {
        let (svc, handle) = service(MockHost::binary());
        let decoded = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: None,
            depth: 6,
        }))
        .unwrap();
        assert_eq!(decoded.codec, "generic");
        assert!(!decoded.encode_capable);
        assert_eq!(decoded.doc["kind"], "binary");
        done(svc, handle);
    }

    #[test]
    fn edit_state_full_flow_with_verification() {
        let (svc, handle) = service(MockHost::vital());
        let base = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: None,
            depth: 8,
        }))
        .unwrap();

        // Dry-run: nothing changes.
        let dry = block_on(svc.edit_state(EditStateRequest {
            instance: "default".into(),
            base_sha256: base.base_sha256.clone(),
            edits: vec![StateEdit {
                path: "/settings/filter_1_cutoff".into(),
                op: EditOp::Set { value: json!(0.58) },
            }],
            apply: false,
        }))
        .unwrap();
        assert!(!dry.applied);
        let still = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: Some("/settings/filter_1_cutoff".into()),
            depth: 1,
        }))
        .unwrap();
        assert_eq!(still.doc, json!(0.42));

        // Apply: state changes, verification passes, auto-snapshot exists.
        let applied = block_on(svc.edit_state(EditStateRequest {
            instance: "default".into(),
            base_sha256: base.base_sha256.clone(),
            edits: vec![StateEdit {
                path: "/settings/filter_1_cutoff".into(),
                op: EditOp::Set { value: json!(0.58) },
            }],
            apply: true,
        }))
        .unwrap();
        assert!(applied.applied);
        assert!(applied.verification.as_ref().unwrap().verified);
        let auto = applied.auto_snapshot.clone().unwrap();

        let now = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: Some("/settings/filter_1_cutoff".into()),
            depth: 1,
        }))
        .unwrap();
        assert_eq!(now.doc, json!(0.58));

        // The stale base hash now conflicts.
        let stale = block_on(svc.edit_state(EditStateRequest {
            instance: "default".into(),
            base_sha256: base.base_sha256,
            edits: vec![StateEdit { path: "/settings/osc_1_level".into(), op: EditOp::Set { value: json!(0.1) } }],
            apply: true,
        }));
        assert_eq!(stale.unwrap_err().code, ErrorCode::Conflict);

        // The auto-snapshot restores the pre-edit state.
        let restored = block_on(svc.restore_snapshot(RestoreSnapshotRequest {
            instance: "default".into(),
            name: auto,
        }))
        .unwrap();
        assert!(restored.auto_snapshot.is_some());
        let back = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: Some("/settings/filter_1_cutoff".into()),
            depth: 1,
        }))
        .unwrap();
        assert_eq!(back.doc, json!(0.42));
        done(svc, handle);
    }

    #[test]
    fn edit_on_decode_only_codec_is_unsupported() {
        let (svc, handle) = service(MockHost::binary());
        let base = block_on(svc.decode_state(DecodeStateRequest {
            instance: "default".into(),
            snapshot: None,
            path: None,
            depth: 2,
        }))
        .unwrap();
        let err = block_on(svc.edit_state(EditStateRequest {
            instance: "default".into(),
            base_sha256: base.base_sha256,
            edits: vec![StateEdit { path: "/kind".into(), op: EditOp::Set { value: json!("x") } }],
            apply: true,
        }))
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        done(svc, handle);
    }

    #[test]
    fn snapshot_diff_names_the_changed_path() {
        let (svc, handle) = service(MockHost::vital());
        let snap = block_on(svc.snapshot_state(SnapshotStateRequest {
            instance: "default".into(),
            name: Some("baseline".into()),
            note: None,
            export: false,
        }))
        .unwrap();
        assert_eq!(snap.name, "baseline");

        // Turn a knob (via set_state, standing in for the plugin GUI).
        let new_state = json!({ "settings": { "osc_1_level": 0.7, "filter_1_cutoff": 0.58 } });
        block_on(svc.set_state(SetStateRequest {
            instance: "default".into(),
            base64: BASE64.encode(serde_json::to_vec(&new_state).unwrap()),
        }))
        .unwrap();

        let diff = block_on(svc.diff_state(DiffStateRequest {
            instance: "default".into(),
            a: "baseline".into(),
            b: None,
        }))
        .unwrap();
        assert_eq!(diff.codec, "vital-json");
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].path, "/settings/filter_1_cutoff");
        done(svc, handle);
    }

    #[test]
    fn snapshot_restore_rejects_plugin_mismatch() {
        let (svc, handle) = service(MockHost::vital());
        block_on(svc.snapshot_state(SnapshotStateRequest {
            instance: "default".into(),
            name: Some("v".into()),
            note: None,
            export: false,
        }))
        .unwrap();
        // Forge a snapshot from another plugin.
        svc.snapshots.insert(
            Some("other".into()),
            None,
            "some.other.plugin".into(),
            Arc::new(vec![1, 2, 3]),
            "xx".into(),
        );
        let err = block_on(svc.restore_snapshot(RestoreSnapshotRequest {
            instance: "default".into(),
            name: "other".into(),
        }))
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict);
        done(svc, handle);
    }

    #[test]
    fn no_plugin_errors_are_actionable() {
        let mut mock = MockHost::vital();
        mock.loaded = false;
        let (svc, handle) = service(mock);
        let err = block_on(svc.get_state(GetStateRequest {
            instance: "default".into(),
            include_data: false,
            max_bytes: 1024,
        }))
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NoPlugin);
        done(svc, handle);
    }
}

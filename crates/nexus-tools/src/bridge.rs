//! The main-thread bridge.
//!
//! `PluginInstance` is deliberately not `Send` — every plugin call happens on
//! the app's main thread. Tool implementations run wherever the frontend's
//! runtime puts them, so plugin work crosses this bridge: a request enum with a
//! oneshot reply sender, drained by the app once per frame.
//!
//! Timeouts are the frontend's job (it owns a runtime; this crate has none) —
//! see [`crate::registry::TimeoutClass`].

use std::sync::Arc;

use futures::channel::oneshot;
use nexus_host::host::PluginDescriptor;

use crate::error::ToolError;
use crate::types::{LoadedPlugin, SessionInfo};

/// What the session's owner (the app) reports about itself. The service adds
/// tool-side fields (corpus directory) to build [`SessionInfo`].
#[derive(Debug, Clone)]
pub struct HostSession {
    pub loaded: Option<LoadedPlugin>,
    pub sample_rate: Option<f64>,
}

/// Implemented by whoever owns the `PluginInstance` on its main thread —
/// `NexusApp` today, nexus-wrap's main-thread half later.
pub trait MainThreadHost {
    fn session(&mut self) -> HostSession;
    /// Descriptor of the loaded plugin (feeds codec selection).
    fn descriptor(&mut self) -> Option<PluginDescriptor>;
    fn save_state(&mut self) -> Result<Vec<u8>, ToolError>;
    fn load_state(&mut self, data: &[u8]) -> Result<(), ToolError>;

    fn list_plugins(&mut self, _query: Option<&str>) -> Result<Vec<crate::types::PluginItem>, ToolError> {
        Ok(Vec::new())
    }
    fn load_plugin(&mut self, _plugin: &str) -> Result<LoadedPlugin, ToolError> {
        Err(ToolError::not_found("load_plugin not implemented on host"))
    }
    fn list_presets(&mut self, _plugin_id: Option<&str>) -> Result<Vec<crate::types::PresetItem>, ToolError> {
        Ok(Vec::new())
    }
    fn load_preset(&mut self, _preset: &str) -> Result<crate::types::LoadPresetResponse, ToolError> {
        Err(ToolError::not_found("load_preset not implemented on host"))
    }
    fn save_preset(&mut self, _name: &str, _description: Option<&str>) -> Result<crate::types::SavePresetResponse, ToolError> {
        Err(ToolError::internal("save_preset not implemented on host"))
    }
    fn list_params(
        &mut self,
        _query: Option<&str>,
        _offset: Option<usize>,
        _limit: Option<usize>,
    ) -> Result<crate::types::ListParamsResponse, ToolError> {
        Ok(crate::types::ListParamsResponse {
            total: 0,
            offset: 0,
            params: Vec::new(),
        })
    }
    fn get_params(&mut self, _ids: &[u32]) -> Result<Vec<crate::types::ParamValueItem>, ToolError> {
        Ok(Vec::new())
    }
    fn set_params(&mut self, _params: &[crate::types::SetParamItem]) -> Result<Vec<crate::types::ParamValueItem>, ToolError> {
        Ok(Vec::new())
    }
    fn play(&mut self, _fugue: &nexus_host::fugue::CompositeFugue) -> Result<crate::types::PlayResponse, ToolError> {
        Ok(crate::types::PlayResponse {
            queued: true,
            duration_beats: 4.0,
            total_events: 0,
        })
    }
    fn render_audio(&mut self, _req: &crate::types::RenderAudioRequest) -> Result<crate::types::RenderAudioResponse, ToolError> {
        Err(ToolError::internal("render_audio not implemented on host"))
    }
}

/// One marshalled call: arguments + the oneshot the reply goes back on.
pub enum HostRequest {
    Session(oneshot::Sender<HostSession>),
    Descriptor(oneshot::Sender<Option<PluginDescriptor>>),
    SaveState(oneshot::Sender<Result<Vec<u8>, ToolError>>),
    LoadState(Vec<u8>, oneshot::Sender<Result<(), ToolError>>),
    ListPlugins(Option<String>, oneshot::Sender<Result<Vec<crate::types::PluginItem>, ToolError>>),
    LoadPlugin(String, oneshot::Sender<Result<LoadedPlugin, ToolError>>),
    ListPresets(Option<String>, oneshot::Sender<Result<Vec<crate::types::PresetItem>, ToolError>>),
    LoadPreset(String, oneshot::Sender<Result<crate::types::LoadPresetResponse, ToolError>>),
    SavePreset(String, Option<String>, oneshot::Sender<Result<crate::types::SavePresetResponse, ToolError>>),
    ListParams(Option<String>, Option<usize>, Option<usize>, oneshot::Sender<Result<crate::types::ListParamsResponse, ToolError>>),
    GetParams(Vec<u32>, oneshot::Sender<Result<Vec<crate::types::ParamValueItem>, ToolError>>),
    SetParams(Vec<crate::types::SetParamItem>, oneshot::Sender<Result<Vec<crate::types::ParamValueItem>, ToolError>>),
    Play(nexus_host::fugue::CompositeFugue, oneshot::Sender<Result<crate::types::PlayResponse, ToolError>>),
    RenderAudio(crate::types::RenderAudioRequest, oneshot::Sender<Result<crate::types::RenderAudioResponse, ToolError>>),
}

/// Handle held by the tools service; `Clone` and `Send`. Each call enqueues a
/// request, pokes the waker (so the app repaints even if idle), and awaits the
/// reply.
#[derive(Clone)]
pub struct HostBridge {
    tx: crossbeam_channel::Sender<HostRequest>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl HostBridge {
    /// Create the bridge and the receiver the app drains. `waker` is invoked
    /// after every enqueue — the app passes `ctx.request_repaint()`.
    pub fn channel(
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> (Self, crossbeam_channel::Receiver<HostRequest>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Self { tx, waker }, rx)
    }

    async fn call<T>(&self, build: impl FnOnce(oneshot::Sender<T>) -> HostRequest) -> Result<T, ToolError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(build(reply_tx))
            .map_err(|_| ToolError::internal("host bridge closed (app gone)"))?;
        (self.waker)();
        reply_rx
            .await
            .map_err(|_| ToolError::internal("host dropped the request without replying"))
    }

    pub async fn session(&self) -> Result<HostSession, ToolError> {
        self.call(HostRequest::Session).await
    }

    pub async fn descriptor(&self) -> Result<Option<PluginDescriptor>, ToolError> {
        self.call(HostRequest::Descriptor).await
    }

    pub async fn save_state(&self) -> Result<Vec<u8>, ToolError> {
        self.call(HostRequest::SaveState).await?
    }

    pub async fn load_state(&self, data: Vec<u8>) -> Result<(), ToolError> {
        self.call(|tx| HostRequest::LoadState(data, tx)).await?
    }

    pub async fn list_plugins(&self, query: Option<String>) -> Result<Vec<crate::types::PluginItem>, ToolError> {
        self.call(|tx| HostRequest::ListPlugins(query, tx)).await?
    }

    pub async fn load_plugin(&self, plugin: String) -> Result<LoadedPlugin, ToolError> {
        self.call(|tx| HostRequest::LoadPlugin(plugin, tx)).await?
    }

    pub async fn list_presets(&self, plugin_id: Option<String>) -> Result<Vec<crate::types::PresetItem>, ToolError> {
        self.call(|tx| HostRequest::ListPresets(plugin_id, tx)).await?
    }

    pub async fn load_preset(&self, preset: String) -> Result<crate::types::LoadPresetResponse, ToolError> {
        self.call(|tx| HostRequest::LoadPreset(preset, tx)).await?
    }

    pub async fn save_preset(&self, name: String, description: Option<String>) -> Result<crate::types::SavePresetResponse, ToolError> {
        self.call(|tx| HostRequest::SavePreset(name, description, tx)).await?
    }

    pub async fn list_params(&self, query: Option<String>, offset: Option<usize>, limit: Option<usize>) -> Result<crate::types::ListParamsResponse, ToolError> {
        self.call(|tx| HostRequest::ListParams(query, offset, limit, tx)).await?
    }

    pub async fn get_params(&self, ids: Vec<u32>) -> Result<Vec<crate::types::ParamValueItem>, ToolError> {
        self.call(|tx| HostRequest::GetParams(ids, tx)).await?
    }

    pub async fn set_params(&self, params: Vec<crate::types::SetParamItem>) -> Result<Vec<crate::types::ParamValueItem>, ToolError> {
        self.call(|tx| HostRequest::SetParams(params, tx)).await?
    }

    pub async fn play(&self, fugue: nexus_host::fugue::CompositeFugue) -> Result<crate::types::PlayResponse, ToolError> {
        self.call(|tx| HostRequest::Play(fugue, tx)).await?
    }

    pub async fn render_audio(&self, req: crate::types::RenderAudioRequest) -> Result<crate::types::RenderAudioResponse, ToolError> {
        self.call(|tx| HostRequest::RenderAudio(req, tx)).await?
    }
}

/// Serve one request against the host. Shared by [`drain`] and by tests that
/// run their own receive loop.
pub fn handle(request: HostRequest, host: &mut dyn MainThreadHost) {
    match request {
        HostRequest::Session(reply) => {
            let _ = reply.send(host.session());
        }
        HostRequest::Descriptor(reply) => {
            let _ = reply.send(host.descriptor());
        }
        HostRequest::SaveState(reply) => {
            let _ = reply.send(host.save_state());
        }
        HostRequest::LoadState(data, reply) => {
            let _ = reply.send(host.load_state(&data));
        }
        HostRequest::ListPlugins(query, reply) => {
            let _ = reply.send(host.list_plugins(query.as_deref()));
        }
        HostRequest::LoadPlugin(plugin, reply) => {
            let _ = reply.send(host.load_plugin(&plugin));
        }
        HostRequest::ListPresets(plugin_id, reply) => {
            let _ = reply.send(host.list_presets(plugin_id.as_deref()));
        }
        HostRequest::LoadPreset(preset, reply) => {
            let _ = reply.send(host.load_preset(&preset));
        }
        HostRequest::SavePreset(name, description, reply) => {
            let _ = reply.send(host.save_preset(&name, description.as_deref()));
        }
        HostRequest::ListParams(query, offset, limit, reply) => {
            let _ = reply.send(host.list_params(query.as_deref(), offset, limit));
        }
        HostRequest::GetParams(ids, reply) => {
            let _ = reply.send(host.get_params(&ids));
        }
        HostRequest::SetParams(params, reply) => {
            let _ = reply.send(host.set_params(&params));
        }
        HostRequest::Play(fugue, reply) => {
            let _ = reply.send(host.play(&fugue));
        }
        HostRequest::RenderAudio(req, reply) => {
            let _ = reply.send(host.render_audio(&req));
        }
    }
}

/// Drain all pending requests on the main thread. Call once per frame; the
/// single drain point also serializes tool operations against the plugin.
pub fn drain(rx: &crossbeam_channel::Receiver<HostRequest>, host: &mut dyn MainThreadHost) {
    while let Ok(request) = rx.try_recv() {
        handle(request, host);
    }
}

impl SessionInfo {
    pub fn from_host(host: HostSession, corpus_directory: String) -> Self {
        Self { loaded: host.loaded, sample_rate: host.sample_rate, corpus_directory }
    }
}

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
}

/// One marshalled call: arguments + the oneshot the reply goes back on.
pub enum HostRequest {
    Session(oneshot::Sender<HostSession>),
    Descriptor(oneshot::Sender<Option<PluginDescriptor>>),
    SaveState(oneshot::Sender<Result<Vec<u8>, ToolError>>),
    LoadState(Vec<u8>, oneshot::Sender<Result<(), ToolError>>),
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

//! The clack host handler types Nexus presents to CLAP plugins.
//!
//! clack splits host state across three thread contexts — shared (any thread),
//! main-thread, and audio-processor — and asks the host to declare which
//! host-side extensions it offers. This module wires up that scaffolding plus
//! the `log` extension; later tasks register `audio-ports`, `note-ports`,
//! `params`, `timer`, and `gui`. Modeled on clack's official cpal host example.

use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_host::prelude::*;
use crossbeam_channel::Sender;

/// Messages sent to the main thread from any of a plugin's threads. A plugin
/// can ask (from any thread) for its main-thread callback to run; the audio and
/// UI loops drain this channel.
pub enum MainThreadMessage {
    /// Run the plugin's `on_main_thread` callback.
    RunOnMainThread,
}

/// Identity this host reports to plugins.
pub fn host_info() -> HostInfo {
    HostInfo::new(
        "Nexus",
        "Simply Chris",
        "https://github.com/poucet/nexus",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("static host info strings are always valid")
}

/// The clack host handler bundle for Nexus.
pub struct NexusHost;

impl HostHandlers for NexusHost {
    type Shared<'a> = NexusHostShared;
    type MainThread<'a> = NexusHostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostLog>();
    }
}

/// Host data reachable from every plugin thread (must be `Send + Sync`).
pub struct NexusHostShared {
    sender: Sender<MainThreadMessage>,
}

impl NexusHostShared {
    pub fn new(sender: Sender<MainThreadMessage>) -> Self {
        Self { sender }
    }
}

impl<'a> SharedHandler<'a> for NexusHostShared {
    fn request_restart(&self) {
        // Activation/restart support comes with the engine (F5); ignore for now.
    }

    fn request_process(&self) {
        // The engine drives processing; nothing to schedule here yet.
    }

    fn request_callback(&self) {
        // Best-effort: if the main-thread loop has gone, there's nothing to run.
        let _ = self.sender.send(MainThreadMessage::RunOnMainThread);
    }
}

/// Host data only touched on the main thread.
pub struct NexusHostMainThread<'a> {
    _shared: &'a NexusHostShared,
    /// Handle to the running plugin instance, set once it is initialized.
    plugin: Option<InitializedPluginHandle<'a>>,
}

impl<'a> NexusHostMainThread<'a> {
    pub fn new(shared: &'a NexusHostShared) -> Self {
        Self { _shared: shared, plugin: None }
    }
}

impl<'a> MainThreadHandler<'a> for NexusHostMainThread<'a> {
    fn initialized(&mut self, instance: InitializedPluginHandle<'a>) {
        self.plugin = Some(instance);
    }
}

/// Host-side `log` extension: forward plugin log messages to the `log` crate.
impl HostLogImpl for NexusHostShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        match severity {
            LogSeverity::Debug => log::debug!("[plugin] {message}"),
            LogSeverity::Info => log::info!("[plugin] {message}"),
            LogSeverity::Warning => log::warn!("[plugin] {message}"),
            _ => log::error!("[plugin] {message}"),
        }
    }
}

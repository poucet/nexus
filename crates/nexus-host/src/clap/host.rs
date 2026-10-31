//! The clack host handler types Nexus presents to CLAP plugins.
//!
//! clack splits host state across three thread contexts — shared (any thread),
//! main-thread, and audio-processor — and asks the host to declare which
//! host-side extensions it offers. This module wires up that scaffolding plus
//! the `log` extension; later tasks register `audio-ports`, `note-ports`,
//! `params`, `timer`, and `gui`. Modeled on clack's official cpal host example.

use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_host::prelude::*;
use crossbeam_channel::Sender;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

/// Messages sent to the main thread by the plugin's editor (`gui` extension).
///
/// `request_callback` is not one of them: plugins call it from the audio
/// thread, so it only sets a flag (see [`NexusHostShared::take_callback_request`]).
pub enum MainThreadMessage {
    /// The plugin asked its editor window to resize to this size.
    GuiResize(GuiSize),
    /// The plugin's editor window was closed.
    GuiClosed,
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
        builder.register::<HostLog>().register::<HostGui>().register::<HostThreadCheck>();
    }
}

/// Host data reachable from every plugin thread (must be `Send + Sync`).
pub struct NexusHostShared {
    sender: Sender<MainThreadMessage>,
    /// Set by `request_callback` from any thread; cleared by `pump`.
    callback_requested: AtomicBool,
    /// The thread that made the instance and owns it: the plugin's "main
    /// thread" (`thread-check`), whichever thread of the process it is.
    owner: ThreadId,
}

impl NexusHostShared {
    /// Made on the thread that will own the instance.
    pub fn new(sender: Sender<MainThreadMessage>) -> Self {
        Self {
            sender,
            callback_requested: AtomicBool::new(false),
            owner: std::thread::current().id(),
        }
    }

    /// Whether the plugin asked for `on_main_thread` since the last call;
    /// clears the request.
    pub fn take_callback_request(&self) -> bool {
        self.callback_requested.swap(false, Ordering::AcqRel)
    }
}

impl<'a> SharedHandler<'a> for NexusHostShared {
    fn request_restart(&self) {
        // Activation/restart support comes with the engine (F5); ignore for now.
    }

    fn request_process(&self) {
        // The engine drives processing; nothing to schedule here yet.
    }

    /// Realtime-safe: JUCE-built plugins call this from `process()` on every
    /// parameter change, so it must not allocate or block — a flag, not a
    /// queue. Requests coalesce, as CLAP allows: one `on_main_thread` per pump.
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::Release);
    }
}

thread_local! {
    /// How many audio-processor calls into a plugin this thread is inside.
    static IN_AUDIO: Cell<u32> = const { Cell::new(0) };
}

/// Marks the current thread as a plugin's audio thread while it lives: held
/// around every `start_processing`, `process` and `stop_processing` call, so
/// `thread-check`'s `is_audio_thread` answers true inside them. A
/// thread-local counter: no allocation, no lock.
pub(crate) struct AudioThread(());

impl AudioThread {
    pub(crate) fn enter() -> Self {
        IN_AUDIO.with(|n| n.set(n.get() + 1));
        Self(())
    }

    fn here() -> bool {
        IN_AUDIO.with(|n| n.get() > 0)
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        IN_AUDIO.with(|n| n.set(n.get() - 1));
    }
}

/// Host-side `thread-check` extension (`N10`). The main thread is the
/// instance's owning thread; the audio thread is whichever thread is inside
/// one of its processor's calls. An offline render on the owning thread is
/// both, which CLAP allows. `is_main_thread` reads `std::thread::current()`,
/// which allocates once on a thread std did not start (a CoreAudio thread)
/// the first time it is asked there.
impl HostThreadCheckImpl for NexusHostShared {
    fn is_main_thread(&self) -> bool {
        std::thread::current().id() == self.owner
    }

    fn is_audio_thread(&self) -> bool {
        AudioThread::here()
    }
}

/// Host data only touched on the owning (main) thread.
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

/// Host-side `gui` extension: the plugin uses these to drive its editor window.
/// We forward resize/close to the main thread (which owns the window) via the
/// message channel.
impl HostGuiImpl for NexusHostShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        let _ = self.sender.send(MainThreadMessage::GuiResize(new_size));
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        let _ = self.sender.send(MainThreadMessage::GuiClosed);
    }
}

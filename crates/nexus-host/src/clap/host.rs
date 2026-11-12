//! The clack host handler types Nexus presents to CLAP plugins.
//!
//! clack splits host state across three thread contexts — shared (any thread),
//! main-thread, and audio-processor — and asks the host to declare which
//! host-side extensions it offers. This module wires up that scaffolding plus
//! the `log` extension; later tasks register `audio-ports`, `note-ports`,
//! `params`, `timer`, and `gui`. Modeled on clack's official cpal host example.

use clack_extensions::audio_ports::{AudioPortRescanFlags, HostAudioPorts, HostAudioPortsImpl};
use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl};
use clack_extensions::latency::{HostLatency, HostLatencyImpl};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::note_ports::{
    HostNotePorts, HostNotePortsImpl, NoteDialects, NotePortRescanFlags,
};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_host::prelude::*;
use crossbeam_channel::Sender;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

/// What the plugin told the host on its owning thread, queued for `pump`
/// (`gui`, `latency`, `params`, `audio-ports` and `note-ports` callbacks).
///
/// `request_callback`, `request_restart` and `request_flush` are not among
/// them: plugins may call those from the audio thread, so each only sets a flag
/// (see [`NexusHostShared::take_requests`]).
pub enum MainThreadMessage {
    /// The plugin asked its editor window to resize to this size.
    GuiResize(GuiSize),
    /// The plugin's editor window was closed.
    GuiClosed,
    /// `latency.changed`: ask the plugin for its latency again.
    LatencyChanged,
    /// `params.rescan`, `audio-ports.rescan` or `note-ports.rescan`.
    Rescan(crate::notice::Rescan),
    /// `params.clear` for this parameter.
    ParamCleared(u32),
}

/// What the plugin asked for from any thread since the last
/// [`NexusHostShared::take_requests`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Requests {
    /// `request_callback`: run `on_main_thread`.
    pub callback: bool,
    /// `request_restart`: deactivate and activate again.
    pub restart: bool,
    /// `params.request_flush`: flush parameters (on an inactive plugin; an
    /// active one is flushed by its next `process()`).
    pub flush: bool,
}

/// Identity this host reports to plugins.
pub fn host_info() -> HostInfo {
    HostInfo::new(
        "Nexus",
        "Simply Chris",
        "https://github.com/simply-audio/nexus",
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
        builder
            .register::<HostLog>()
            .register::<HostGui>()
            .register::<HostThreadCheck>()
            .register::<HostLatency>()
            .register::<HostParams>()
            .register::<HostAudioPorts>()
            .register::<HostNotePorts>();
    }
}

/// Host data reachable from every plugin thread (must be `Send + Sync`).
pub struct NexusHostShared {
    sender: Sender<MainThreadMessage>,
    /// Set by `request_callback` from any thread; cleared by `pump`.
    callback_requested: AtomicBool,
    /// Set by `request_restart` from any thread; cleared by `pump`.
    restart_requested: AtomicBool,
    /// Set by `params.request_flush` from any thread; cleared by `pump`.
    flush_requested: AtomicBool,
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
            restart_requested: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
            owner: std::thread::current().id(),
        }
    }

    /// What the plugin asked for since the last call; clears the requests.
    pub fn take_requests(&self) -> Requests {
        Requests {
            callback: self.callback_requested.swap(false, Ordering::AcqRel),
            restart: self.restart_requested.swap(false, Ordering::AcqRel),
            flush: self.flush_requested.swap(false, Ordering::AcqRel),
        }
    }

    /// Queue `message` for the owning thread's next `pump`.
    fn post(&self, message: MainThreadMessage) {
        let _ = self.sender.send(message);
    }
}

impl<'a> SharedHandler<'a> for NexusHostShared {
    /// A flag, like `request_callback`: `pump` turns it into
    /// [`Notice::Restart`](crate::Notice::Restart).
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::Release);
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
    shared: &'a NexusHostShared,
    /// Handle to the running plugin instance, set once it is initialized.
    plugin: Option<InitializedPluginHandle<'a>>,
}

impl<'a> NexusHostMainThread<'a> {
    pub fn new(shared: &'a NexusHostShared) -> Self {
        Self { shared, plugin: None }
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
        self.post(MainThreadMessage::GuiResize(new_size));
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.post(MainThreadMessage::GuiClosed);
    }
}

/// Host-side `latency` extension: queued, so `pump` asks the plugin for its
/// latency outside this callback.
impl HostLatencyImpl for NexusHostMainThread<'_> {
    fn changed(&mut self) {
        self.shared.post(MainThreadMessage::LatencyChanged);
    }
}

/// Host-side `params` extension, main-thread half.
impl HostParamsImplMainThread for NexusHostMainThread<'_> {
    fn rescan(&mut self, flags: ParamRescanFlags) {
        let all = flags.contains(ParamRescanFlags::ALL);
        self.shared.post(MainThreadMessage::Rescan(crate::notice::Rescan {
            values: all || flags.contains(ParamRescanFlags::VALUES),
            text: all || flags.contains(ParamRescanFlags::TEXT),
            info: all || flags.contains(ParamRescanFlags::INFO),
            list: all,
            ..Default::default()
        }));
    }

    fn clear(&mut self, param_id: ClapId, _flags: ParamClearFlags) {
        self.shared.post(MainThreadMessage::ParamCleared(param_id.get()));
    }
}

/// Host-side `params` extension, thread-safe half: a flag.
impl HostParamsImplShared for NexusHostShared {
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::Release);
    }
}

/// Host-side `audio-ports` extension: every rescan is a [`Notice::Rescan`]
/// with `io` set; nexus reads the ports again at the next activation.
///
/// [`Notice::Rescan`]: crate::Notice::Rescan
impl HostAudioPortsImpl for NexusHostMainThread<'_> {
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        true
    }

    fn rescan(&mut self, _flags: AudioPortRescanFlags) {
        self.shared.post(MainThreadMessage::Rescan(crate::notice::Rescan {
            io: true,
            ..Default::default()
        }));
    }
}

/// Host-side `note-ports` extension: as `audio-ports`.
impl HostNotePortsImpl for NexusHostMainThread<'_> {
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::CLAP | NoteDialects::MIDI | NoteDialects::MIDI2
    }

    fn rescan(&mut self, _flags: NotePortRescanFlags) {
        self.shared.post(MainThreadMessage::Rescan(crate::notice::Rescan {
            io: true,
            ..Default::default()
        }));
    }
}

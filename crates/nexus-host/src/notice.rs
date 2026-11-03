//! Plugin → host notices: what a plugin tells its host on its own initiative.
//!
//! A parameter edited in the plugin's editor, a value the plugin's `process()`
//! wrote out, a latency change, a parameter list that changed. Both formats
//! deliver them as one [`Notice`] type, and a host reads them on the instance's
//! **owning thread**: [`PluginInstance::pump`] collects them, and
//! [`PluginInstance::take_notices`] hands them over.
//!
//! **Where they come from, and how they reach the owning thread:**
//!
//! - the **audio thread** (CLAP output events from `process()`, VST3
//!   `outputParameterChanges`) writes into a preallocated single-producer
//!   ring ([`ring`]): no allocation, no lock. A full ring drops the notice and
//!   counts it; `pump` logs the count.
//! - **plugin callbacks on the owning thread** (VST3 `IComponentHandler`, CLAP
//!   `latency.changed`, `params.rescan`, the main-thread `params.flush`) are
//!   queued as they arrive and translated by `pump`, which may ask the plugin
//!   for the new latency or a plain value; never re-entrantly inside the
//!   plugin's own callback.
//!
//! Order: each source keeps its order; within one `pump`, the owning thread's
//! notices come before the audio thread's.
//!
//! [`PluginInstance::pump`]: crate::PluginInstance::pump
//! [`PluginInstance::take_notices`]: crate::PluginInstance::take_notices

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// One thing a plugin told its host. `Copy` and heap-free, so the audio thread
/// can queue it without allocating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Notice {
    /// The plugin's latency is now this many samples. A host re-reads its
    /// delay compensation; the processor may need re-activating to apply it
    /// (VST3 reports it from `getLatencySamples`, CLAP from `latency.get`).
    Latency(u32),
    /// The plugin set parameter `id` to this **plain** value: an edit in its
    /// editor, or a value its `process()` wrote out. On VST3 nexus has also
    /// forwarded an editor edit to the processor; the host need not.
    ParamValue { id: u32, value: f64 },
    /// The user started dragging parameter `id` in the editor. A host makes the
    /// values up to the matching [`GestureEnd`](Self::GestureEnd) one edit.
    GestureBegin(u32),
    /// The user let go of parameter `id`.
    GestureEnd(u32),
    /// Something the host read from the plugin is stale: read it again.
    Rescan(Rescan),
    /// The plugin dropped parameter `id` (CLAP `params.clear`): forget its
    /// automation and modulation.
    ParamCleared(u32),
    /// The plugin asks to be deactivated and activated again (CLAP
    /// `request_restart`, VST3 `kReloadComponent`).
    Restart,
    /// The plugin asks its editor to be this size, in logical pixels (CLAP
    /// `gui.request_resize`, VST3 `IPlugFrame::resizeView`). A host that
    /// parented the editor resizes the parent; nexus's own window follows by
    /// itself.
    EditorResize { width: u32, height: u32 },
    /// The editor was closed, by the plugin or by the user closing nexus's
    /// window. The editor is gone; open it again to show it.
    EditorClosed,
}

/// What a [`Notice::Rescan`] says is stale. Several can be set at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rescan {
    /// Parameter values (CLAP `VALUES`, VST3 `kParamValuesChanged`).
    pub values: bool,
    /// Value-to-text conversions (CLAP `TEXT`).
    pub text: bool,
    /// Parameter names, ranges and flags (CLAP `INFO`, VST3
    /// `kParamTitlesChanged`).
    pub info: bool,
    /// The parameter list itself (CLAP `ALL`, VST3 `kParamIDMappingChanged`).
    pub list: bool,
    /// Audio or note ports (CLAP `audio-ports` / `note-ports` rescan, VST3
    /// `kIoChanged` / `kIoTitlesChanged`).
    pub io: bool,
    /// The MIDI controller → parameter mapping (VST3
    /// `kMidiCCAssignmentChanged`; nexus re-reads it for the next activation).
    pub midi_mapping: bool,
}

impl Rescan {
    /// Whether anything is set.
    pub fn any(&self) -> bool {
        self.values || self.text || self.info || self.list || self.io || self.midi_mapping
    }
}

/// Notices the audio thread can hold between two pumps.
pub const RING: usize = 1024;

/// The audio thread's end of the notice ring: realtime-safe.
pub struct NoticeSender {
    producer: rtrb::Producer<Notice>,
    dropped: Arc<AtomicU64>,
}

impl NoticeSender {
    /// Queue `notice`; a full ring drops it and counts it. Never allocates,
    /// never blocks. Returns whether it was kept.
    pub fn send(&mut self, notice: Notice) -> bool {
        if self.producer.push(notice).is_ok() {
            true
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// The owning thread's end of the notice ring.
pub struct NoticeReceiver {
    consumer: rtrb::Consumer<Notice>,
    dropped: Arc<AtomicU64>,
    reported: u64,
}

impl NoticeReceiver {
    /// Move every queued notice into `out`, in order; logs notices a full ring
    /// dropped since the last call.
    pub fn drain_into(&mut self, out: &mut Vec<Notice>) {
        while let Ok(notice) = self.consumer.pop() {
            out.push(notice);
        }
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > self.reported {
            log::warn!(
                "{} plugin notices dropped: the ring of {RING} filled between two pumps",
                dropped - self.reported
            );
            self.reported = dropped;
        }
    }

    /// Notices a full ring has dropped since it was made.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// A ring for `capacity` notices from the audio thread. Made on the owning
/// thread (it allocates), at activation.
pub fn ring(capacity: usize) -> (NoticeSender, NoticeReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        NoticeSender { producer, dropped: dropped.clone() },
        NoticeReceiver { consumer, dropped, reported: 0 },
    )
}

/// Somewhere a format's output events can be written as notices: the audio
/// ring, or a `Vec` on the owning thread.
pub trait NoticeSink {
    /// Keep `notice`; false if there was no room.
    fn notice(&mut self, notice: Notice) -> bool;
}

impl NoticeSink for NoticeSender {
    fn notice(&mut self, notice: Notice) -> bool {
        self.send(notice)
    }
}

impl NoticeSink for Vec<Notice> {
    fn notice(&mut self, notice: Notice) -> bool {
        self.push(notice);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_order_and_counts_what_it_drops() {
        let (mut tx, mut rx) = ring(3);
        for id in 0..5 {
            tx.send(Notice::GestureBegin(id));
        }
        let mut out = Vec::new();
        rx.drain_into(&mut out);
        assert_eq!(out, [Notice::GestureBegin(0), Notice::GestureBegin(1), Notice::GestureBegin(2)]);
        assert_eq!(rx.dropped(), 2);
        assert!(tx.send(Notice::Restart), "room again once drained");
    }

    #[test]
    fn rescan_any() {
        assert!(!Rescan::default().any());
        assert!(Rescan { io: true, ..Default::default() }.any());
    }
}

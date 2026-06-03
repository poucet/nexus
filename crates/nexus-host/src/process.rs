//! Audio and event buffers handed to a plugin for one processing block.
//!
//! These are the format-agnostic types that sit between the engine and a
//! plugin instance. Both CLAP and VST3 map their native buffer/event structs
//! to and from these.
//!
//! **Allocation discipline:** every buffer here is meant to be allocated once
//! (at the largest block size the engine will use) and then *reused* every
//! block via [`AudioBuffer::clear`] / [`EventBuffer::clear`] and
//! [`AudioBuffer::set_frames`]. Nothing on the per-block path allocates, which
//! keeps it safe to drive from a realtime audio callback.

/// Planar 32-bit float audio: a fixed set of channels, each holding up to
/// `capacity` samples, of which the first `frames` are live for the current
/// block.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    channels: Vec<Vec<f32>>,
    frames: usize,
}

impl AudioBuffer {
    /// Allocate `channels` channels each able to hold `capacity` samples. The
    /// live frame count starts at `capacity`; shrink it per block with
    /// [`set_frames`](Self::set_frames).
    pub fn new(channels: usize, capacity: usize) -> Self {
        Self {
            channels: vec![vec![0.0; capacity]; channels],
            frames: capacity,
        }
    }

    /// An empty buffer with no channels — used for the input side of an
    /// instrument, which consumes no audio.
    pub fn empty() -> Self {
        Self { channels: Vec::new(), frames: 0 }
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Number of live samples per channel for the current block.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Sample capacity per channel (the allocated length).
    pub fn capacity(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// Set the live frame count for this block. Panics if `frames` exceeds the
    /// allocated capacity — the engine must size the buffer for its max block.
    pub fn set_frames(&mut self, frames: usize) {
        assert!(
            frames <= self.capacity(),
            "frames {frames} exceeds capacity {}",
            self.capacity()
        );
        self.frames = frames;
    }

    /// Live samples of channel `index`, or `None` if out of range.
    pub fn channel(&self, index: usize) -> Option<&[f32]> {
        self.channels.get(index).map(|c| &c[..self.frames])
    }

    /// Mutable live samples of channel `index`, or `None` if out of range.
    pub fn channel_mut(&mut self, index: usize) -> Option<&mut [f32]> {
        let frames = self.frames;
        self.channels.get_mut(index).map(|c| &mut c[..frames])
    }

    /// Zero the live region of every channel. Reuse instead of reallocating.
    pub fn clear(&mut self) {
        for channel in &mut self.channels {
            channel[..self.frames].fill(0.0);
        }
    }
}

/// A single event, timed by its sample offset within the block.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedEvent {
    /// Sample offset from the start of the current block.
    pub time: u32,
    pub event: Event,
}

/// Which kind of note event this is (mirrors CLAP's note event types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteEventKind {
    On,
    Off,
    /// Terminate the voice immediately, without a release stage.
    Choke,
    /// A voice has ended (plugin → host direction).
    End,
}

/// Per-note expression dimension (mirrors CLAP's note expressions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteExpression {
    Volume,
    Pan,
    Tuning,
    Vibrato,
    Expression,
    Brightness,
    Pressure,
}

/// Transport / playhead state for the block (the audible subset of CLAP's
/// `clap_event_transport`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub loop_active: bool,
    pub tempo_bpm: f64,
    /// Song position, in beats.
    pub song_pos_beats: f64,
    /// Song position, in seconds.
    pub song_pos_seconds: f64,
    pub time_sig_numerator: u16,
    pub time_sig_denominator: u16,
}

/// A format-agnostic plugin event, modelled faithfully on CLAP's event union so
/// that hosting — and **wrapping CLAP-in-CLAP** — is lossless. VST3 maps onto
/// this same set. (CLAP's `cookie` optimization hint and a few rarely-used
/// transport fields are intentionally dropped; everything affecting audible
/// behaviour is preserved.)
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Note on / off / choke / end (`clap_event_note`).
    Note {
        kind: NoteEventKind,
        /// Host-assigned note id, or `-1` if unused.
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        velocity: f64,
    },
    /// Per-note expression (`clap_event_note_expression`).
    NoteExpression {
        expression: NoteExpression,
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        value: f64,
    },
    /// Parameter value change (`clap_event_param_value`).
    ParamValue {
        param_id: u32,
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        value: f64,
    },
    /// Parameter modulation (`clap_event_param_mod`).
    ParamMod {
        param_id: u32,
        note_id: i32,
        port_index: i16,
        channel: i16,
        key: i16,
        amount: f64,
    },
    /// Begin a parameter edit gesture (`clap_event_param_gesture`).
    ParamGestureBegin { param_id: u32 },
    /// End a parameter edit gesture.
    ParamGestureEnd { param_id: u32 },
    /// Transport / playhead update (`clap_event_transport`).
    Transport(Transport),
    /// Raw MIDI 1.0 message (`clap_event_midi`): status + up to two data bytes.
    Midi { port_index: u16, data: [u8; 3] },
    /// MIDI 2.0 / UMP packet (`clap_event_midi2`).
    Midi2 { port_index: u16, data: [u32; 4] },
    /// MIDI System Exclusive (`clap_event_midi_sysex`). The only variant that
    /// owns a heap buffer — sysex is rare on the audio path.
    Sysex { port_index: u16, data: Vec<u8> },
}

impl Event {
    /// A simple channel note-on (no note id, port 0).
    pub fn note_on(channel: i16, key: i16, velocity: f64) -> Self {
        Event::Note { kind: NoteEventKind::On, note_id: -1, port_index: 0, channel, key, velocity }
    }

    /// A simple channel note-off (no note id, port 0).
    pub fn note_off(channel: i16, key: i16, velocity: f64) -> Self {
        Event::Note { kind: NoteEventKind::Off, note_id: -1, port_index: 0, channel, key, velocity }
    }
}

/// An ordered list of events for one block. Reused across blocks via
/// [`clear`](Self::clear).
#[derive(Debug, Clone, Default)]
pub struct EventBuffer {
    events: Vec<TimedEvent>,
}

impl EventBuffer {
    /// Preallocate room for `capacity` events so steady-state pushes don't
    /// allocate on the audio path.
    pub fn with_capacity(capacity: usize) -> Self {
        Self { events: Vec::with_capacity(capacity) }
    }

    /// Append an event at sample offset `time`. Callers should push in
    /// non-decreasing `time` order (the engine drains MIDI in arrival order).
    pub fn push(&mut self, time: u32, event: Event) {
        self.events.push(TimedEvent { time, event });
    }

    /// Drop all events, keeping the allocation for reuse next block.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, TimedEvent> {
        self.events.iter()
    }
}

/// Everything a plugin needs to render one block: input audio (empty for an
/// instrument), the output audio to fill, the input events, and the block size.
pub struct ProcessBlock<'a> {
    /// Live frame count for this block (mirrors the buffers' `frames`).
    pub frames: usize,
    pub input: &'a AudioBuffer,
    pub output: &'a mut AudioBuffer,
    pub events: &'a EventBuffer,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_independent_and_clearable() {
        let mut buf = AudioBuffer::new(2, 4);
        assert_eq!(buf.channel_count(), 2);
        assert_eq!(buf.frames(), 4);

        buf.channel_mut(0).unwrap().fill(1.0);
        buf.channel_mut(1).unwrap().fill(-1.0);
        assert_eq!(buf.channel(0).unwrap(), &[1.0, 1.0, 1.0, 1.0]);
        assert_eq!(buf.channel(1).unwrap(), &[-1.0, -1.0, -1.0, -1.0]);
        assert!(buf.channel(2).is_none());

        buf.clear();
        assert_eq!(buf.channel(0).unwrap(), &[0.0; 4]);
    }

    #[test]
    fn set_frames_shrinks_the_live_region_without_reallocating() {
        let mut buf = AudioBuffer::new(1, 8);
        assert_eq!(buf.capacity(), 8);
        buf.set_frames(3);
        assert_eq!(buf.frames(), 3);
        assert_eq!(buf.channel(0).unwrap().len(), 3);
        assert_eq!(buf.capacity(), 8); // still allocated for the max block
    }

    #[test]
    #[should_panic]
    fn set_frames_beyond_capacity_panics() {
        let mut buf = AudioBuffer::new(1, 4);
        buf.set_frames(5);
    }

    #[test]
    fn empty_buffer_has_no_channels() {
        let buf = AudioBuffer::empty();
        assert_eq!(buf.channel_count(), 0);
        assert!(buf.channel(0).is_none());
    }

    #[test]
    fn event_buffer_preserves_push_order_and_clears() {
        let mut events = EventBuffer::with_capacity(8);
        events.push(0, Event::note_on(0, 60, 1.0));
        events.push(128, Event::Midi { port_index: 0, data: [0xB0, 7, 100] });
        events.push(256, Event::note_off(0, 60, 0.0));

        let collected: Vec<_> = events.iter().cloned().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0].time, 0);
        assert_eq!(collected[0].event, Event::note_on(0, 60, 1.0));
        assert_eq!(collected[1].event, Event::Midi { port_index: 0, data: [0xB0, 7, 100] });

        events.clear();
        assert!(events.is_empty());
    }
}

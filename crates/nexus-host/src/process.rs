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
///
/// Ordered and hashable so a renderer can key one lane per
/// `(dimension, channel, note)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
/// `clap_event_transport`). Send one as [`Event::Transport`] whenever it
/// changes — a start, a stop, a tempo change, a jump; between events the
/// backends advance the position with the audio ([`TransportClock`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub loop_active: bool,
    pub tempo_bpm: f64,
    /// Song position, in beats (quarter notes, as in CLAP and VST3).
    pub song_pos_beats: f64,
    /// Song position, in seconds.
    pub song_pos_seconds: f64,
    pub time_sig_numerator: u16,
    pub time_sig_denominator: u16,
}

impl Transport {
    /// The current bar: `(bar number from 0, its start in beats)`. A bar is
    /// `numerator · 4 / denominator` quarter notes (4/4 if the signature is
    /// unset).
    pub fn bar(&self) -> (i32, f64) {
        let (num, den) = match (self.time_sig_numerator, self.time_sig_denominator) {
            (0, _) | (_, 0) => (4, 4),
            sig => sig,
        };
        let bar_len = f64::from(num) * 4.0 / f64::from(den);
        let bar = (self.song_pos_beats / bar_len).floor();
        (bar as i32, bar * bar_len)
    }
}

/// The transport each block starts at, for a backend to hand its plugin.
///
/// The newest [`Event::Transport`] in a block replaces the clock's state; a
/// block without one continues from the last, moved on by the audio since
/// while playing. Before the first event there is no transport (a
/// free-running host). Realtime-safe.
#[derive(Debug, Clone, Copy)]
pub struct TransportClock {
    sample_rate: f64,
    current: Option<Transport>,
}

impl TransportClock {
    pub fn new(sample_rate: f64) -> Self {
        Self { sample_rate, current: None }
    }

    /// The transport at the first sample of a block of `frames` frames
    /// carrying `events`; the clock then stands at the block's end.
    pub fn block(&mut self, events: &EventBuffer, frames: usize) -> Option<Transport> {
        let newest = events.iter().rev().find_map(|e| match e.event {
            Event::Transport(t) => Some(t),
            _ => None,
        });
        if newest.is_some() {
            self.current = newest;
        }
        let now = self.current?;
        if now.playing && self.sample_rate > 0.0 {
            let seconds = frames as f64 / self.sample_rate;
            let next = self.current.as_mut().expect("just read");
            next.song_pos_seconds += seconds;
            next.song_pos_beats += seconds * now.tempo_bpm / 60.0;
        }
        Some(now)
    }
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

    fn transport(playing: bool, beats: f64) -> Transport {
        Transport {
            playing,
            loop_active: false,
            tempo_bpm: 120.0,
            song_pos_beats: beats,
            song_pos_seconds: beats / 2.0,
            time_sig_numerator: 3,
            time_sig_denominator: 4,
        }
    }

    #[test]
    fn the_transport_clock_follows_events_and_advances_while_playing() {
        let mut clock = TransportClock::new(48_000.0);
        let mut events = EventBuffer::with_capacity(4);
        assert_eq!(clock.block(&events, 24_000), None, "free-running until told");

        events.push(0, Event::Transport(transport(true, 4.0)));
        assert_eq!(clock.block(&events, 24_000).unwrap().song_pos_beats, 4.0);
        events.clear();
        // Half a second at 120 BPM is one beat.
        let next = clock.block(&events, 24_000).unwrap();
        assert_eq!((next.song_pos_beats, next.song_pos_seconds), (5.0, 2.5));

        events.push(0, Event::Transport(transport(false, 9.0)));
        assert_eq!(clock.block(&events, 24_000).unwrap().song_pos_beats, 9.0);
        events.clear();
        assert_eq!(clock.block(&events, 24_000).unwrap().song_pos_beats, 9.0, "stopped: holds");
    }

    #[test]
    fn a_bar_is_numerator_quarters_scaled_by_the_denominator() {
        assert_eq!(transport(true, 7.5).bar(), (2, 6.0)); // 3/4: bars of 3 beats
        let mut six_eight = transport(true, 7.5);
        (six_eight.time_sig_numerator, six_eight.time_sig_denominator) = (6, 8);
        assert_eq!(six_eight.bar(), (2, 6.0)); // 6/8: bars of 3 quarters
        six_eight.time_sig_denominator = 0;
        assert_eq!(six_eight.bar(), (1, 4.0)); // unset: 4/4
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

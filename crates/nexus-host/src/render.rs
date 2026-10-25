//! Offline plugin audio rendering engine.
//!
//! Executes a [`PluginInstance`] / [`AudioProcessor`] at maximum CPU speed without
//! requiring an active hardware audio device or sound driver.

use std::collections::BTreeMap;
use std::io::Cursor;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fugue::{CompiledFugue, FugueEvent};
use crate::host::{AudioConfig, HostError, ParamChange, PluginInstance};
use crate::process::{
    AudioBuffer, Event, EventBuffer, NoteEventKind, NoteExpression, ProcessBlock, Transport,
};
use simply_fugue::types::{InterpolationMode, LoopMode};

/// The fixed release tail appended to every offline render, in seconds.
///
/// A render used to stop exactly at `duration_beats`, so a note ending on the
/// last beat had its note-off at the final sample and its release was cut —
/// audible on any patch with a decay, and it skews the peak and RMS figures the
/// analysis loop reads (FUGUE-API.md §4.2, ruled in §6.2). One second, **fixed**
/// and stated in the tool description, because an invisible tail is a second
/// thing that makes two renders of the same fugue differ. `frames` and
/// `duration_s` include it.
pub const RELEASE_TAIL_S: f64 = 1.0;

/// Maximum allowed offline render length in seconds. Matches flux's policy (flux
/// refuses past 60 s) to guard against accidental multi-hundred-megabyte float WAVs (I-96).
pub const MAX_DURATION_S: f64 = 60.0;

/// The smallest block the renderer will run. A smaller `block_size` is raised
/// to this and the caller is told in `warnings`.
pub const MIN_BLOCK_FRAMES: usize = 64;

/// The automation grid, in frames.
///
/// Every lane kind — parameter, CC, per-note expression — is sampled at the
/// same instants, so a fugue whose lanes move together on paper moves together
/// in the plugin. Nothing downstream resamples: the offset travels through
/// `EventBuffer` and the parameter ring to the plugin unchanged.
pub const AUTOMATION_GRID_FRAMES: usize = 64;

/// Configuration options for an offline render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenderOptions {
    /// Sample rate in Hz (default: 48000.0).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// Maximum processing block size in frames (default: 512).
    #[serde(default = "default_block_size")]
    pub block_size: usize,
    /// Playback tempo in BPM (default: 120.0).
    #[serde(default = "default_tempo_bpm")]
    pub tempo_bpm: f64,
    /// Musical performance / automation to render.
    pub fugue: CompiledFugue,
}

fn default_sample_rate() -> f64 {
    48000.0
}

fn default_block_size() -> usize {
    512
}

fn default_tempo_bpm() -> f64 {
    120.0
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48000.0,
            block_size: 512,
            tempo_bpm: 120.0,
            fugue: crate::fugue::CompositeFugue::default().compile(),
        }
    }
}

/// The result of an offline render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenderResult {
    /// Rendered audio sample channels (typically stereo L/R).
    #[serde(skip)]
    pub channels: Vec<Vec<f32>>,
    /// Total rendered frames.
    pub frames: usize,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Total duration in seconds.
    pub duration_s: f64,
    /// Peak amplitude in dBFS (-inf to +0.0 or higher).
    pub peak_db: f64,
    /// RMS amplitude in dBFS.
    pub rms_db: f64,
    /// Whether the output contains only silence (peak < -96 dB).
    pub is_silent: bool,
    /// Standard 32-bit float WAV byte stream.
    #[serde(skip)]
    pub wav_bytes: Vec<u8>,
    /// What the renderer decided on the caller's behalf, in the caller's words:
    /// a block size it raised, the tail it added, an expression kind this
    /// plugin instance cannot be handed. Silence is the failure mode this
    /// exists to prevent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScheduledHostEvent {
    sample_offset: usize,
    /// The note-off-before-note-on tie-break, carried per scheduled event
    /// because one `TimedNote` becomes both edges. `false < true`, so
    /// `(sample_offset, is_note_on)` is the whole rule.
    is_note_on: bool,
    event: Event,
}

/// A keyframe on an automation timeline — a parameter, a CC lane, or a per-note
/// expression lane. All three are the same shape and are sampled on the same
/// grid, which is why there is one type and not three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    pub sample_offset: usize,
    pub value: f64,
    pub interpolation: InterpolationMode,
}

/// The interpolation curves, in one place, so no site can quietly disagree.
///
/// `Log` is **`1 − (1−t)²`** — the formula flux and droplets both use. Nexus
/// computed `t.sqrt()`, a different ease-out that agrees near the middle and
/// diverges at the ends, so identical JSON produced different audio on the two
/// hosts with no error at all (FUGUE-API.md §1.2a). Two of three agreed and
/// nexus was the outlier.
///
/// `None` means *stepped*: hold the previous value until the next keyframe.
pub fn curve_factor(mode: InterpolationMode, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match mode {
        InterpolationMode::None => 0.0,
        InterpolationMode::Linear => t,
        InterpolationMode::Exp => t * t,
        InterpolationMode::Log => 1.0 - (1.0 - t) * (1.0 - t),
    }
}

/// Evaluate an interpolated automation value at sample `s` from sorted keyframes.
pub fn evaluate_curve(keyframes: &[Keyframe], s: usize) -> Option<f64> {
    if keyframes.is_empty() {
        return None;
    }
    if s <= keyframes[0].sample_offset {
        return Some(keyframes[0].value);
    }
    if s >= keyframes.last().unwrap().sample_offset {
        return Some(keyframes.last().unwrap().value);
    }

    for i in 0..keyframes.len() - 1 {
        let k0 = &keyframes[i];
        let k1 = &keyframes[i + 1];
        if s >= k0.sample_offset && s <= k1.sample_offset {
            let span = k1.sample_offset - k0.sample_offset;
            if span == 0 {
                return Some(k1.value);
            }
            // A segment is eased using the curve of the keyframe arriving at
            // k1 (flux_contract::Point / droplets' rule, verbatim; flux I-92).
            let t = (s - k0.sample_offset) as f64 / span as f64;
            return Some(k0.value + (k1.value - k0.value) * curve_factor(k1.interpolation, t));
        }
    }
    Some(keyframes.last().unwrap().value)
}

/// Sample one automation lane across one block, on the shared grid.
///
/// Fills `out` with `(frame_in_block, value)` in time order: every keyframe
/// that falls inside the block, at its own offset, plus a grid point every
/// `step` frames once the lane has started, never two at one offset. The caller
/// decides what each point becomes — a `ParamChange`, a CC byte, a note
/// expression. Sharing this is what makes "the same grid" a fact rather than a
/// hope. `out` is passed in so the block loop reuses one allocation.
fn sample_lane(
    keyframes: &[Keyframe],
    current_frame: usize,
    block_frames: usize,
    step: usize,
    out: &mut Vec<(u32, f64)>,
) {
    out.clear();
    let block_end = current_frame + block_frames;
    for k in keyframes {
        if k.sample_offset >= current_frame && k.sample_offset < block_end {
            out.push(((k.sample_offset - current_frame) as u32, k.value));
        }
    }

    if let Some(first) = keyframes.first() {
        if current_frame >= first.sample_offset {
            for offset in (0..block_frames).step_by(step) {
                let offset = offset as u32;
                // A keyframe already speaks for its own instant. Evaluating the
                // curve there would answer with the *previous* value under a
                // stepped (`None`) lane, so the keyframe wins.
                if out.iter().any(|(o, _)| *o == offset) {
                    continue;
                }
                if let Some(value) = evaluate_curve(keyframes, current_frame + offset as usize) {
                    out.push((offset, value));
                }
            }
        }
    }

    out.sort_by_key(|(offset, _)| *offset);
}

/// A count of event kinds a fugue asked for, for a warning that names both the
/// kind and how much of it went nowhere.
#[derive(Debug, Default)]
struct Tally(BTreeMap<&'static str, usize>);

impl Tally {
    fn add(&mut self, kind: &'static str) {
        *self.0.entry(kind).or_default() += 1;
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `"1 channel pitch bend, 12 control change"`.
    fn describe(&self) -> String {
        self.0
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Render `instance` offline according to `opts`.
pub fn render_offline(
    instance: &mut dyn PluginInstance,
    opts: &RenderOptions,
) -> Result<RenderResult, HostError> {
    let mut warnings: Vec<String> = Vec::new();

    let sample_rate = opts.sample_rate;
    let block_size = opts.block_size.max(MIN_BLOCK_FRAMES);
    if block_size != opts.block_size {
        warnings.push(format!(
            "block_size {} raised to {MIN_BLOCK_FRAMES}, the smallest block this renderer runs",
            opts.block_size
        ));
    }
    let tempo = opts.tempo_bpm.max(1.0);
    let beats_per_second = tempo / 60.0;
    let samples_per_beat = sample_rate / beats_per_second;

    // `loop_mode`, offline (FUGUE-API.md §4.6). An endless loop has no length,
    // so there is nothing to render and guessing one would be its own silent
    // divergence — say so instead.
    let passes: u32 = match opts.fugue.loop_mode {
        LoopMode::Once => 1,
        LoopMode::Times(0) => {
            // Flux renders `{"times": 0}` as one pass (`n.max(1)`), and the two
            // hosts must not answer the same JSON with different lengths. A
            // decision made for the caller, so it is said out loud.
            warnings.push(
                "loop_mode {\"times\": 0} has no pass to render; rendered one pass, as flux does"
                    .into(),
            );
            1
        }
        LoopMode::Times(n) => n,
        LoopMode::Loop | LoopMode::PingPong => {
            let spelling = match opts.fugue.loop_mode {
                LoopMode::PingPong => "ping_pong",
                _ => "loop",
            };
            return Err(HostError::InvalidRequest(format!(
                "loop_mode \"{spelling}\" is endless and has no length to render offline; it \
                 applies to live playback (the `play` tool). Render a fixed number of passes \
                 with {{\"times\": n}}."
            )));
        }
    };

    let pass_beats = opts.fugue.duration_beats;
    let pass_frames = (pass_beats * samples_per_beat).round() as usize;
    let musical_frames = (pass_frames * passes as usize).max(block_size);
    // The tail is applied ONCE, after the last pass — not per pass.
    let tail_frames = (RELEASE_TAIL_S * sample_rate).round() as usize;
    let total_frames = musical_frames + tail_frames;
    let duration_s = total_frames as f64 / sample_rate;

    if duration_s > MAX_DURATION_S {
        return Err(HostError::InvalidRequest(format!(
            "render duration ({duration_s:.1} s) exceeds the maximum allowed offline length ({MAX_DURATION_S:.0} s); \
             shorten the fugue or reduce loop passes"
        )));
    }

    warnings.push(format!(
        "rendered {pass_beats} beat(s){} at {tempo} BPM plus a fixed {RELEASE_TAIL_S:.1} s \
         release tail; frames and duration_s include the tail",
        if passes > 1 { format!(" × {passes} passes") } else { String::new() }
    ));

    // 1. Compile timed musical events and every automation lane.
    //
    // The match below is **exhaustive on purpose**. It used to end in `_ => {}`,
    // which silently swallowed five event kinds nexus parses and schematizes
    // (FUGUE-API.md §1.2c); an exhaustive match makes the next new variant a
    // compile error instead of a silence.
    let mut scheduled: Vec<ScheduledHostEvent> = Vec::new();
    let mut param_tracks: BTreeMap<u32, Vec<Keyframe>> = BTreeMap::new();
    let mut cc_tracks: BTreeMap<(u8, u8), Vec<Keyframe>> = BTreeMap::new();
    let mut expr_tracks: BTreeMap<(NoteExpression, u8, u8), Vec<Keyframe>> = BTreeMap::new();
    let mut raw_midi_tally = Tally::default();
    let mut expression_tally = Tally::default();

    for pass in 0..passes {
        let pass_offset_beats = pass as f64 * pass_beats;

        for timed in &opts.fugue.events {
            let beat = timed.beat_offset + pass_offset_beats;
            let onset_sample = (beat * samples_per_beat).round() as usize;

            match &timed.event {
                FugueEvent::NoteOn { channel, note, velocity } => {
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: true,
                        event: Event::Note {
                            kind: NoteEventKind::On,
                            note_id: -1,
                            port_index: 0,
                            channel: *channel as i16,
                            key: *note as i16,
                            velocity: *velocity as f64 / 127.0,
                        },
                    });
                }
                FugueEvent::NoteOff { channel, note, .. } => {
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: false,
                        event: Event::Note {
                            kind: NoteEventKind::Off,
                            note_id: -1,
                            port_index: 0,
                            channel: *channel as i16,
                            key: *note as i16,
                            velocity: 0.0,
                        },
                    });
                }
                FugueEvent::TimedNote { channel, note, velocity, duration_beats } => {
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: true,
                        event: Event::Note {
                            kind: NoteEventKind::On,
                            note_id: -1,
                            port_index: 0,
                            channel: *channel as i16,
                            key: *note as i16,
                            velocity: *velocity as f64 / 127.0,
                        },
                    });

                    let off_sample = ((beat + duration_beats) * samples_per_beat).round() as usize;
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: off_sample,
                        is_note_on: false,
                        event: Event::Note {
                            kind: NoteEventKind::Off,
                            note_id: -1,
                            port_index: 0,
                            channel: *channel as i16,
                            key: *note as i16,
                            velocity: 0.0,
                        },
                    });
                }
                FugueEvent::Cc { channel, controller, value, interpolation } => {
                    raw_midi_tally.add("control change");
                    cc_tracks.entry((*channel, *controller)).or_default().push(Keyframe {
                        sample_offset: onset_sample,
                        value: *value as f64,
                        interpolation: *interpolation,
                    });
                }
                FugueEvent::Param { id, value, interpolation } => {
                    param_tracks.entry(*id).or_default().push(Keyframe {
                        sample_offset: onset_sample,
                        value: *value as f64,
                        interpolation: *interpolation,
                    });
                }
                FugueEvent::PitchBend { channel, value } => {
                    // The wire carries a signed offset, -8192..=8191, resting at
                    // 0; MIDI carries 0..=16383 with 8192 as the rest, LSB then
                    // MSB, seven bits each.
                    raw_midi_tally.add("channel pitch bend");
                    let bend = (*value as i32 + 8192).clamp(0, 16383) as u16;
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: false,
                        event: Event::Midi {
                            port_index: 0,
                            data: [
                                0xE0 | (channel & 0x0F),
                                (bend & 0x7F) as u8,
                                ((bend >> 7) & 0x7F) as u8,
                            ],
                        },
                    });
                }
                FugueEvent::Pressure { channel, value } => {
                    raw_midi_tally.add("channel pressure");
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: false,
                        event: Event::Midi {
                            port_index: 0,
                            data: [0xD0 | (channel & 0x0F), *value, 0],
                        },
                    });
                }
                FugueEvent::PolyPressure { channel, note, value } => {
                    raw_midi_tally.add("poly pressure");
                    scheduled.push(ScheduledHostEvent {
                        sample_offset: onset_sample,
                        is_note_on: false,
                        event: Event::Midi {
                            port_index: 0,
                            data: [0xA0 | (channel & 0x0F), *note, *value],
                        },
                    });
                }
                // Per-note lanes arrive from `compile()` as sparse points with a
                // curve, exactly like a param lane, so they are interpolated on
                // the same grid. CLAP's `Tuning` is relative semitones
                // (-120..=120) and `Pressure` is 0..=1 — the fugue's units
                // already, so nothing is scaled here.
                FugueEvent::PerNotePitchBend { channel, note, semitones, interpolation } => {
                    expression_tally.add("per-note pitch bend");
                    expr_tracks
                        .entry((NoteExpression::Tuning, *channel, *note))
                        .or_default()
                        .push(Keyframe {
                            sample_offset: onset_sample,
                            value: *semitones as f64,
                            interpolation: *interpolation,
                        });
                }
                FugueEvent::PerNotePressure { channel, note, pressure, interpolation } => {
                    expression_tally.add("per-note pressure");
                    expr_tracks
                        .entry((NoteExpression::Pressure, *channel, *note))
                        .or_default()
                        .push(Keyframe {
                            sample_offset: onset_sample,
                            value: *pressure as f64,
                            interpolation: *interpolation,
                        });
                }
            }
        }
    }

    // Chronological, note-offs before note-ons at the same sample — regardless
    // of what `compile()` did, because a same-pitch retrigger at a note boundary
    // otherwise starts the second note and immediately kills it.
    scheduled.sort_by_key(|s| (s.sample_offset, s.is_note_on));
    for keyframes in param_tracks.values_mut() {
        keyframes.sort_by_key(|k| k.sample_offset);
    }
    for keyframes in cc_tracks.values_mut() {
        keyframes.sort_by_key(|k| k.sample_offset);
    }
    for keyframes in expr_tracks.values_mut() {
        keyframes.sort_by_key(|k| k.sample_offset);
    }

    // What this instance cannot carry, said out loud rather than dropped.
    let support = instance.event_support();
    if !support.raw_midi && !raw_midi_tally.is_empty() {
        warnings.push(format!(
            "this plugin instance takes note on/off only, so {} event(s) will not be heard \
             (the VST3 backend converts no other raw MIDI)",
            raw_midi_tally.describe()
        ));
    }
    if !support.note_expression && !expression_tally.is_empty() {
        warnings.push(format!(
            "this plugin instance accepts no per-note expression, so {} lane point(s) will \
             not be heard",
            expression_tally.describe()
        ));
    }

    // 2. Activate plugin
    let config = AudioConfig {
        sample_rate,
        max_block_frames: block_size,
    };
    let mut processor = instance.activate(config)?;
    processor.start_processing()?;

    // 3. Allocate buffers
    let input_audio = AudioBuffer::empty();
    let channel_count = 2; // Stereo output
    let mut output_audio = AudioBuffer::new(channel_count, block_size);
    let mut input_events = EventBuffer::with_capacity(64);

    let mut rendered_channels = vec![Vec::with_capacity(total_frames); channel_count];

    let mut current_frame = 0;
    let mut event_idx = 0;

    // One block's worth of events, with the tie-break key, before they are put
    // in time order and handed over.
    let mut block_events: Vec<(u32, bool, Event)> = Vec::new();
    let mut block_params: Vec<ParamChange> = Vec::new();
    let mut lane_points: Vec<(u32, f64)> = Vec::new();
    // The last value actually sent per param, CC, and per expression lane, so that a
    // held value is not re-sent at every grid point — which is also what keeps
    // a curve-less lane exactly one instantaneous byte, as it has always been.
    let mut last_param: BTreeMap<u32, f64> = BTreeMap::new();
    let mut last_cc: BTreeMap<(u8, u8), u8> = BTreeMap::new();
    let mut last_expression: BTreeMap<(NoteExpression, u8, u8), f64> = BTreeMap::new();

    while current_frame < total_frames {
        let block_frames = (total_frames - current_frame).min(block_size);
        output_audio.set_frames(block_frames);
        output_audio.clear();
        input_events.clear();
        block_events.clear();
        block_params.clear();

        let block_end = current_frame + block_frames;
        let step = AUTOMATION_GRID_FRAMES.min(block_frames).max(1);

        // Musical note / raw MIDI events falling into this block
        while event_idx < scheduled.len() && scheduled[event_idx].sample_offset < block_end {
            let ev = &scheduled[event_idx];
            if ev.sample_offset >= current_frame {
                let frame_in_block = (ev.sample_offset - current_frame) as u32;
                block_events.push((frame_in_block, ev.is_note_on, ev.event.clone()));
            }
            event_idx += 1;
        }

        // Parameter lanes, on the shared grid. Held values are not re-sent (I-95).
        for (&param_id, keyframes) in &param_tracks {
            sample_lane(keyframes, current_frame, block_frames, step, &mut lane_points);
            for &(offset_frames, value) in &lane_points {
                if last_param.get(&param_id) == Some(&value) {
                    continue;
                }
                last_param.insert(param_id, value);
                block_params.push(ParamChange { id: param_id, value, offset_frames });
            }
        }
        block_params.sort_by_key(|p| p.offset_frames);

        // Deliver parameter changes: to the rtrb ring buffer if the instance provides
        // one, or to the facade event list if not. Pushing to both caused CLAP plugins
        // to receive every parameter change twice (I-94).
        for change in &block_params {
            if let Some(producer) = instance.param_producer() {
                let _ = producer.push(*change);
            } else {
                block_events.push((
                    change.offset_frames,
                    false,
                    Event::ParamValue {
                        param_id: change.id,
                        note_id: -1,
                        port_index: 0,
                        channel: 0,
                        key: 0,
                        value: change.value,
                    },
                ));
            }
        }

        // CC lanes, on the same grid, rounded to a byte. `None` (the default)
        // holds its value, so every grid point after the keyframe repeats it and
        // is dropped: curve-less JSON is still one byte per point, exactly as
        // before. A curve becomes a ramp of distinct bytes.
        for (&(channel, controller), keyframes) in &cc_tracks {
            sample_lane(keyframes, current_frame, block_frames, step, &mut lane_points);
            for &(offset, value) in &lane_points {
                let byte = value.round().clamp(0.0, 127.0) as u8;
                if last_cc.get(&(channel, controller)) == Some(&byte) {
                    continue;
                }
                last_cc.insert((channel, controller), byte);
                block_events.push((
                    offset,
                    false,
                    Event::Midi {
                        port_index: 0,
                        data: [0xB0 | (channel & 0x0F), controller, byte],
                    },
                ));
            }
        }

        // Per-note expression lanes, same grid, same held-value rule.
        for (&key, keyframes) in &expr_tracks {
            let (expression, channel, note) = key;
            sample_lane(keyframes, current_frame, block_frames, step, &mut lane_points);
            for &(offset, value) in &lane_points {
                if last_expression.get(&key) == Some(&value) {
                    continue;
                }
                last_expression.insert(key, value);
                block_events.push((
                    offset,
                    false,
                    Event::NoteExpression {
                        expression,
                        note_id: -1,
                        port_index: 0,
                        channel: channel as i16,
                        key: note as i16,
                        value,
                    },
                ));
            }
        }

        // One list, in time order, note-offs and controls ahead of a note-on at
        // the same frame. CLAP specifies its input event list as time-ordered
        // and this buffer was not: every note went in ahead of every parameter
        // change, whatever their frames said.
        block_events.sort_by_key(|(offset, is_note_on, _)| (*offset, *is_note_on));
        if current_frame == 0 {
            // Playing from the top at the fugue's tempo, so tempo-synced
            // plugins follow it; the backends advance it from here (`I-104`).
            input_events.push(
                0,
                Event::Transport(Transport {
                    playing: true,
                    loop_active: false,
                    tempo_bpm: tempo,
                    song_pos_beats: 0.0,
                    song_pos_seconds: 0.0,
                    time_sig_numerator: 4,
                    time_sig_denominator: 4,
                }),
            );
        }
        for (offset, _, event) in block_events.drain(..) {
            input_events.push(offset, event);
        }

        let mut block = ProcessBlock {
            frames: block_frames,
            input: &input_audio,
            output: &mut output_audio,
            events: &input_events,
        };

        processor.process(&mut block);

        for ch in 0..channel_count {
            if let Some(samples) = output_audio.channel(ch) {
                rendered_channels[ch].extend_from_slice(samples);
            }
        }

        current_frame += block_frames;
    }

    processor.stop_processing();
    drop(processor);
    instance.deactivate();

    // 4. Compute audio telemetry (peak and RMS)
    let mut peak_sq = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut total_samples = 0usize;

    for ch in &rendered_channels {
        for &s in ch {
            let abs_s = s.abs();
            if abs_s > peak_sq {
                peak_sq = abs_s;
            }
            sum_sq += (s as f64) * (s as f64);
            total_samples += 1;
        }
    }

    let peak = peak_sq as f64;
    let peak_db = if peak > 1e-9 {
        20.0 * peak.log10()
    } else {
        -120.0
    };

    let rms = if total_samples > 0 {
        (sum_sq / total_samples as f64).sqrt()
    } else {
        0.0
    };
    let rms_db = if rms > 1e-9 {
        20.0 * rms.log10()
    } else {
        -120.0
    };

    let is_silent = peak_db < -96.0;

    // 5. Encode 32-bit float WAV bytes
    let wav_bytes = encode_wav_f32(&rendered_channels, sample_rate as u32)?;

    Ok(RenderResult {
        channels: rendered_channels,
        frames: total_frames,
        sample_rate: sample_rate as u32,
        duration_s,
        peak_db,
        rms_db,
        is_silent,
        wav_bytes,
        warnings,
    })
}

/// Encode planar float audio channels into 32-bit float WAV format.
pub fn encode_wav_f32(channels: &[Vec<f32>], sample_rate: u32) -> Result<Vec<u8>, HostError> {
    let channel_count = channels.len() as u16;
    if channel_count == 0 {
        return Err(HostError::Load("cannot encode WAV with 0 channels".into()));
    }

    let frame_count = channels[0].len();
    let spec = hound::WavSpec {
        channels: channel_count,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut cursor = Cursor::new(Vec::with_capacity(44 + frame_count * channel_count as usize * 4));
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| HostError::Load(format!("failed to initialize WAV writer: {e}")))?;

        for f in 0..frame_count {
            for ch in 0..channel_count as usize {
                let sample = channels[ch].get(f).copied().unwrap_or(0.0);
                writer
                    .write_sample(sample)
                    .map_err(|e| HostError::Load(format!("failed to write WAV sample: {e}")))?;
            }
        }

        writer
            .finalize()
            .map_err(|e| HostError::Load(format!("failed to finalize WAV: {e}")))?;
    }

    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::fugue::CompositeFugue;
    use crate::host::{AudioProcessor, EventSupport};

    struct DummyOscillatorInstance {
        active: bool,
    }

    impl PluginInstance for DummyOscillatorInstance {
        fn activate(&mut self, _config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
            self.active = true;
            Ok(Box::new(DummyOscillatorProcessor { phase: 0.0 }))
        }

        fn deactivate(&mut self) {
            self.active = false;
        }
    }

    struct DummyOscillatorProcessor {
        phase: f32,
    }

    impl AudioProcessor for DummyOscillatorProcessor {
        fn start_processing(&mut self) -> Result<(), HostError> {
            Ok(())
        }

        fn stop_processing(&mut self) {}

        fn process(&mut self, block: &mut ProcessBlock<'_>) {
            let frames = block.output.frames();
            for f in 0..frames {
                let sample = (self.phase * std::f32::consts::TAU).sin() * 0.5;
                self.phase = (self.phase + 440.0 / 48000.0).fract();
                if let Some(left) = block.output.channel_mut(0) {
                    left[f] = sample;
                }
                if let Some(right) = block.output.channel_mut(1) {
                    right[f] = sample;
                }
            }
        }
    }

    #[test]
    fn render_offline_sine_wave() {
        let mut inst = DummyOscillatorInstance { active: false };
        let fugue = CompositeFugue {
            duration_beats: Some(2.0),
            ..Default::default()
        }
        .compile();

        let opts = RenderOptions {
            sample_rate: 48000.0,
            block_size: 256,
            tempo_bpm: 120.0,
            fugue,
        };

        let result = render_offline(&mut inst, &opts).unwrap();
        assert_eq!(result.sample_rate, 48000);
        assert_eq!(result.channels.len(), 2);
        assert_eq!(result.frames, 48000 + 48000); // 2 beats at 120 bpm = 1 s, plus the 1 s tail
        assert!(!result.is_silent);
        assert!(result.peak_db > -7.0 && result.peak_db < -5.0); // ~0.5 amplitude ≈ -6 dBFS
        assert!(!result.wav_bytes.is_empty());
        assert_eq!(&result.wav_bytes[0..4], b"RIFF");
    }

    #[test]
    fn evaluate_curve_holds_before_the_first_point_and_after_the_last() {
        let keyframes = vec![
            Keyframe {
                sample_offset: 100,
                value: 0.0,
                interpolation: InterpolationMode::Linear,
            },
            Keyframe {
                sample_offset: 200,
                value: 10.0,
                interpolation: InterpolationMode::Exp,
            },
            Keyframe {
                sample_offset: 300,
                value: 20.0,
                interpolation: InterpolationMode::None,
            },
        ];

        // Before first point
        assert_eq!(evaluate_curve(&keyframes, 50), Some(0.0));
        // At first point
        assert_eq!(evaluate_curve(&keyframes, 100), Some(0.0));
        // Midpoint of segment arriving at keyframes[1] (Exp: t=0.5 -> t^2=0.25 -> 0 + 10*0.25 = 2.5)
        assert_eq!(evaluate_curve(&keyframes, 150), Some(2.5));
        // At second point
        assert_eq!(evaluate_curve(&keyframes, 200), Some(10.0));
        // Midpoint of segment arriving at keyframes[2] (None: stepped holds k0.value = 10.0)
        assert_eq!(evaluate_curve(&keyframes, 250), Some(10.0));
        // At third point
        assert_eq!(evaluate_curve(&keyframes, 300), Some(20.0));
        // After last point
        assert_eq!(evaluate_curve(&keyframes, 400), Some(20.0));
    }

    struct DummyParamInstance {
        param_producer: Option<rtrb::Producer<ParamChange>>,
    }

    impl PluginInstance for DummyParamInstance {
        fn activate(&mut self, _config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
            let (producer, consumer) = rtrb::RingBuffer::new(1024);
            self.param_producer = Some(producer);
            Ok(Box::new(DummyParamProcessor {
                consumer,
                current_gain: 0.0,
            }))
        }

        fn deactivate(&mut self) {
            self.param_producer = None;
        }

        fn param_producer(&mut self) -> Option<&mut rtrb::Producer<ParamChange>> {
            self.param_producer.as_mut()
        }

        fn take_param_producer(&mut self) -> Option<rtrb::Producer<ParamChange>> {
            self.param_producer.take()
        }
    }

    struct DummyParamProcessor {
        consumer: rtrb::Consumer<ParamChange>,
        current_gain: f64,
    }

    impl AudioProcessor for DummyParamProcessor {
        fn start_processing(&mut self) -> Result<(), HostError> {
            Ok(())
        }

        fn stop_processing(&mut self) {}

        fn process(&mut self, block: &mut ProcessBlock<'_>) {
            while let Ok(change) = self.consumer.pop() {
                if change.id == 1 {
                    self.current_gain = change.value;
                }
            }
            let frames = block.output.frames();
            for f in 0..frames {
                let sample = (self.current_gain as f32) * 0.8;
                if let Some(left) = block.output.channel_mut(0) {
                    left[f] = sample;
                }
                if let Some(right) = block.output.channel_mut(1) {
                    right[f] = sample;
                }
            }
        }
    }

    #[test]
    fn render_offline_with_param_curve_automation() {
        let mut inst = DummyParamInstance { param_producer: None };

        let mut params = std::collections::HashMap::new();
        params.insert(
            1,
            vec![
                simply_fugue::composite::ParamPointItem {
                    beat: 0.0,
                    value: 0.0,
                    curve: InterpolationMode::Linear,
                },
                simply_fugue::composite::ParamPointItem {
                    beat: 1.0,
                    value: 1.0,
                    curve: InterpolationMode::Linear,
                },
            ],
        );

        let fugue = CompositeFugue {
            duration_beats: Some(1.0),
            params,
            ..Default::default()
        }
        .compile();

        let opts = RenderOptions {
            sample_rate: 48000.0,
            block_size: 256,
            tempo_bpm: 120.0,
            fugue,
        };

        let result = render_offline(&mut inst, &opts).unwrap();
        assert_eq!(result.frames, 24000 + 48000); // 1 beat at 120 bpm = 0.5 s, plus the 1 s tail

        // The first block should have low amplitude (~0.0) and final blocks should reach ~0.8
        let first_sample = result.channels[0][0];
        let last_sample = result.channels[0][result.frames - 1];

        assert!(first_sample < 0.05, "expected starting gain near 0, got {first_sample}");
        assert!(last_sample > 0.75, "expected final gain near 0.8, got {last_sample}");
    }

    // --- the behaviour ruled in FUGUE-API.md §6 ------------------------------

    /// An instance that records exactly what its plugin was handed, so a test
    /// can assert that an event kind arrived and at which frame.
    struct RecordingInstance {
        support: EventSupport,
        log: Arc<Mutex<Vec<(usize, Event)>>>,
    }

    impl RecordingInstance {
        fn new(support: EventSupport) -> Self {
            Self { support, log: Arc::new(Mutex::new(Vec::new())) }
        }
    }

    impl PluginInstance for RecordingInstance {
        fn event_support(&self) -> EventSupport {
            self.support
        }

        fn activate(&mut self, _config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
            Ok(Box::new(RecordingProcessor { log: self.log.clone(), frame: 0 }))
        }

        fn deactivate(&mut self) {}
    }

    struct RecordingProcessor {
        log: Arc<Mutex<Vec<(usize, Event)>>>,
        frame: usize,
    }

    impl AudioProcessor for RecordingProcessor {
        fn start_processing(&mut self) -> Result<(), HostError> {
            Ok(())
        }

        fn stop_processing(&mut self) {}

        fn process(&mut self, block: &mut ProcessBlock<'_>) {
            let mut log = self.log.lock().unwrap();
            for timed in block.events.iter() {
                log.push((self.frame + timed.time as usize, timed.event.clone()));
            }
            self.frame += block.output.frames();
        }
    }

    /// Render `fugue` through a recorder and return every event the plugin saw,
    /// with its absolute frame.
    fn recorded(fugue: CompositeFugue, support: EventSupport) -> (Vec<(usize, Event)>, RenderResult) {
        let mut inst = RecordingInstance::new(support);
        let log = inst.log.clone();
        let result = render_offline(
            &mut inst,
            &RenderOptions { fugue: fugue.compile(), ..Default::default() },
        )
        .unwrap();
        let events = log.lock().unwrap().clone();
        (events, result)
    }

    fn everything() -> EventSupport {
        EventSupport { raw_midi: true, note_expression: true }
    }

    /// **`Log` is `1 − (1−t)²`, not `√t`** (flux `I-49`, FUGUE-API.md §1.2a).
    /// The two agree in direction and disagree at the ends; at the midpoint one
    /// says 0.75 and the other 0.707, which is the number that pins it.
    #[test]
    fn log_is_the_majority_curve_and_the_others_are_unchanged() {
        assert_eq!(curve_factor(InterpolationMode::Log, 0.5), 0.75);
        assert_ne!(curve_factor(InterpolationMode::Log, 0.5), 0.5f64.sqrt());
        assert_eq!(curve_factor(InterpolationMode::Exp, 0.5), 0.25);
        assert_eq!(curve_factor(InterpolationMode::Linear, 0.5), 0.5);
        // `None` is stepped: hold the previous value for the whole span.
        assert_eq!(curve_factor(InterpolationMode::None, 0.5), 0.0);
        // Both ends are pinned for every mode.
        for mode in [
            InterpolationMode::Linear,
            InterpolationMode::Exp,
            InterpolationMode::Log,
        ] {
            assert_eq!(curve_factor(mode, 0.0), 0.0);
            assert_eq!(curve_factor(mode, 1.0), 1.0);
        }
    }

    /// **A segment eases with the arriving point's curve, not the departing
    /// point's** (flux `I-92`). `[[0, 0, "linear"], [1, 1, "exp"]]` is an `exp`
    /// rise (midpoint 0.25), not linear (0.50).
    #[test]
    fn a_segment_eases_with_the_arriving_points_curve() {
        let keyframes = vec![
            Keyframe {
                sample_offset: 0,
                value: 0.0,
                interpolation: InterpolationMode::Linear,
            },
            Keyframe {
                sample_offset: 100,
                value: 1.0,
                interpolation: InterpolationMode::Exp,
            },
        ];

        // Midpoint: t=0.5 -> Exp arrival = 0.5^2 = 0.25. Linear departure would have been 0.50.
        assert_eq!(evaluate_curve(&keyframes, 50), Some(0.25));
    }

    /// **A CC lane with a curve becomes a ramp; a curve-less one stays a single
    /// byte** (flux `I-50`, §1.2c). The second half is what makes this
    /// backward-compatible: no fugue written before today gains an event.
    #[test]
    fn a_cc_curve_ramps_and_a_curve_less_lane_does_not() {
        let ramped: CompositeFugue =
            serde_json::from_str(r#"{"duration_beats":1,"ccs":{"74":[[0,0],[1,127,"linear"]]}}"#)
                .unwrap();
        let (events, _) = recorded(ramped, everything());
        let values: Vec<u8> = events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Midi { data, .. } if data[0] == 0xB0 && data[1] == 74 => Some(data[2]),
                _ => None,
            })
            .collect();

        assert!(values.len() > 8, "expected a ramp of CC bytes, got {values:?}");
        assert_eq!(values.first(), Some(&0));
        assert_eq!(values.last(), Some(&127));
        assert!(
            values.windows(2).all(|w| w[0] < w[1]),
            "the ramp must be strictly rising with no value re-sent: {values:?}"
        );

        // The same lane without a curve: two points, two bytes, as before.
        let stepped: CompositeFugue =
            serde_json::from_str(r#"{"duration_beats":1,"ccs":{"74":[[0,0],[1,127]]}}"#).unwrap();
        let (events, _) = recorded(stepped, everything());
        let values: Vec<u8> = events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Midi { data, .. } if data[0] == 0xB0 => Some(data[2]),
                _ => None,
            })
            .collect();
        assert_eq!(values, vec![0, 127], "a stepped lane must stay two bytes");
    }

    /// **The five kinds `_ => {}` used to swallow now reach the plugin**, each
    /// at its own frame (flux `I-48`).
    #[test]
    fn every_swallowed_event_kind_reaches_the_plugin() {
        // 1 beat at 120 BPM / 48 kHz = 24000 frames, so beat 1 is frame 24000.
        let fugue: CompositeFugue = serde_json::from_str(
            r#"{"duration_beats": 2,
                "raw_events": [
                  {"beat_offset":0, "event":{"type":"pitch_bend","channel":0,"value":4096}},
                  {"beat_offset":1, "event":{"type":"pressure","channel":0,"value":90}},
                  {"beat_offset":1, "event":{"type":"poly_pressure","channel":0,"note":60,"value":70}}
                ],
                "pitch_bends": [{"note":60,"points":[[0,1.5]]}],
                "pressures":   [{"note":60,"points":[[1,0.25]]}]}"#,
        )
        .unwrap();

        let (events, result) = recorded(fugue, everything());
        assert!(result.warnings.iter().all(|w| !w.contains("will not be heard")));

        // Channel pitch bend: -8192..=8191 on the wire, 0..=16383 on the wire
        // MIDI carries, so +4096 is 12288 = LSB 0, MSB 96.
        assert!(
            events.contains(&(0, Event::Midi { port_index: 0, data: [0xE0, 0, 96] })),
            "channel pitch bend at frame 0: {events:?}"
        );
        assert!(
            events.contains(&(24000, Event::Midi { port_index: 0, data: [0xD0, 90, 0] })),
            "channel pressure at frame 24000"
        );
        assert!(
            events.contains(&(24000, Event::Midi { port_index: 0, data: [0xA0, 60, 70] })),
            "poly pressure at frame 24000"
        );
        assert!(
            events.contains(&(
                0,
                Event::NoteExpression {
                    expression: NoteExpression::Tuning,
                    note_id: -1,
                    port_index: 0,
                    channel: 0,
                    key: 60,
                    value: 1.5,
                }
            )),
            "per-note pitch bend as a Tuning expression, in semitones, at frame 0"
        );
        assert!(
            events.contains(&(
                24000,
                Event::NoteExpression {
                    expression: NoteExpression::Pressure,
                    note_id: -1,
                    port_index: 0,
                    channel: 0,
                    key: 60,
                    value: 0.25,
                }
            )),
            "per-note pressure as a Pressure expression, 0..1, at frame 24000"
        );
    }

    /// Multi-channel CC lanes (`I-53`): a lane keyed by `"channel:controller"`
    /// (e.g. `"2:74"`) delivers CC messages on that MIDI channel (`0xB2`).
    #[test]
    fn multi_channel_cc_lane_delivers_on_specified_channel() {
        let fugue: CompositeFugue = serde_json::from_str(
            r#"{"duration_beats": 1,
                "ccs": {
                    "74": [[0, 10]],
                    "2:74": [[0, 50]]
                }}"#,
        )
        .unwrap();

        let (events, _) = recorded(fugue, everything());
        assert!(
            events.contains(&(0, Event::Midi { port_index: 0, data: [0xB0, 74, 10] })),
            "channel 0 CC 74 at frame 0: {events:?}"
        );
        assert!(
            events.contains(&(0, Event::Midi { port_index: 0, data: [0xB2, 74, 50] })),
            "channel 2 CC 74 at frame 0: {events:?}"
        );
    }

    /// A host path that cannot carry a kind **says so**, with the kind and the
    /// count. That is the whole point of `I-48`'s second half.
    #[test]
    fn an_instance_that_cannot_carry_a_kind_warns_naming_it() {
        let fugue: CompositeFugue = serde_json::from_str(
            r#"{"duration_beats":1,
                "ccs":{"74":[[0,10]]},
                "raw_events":[{"beat_offset":0,"event":{"type":"pitch_bend","channel":0,"value":0}}],
                "pressures":[{"note":60,"points":[[0,0.5]]}]}"#,
        )
        .unwrap();

        // The default `EventSupport` is the VST3 answer: notes only.
        let (_, result) = recorded(fugue, EventSupport::default());
        let joined = result.warnings.join(" | ");
        assert!(joined.contains("1 channel pitch bend"), "{joined}");
        assert!(joined.contains("1 control change"), "{joined}");
        assert!(joined.contains("1 per-note pressure"), "{joined}");
    }

    /// **A note-off precedes a note-on at the same sample**, whatever order the
    /// events arrived in (flux `I-54`, nexus side).
    #[test]
    fn a_same_pitch_retrigger_delivers_the_off_before_the_on() {
        // Two abutting notes on one pitch: the first note's off and the second
        // note's on both land on beat 1.
        let fugue: CompositeFugue =
            serde_json::from_str(r#"{"notes":[[0,"C3",1.0,100],[1,"C3",1.0,100]]}"#).unwrap();
        let (events, _) = recorded(fugue, everything());

        let edges: Vec<(usize, bool)> = events
            .iter()
            .filter_map(|(frame, e)| match e {
                Event::Note { kind: NoteEventKind::On, .. } => Some((*frame, true)),
                Event::Note { kind: NoteEventKind::Off, .. } => Some((*frame, false)),
                _ => None,
            })
            .collect();

        assert_eq!(
            edges,
            vec![(0, true), (24000, false), (24000, true), (48000, false)],
            "off before on at frame 24000"
        );
    }

    /// **A fixed 1.0 s tail, once**, however many passes (§6.2), and it is in
    /// `frames` and `duration_s`.
    #[test]
    fn the_release_tail_is_one_second_applied_once() {
        let tail = (RELEASE_TAIL_S * 48000.0) as usize;

        let once = CompositeFugue { duration_beats: Some(2.0), ..Default::default() };
        let (_, result) = recorded(once.clone(), everything());
        assert_eq!(result.frames, 48000 + tail);
        assert_eq!(result.duration_s, (48000 + tail) as f64 / 48000.0);
        assert!(result.warnings.iter().any(|w| w.contains("release tail")));

        // Twice as long musically, still one tail.
        let twice = CompositeFugue { loop_mode: LoopMode::Times(2), ..once };
        let (_, result) = recorded(twice, everything());
        assert_eq!(result.frames, 2 * 48000 + tail);
    }

    /// `Times(n)` re-emits the whole event set at `k · duration_beats` (§4.6).
    #[test]
    fn times_renders_every_pass() {
        let fugue = CompositeFugue {
            notes: vec![serde_json::from_str(r#"[0,"C3",0.5,100]"#).unwrap()],
            duration_beats: Some(1.0),
            loop_mode: LoopMode::Times(3),
            ..Default::default()
        };
        let (events, _) = recorded(fugue, everything());
        let ons: Vec<usize> = events
            .iter()
            .filter(|(_, e)| matches!(e, Event::Note { kind: NoteEventKind::On, .. }))
            .map(|(frame, _)| *frame)
            .collect();
        assert_eq!(ons, vec![0, 24000, 48000], "one note-on per pass, a beat apart");
    }

    /// `{"times": 0}` is one pass, as it is on flux, and the caller is told.
    #[test]
    fn times_zero_renders_one_pass_and_says_so() {
        let fugue = CompositeFugue {
            notes: vec![serde_json::from_str(r#"[0,"C3",0.5,100]"#).unwrap()],
            duration_beats: Some(1.0),
            loop_mode: LoopMode::Times(0),
            ..Default::default()
        };
        let (events, result) = recorded(fugue, everything());
        let ons = events
            .iter()
            .filter(|(_, e)| matches!(e, Event::Note { kind: NoteEventKind::On, .. }))
            .count();
        assert_eq!(ons, 1, "one pass, not none");
        assert_eq!(result.frames, 24000 + (RELEASE_TAIL_S * 48000.0) as usize);
        assert!(
            result.warnings.iter().any(|w| w.contains("\"times\": 0") && w.contains("one pass")),
            "{:?}",
            result.warnings
        );
    }

    /// An endless `loop_mode` has no length, so it is refused rather than
    /// guessed at — and the message says where it *does* apply (§4.6).
    #[test]
    fn an_endless_loop_mode_is_refused_offline() {
        for (mode, spelling) in [(LoopMode::Loop, "loop"), (LoopMode::PingPong, "ping_pong")] {
            let fugue = CompositeFugue {
                duration_beats: Some(1.0),
                loop_mode: mode,
                ..Default::default()
            };
            let mut inst = RecordingInstance::new(everything());
            let err = render_offline(
                &mut inst,
                &RenderOptions { fugue: fugue.compile(), ..Default::default() },
            )
            .expect_err("an endless loop has no offline length");

            let HostError::InvalidRequest(message) = err else {
                panic!("expected InvalidRequest, got {err:?}");
            };
            assert!(message.contains(spelling), "{message}");
            assert!(message.contains("play"), "the message must name where it applies: {message}");
        }
    }

    /// A block size below the renderer's floor is raised, and the caller is
    /// told — one of the things nexus decides on the caller's behalf.
    #[test]
    fn a_floored_block_size_is_reported() {
        let mut inst = RecordingInstance::new(everything());
        let result = render_offline(
            &mut inst,
            &RenderOptions {
                block_size: 16,
                fugue: CompositeFugue { duration_beats: Some(1.0), ..Default::default() }.compile(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            result.warnings.iter().any(|w| w.contains("block_size 16 raised to 64")),
            "{:?}",
            result.warnings
        );
    }

    /// A render whose total duration exceeds 60 seconds is refused (flux parity, I-96).
    #[test]
    fn render_duration_exceeding_ceiling_is_refused() {
        let mut inst = RecordingInstance::new(everything());
        // 130 beats at 120 bpm = 65 seconds (> 60 s ceiling)
        let fugue = CompositeFugue { duration_beats: Some(130.0), ..Default::default() };
        let err = render_offline(
            &mut inst,
            &RenderOptions { fugue: fugue.compile(), ..Default::default() },
        )
        .expect_err("duration exceeding 60 s must be refused");

        let HostError::InvalidRequest(msg) = err else {
            panic!("expected InvalidRequest, got {err:?}");
        };
        assert!(msg.contains("exceeds the maximum allowed offline length"), "{msg}");
    }

    /// A `params` lane does not re-send held values at every automation step (I-95).
    #[test]
    fn param_held_values_are_not_resent_every_block() {
        // A 2-beat fugue at 120 bpm = 1.0 s (+ 1.0 s tail = 2.0 s total = 96,000 frames = 1500 automation steps).
        // Two points: (0.0, 0.2) and (0.5, 0.8).
        let mut params = std::collections::HashMap::new();
        params.insert(
            1,
            vec![
                simply_fugue::composite::ParamPointItem {
                    beat: 0.0,
                    value: 0.2,
                    curve: InterpolationMode::None,
                },
                simply_fugue::composite::ParamPointItem {
                    beat: 0.5,
                    value: 0.8,
                    curve: InterpolationMode::None,
                },
            ],
        );
        let fugue = CompositeFugue {
            duration_beats: Some(2.0),
            params,
            ..Default::default()
        };
        let (events, _) = recorded(fugue, everything());
        let param_events: Vec<&Event> = events
            .iter()
            .map(|(_, e)| e)
            .filter(|e| matches!(e, Event::ParamValue { param_id: 1, .. }))
            .collect();
        // With stepped interpolation, there are exactly 2 distinct value changes: 0.2 and 0.8.
        assert_eq!(param_events.len(), 2, "held parameter values must not be repeated: {param_events:?}");
    }

    /// Parameter changes are delivered to param_producer when available and NOT duplicated
    /// in block_events (I-94).
    #[test]
    fn param_changes_not_duplicated_when_producer_present() {
        struct ProducerAndEventTrackingInstance {
            param_producer: Option<rtrb::Producer<ParamChange>>,
            received_block_events: Arc<Mutex<Vec<Event>>>,
        }

        impl PluginInstance for ProducerAndEventTrackingInstance {
            fn activate(&mut self, _config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
                let (producer, consumer) = rtrb::RingBuffer::new(1024);
                self.param_producer = Some(producer);
                Ok(Box::new(TrackingProcessor {
                    _consumer: consumer,
                    events: self.received_block_events.clone(),
                }))
            }

            fn deactivate(&mut self) {
                self.param_producer = None;
            }

            fn param_producer(&mut self) -> Option<&mut rtrb::Producer<ParamChange>> {
                self.param_producer.as_mut()
            }

            fn take_param_producer(&mut self) -> Option<rtrb::Producer<ParamChange>> {
                self.param_producer.take()
            }
        }

        struct TrackingProcessor {
            _consumer: rtrb::Consumer<ParamChange>,
            events: Arc<Mutex<Vec<Event>>>,
        }

        impl AudioProcessor for TrackingProcessor {
            fn start_processing(&mut self) -> Result<(), HostError> { Ok(()) }
            fn stop_processing(&mut self) {}
            fn process(&mut self, block: &mut ProcessBlock<'_>) {
                let mut evs = self.events.lock().unwrap();
                for timed in block.events.iter() {
                    evs.push(timed.event.clone());
                }
            }
        }

        let events_seen = Arc::new(Mutex::new(Vec::new()));
        let mut inst = ProducerAndEventTrackingInstance {
            param_producer: None,
            received_block_events: events_seen.clone(),
        };

        let mut params = std::collections::HashMap::new();
        params.insert(
            1,
            vec![simply_fugue::composite::ParamPointItem {
                beat: 0.0,
                value: 0.5,
                curve: InterpolationMode::None,
            }],
        );
        let fugue = CompositeFugue {
            duration_beats: Some(1.0),
            params,
            ..Default::default()
        };

        let result = render_offline(
            &mut inst,
            &RenderOptions { fugue: fugue.compile(), ..Default::default() },
        );
        assert!(result.is_ok());

        let locked = events_seen.lock().unwrap();
        let param_events_in_block: Vec<&Event> = locked
            .iter()
            .filter(|e| matches!(e, Event::ParamValue { .. }))
            .collect();
        assert!(
            param_events_in_block.is_empty(),
            "ParamValue must not be pushed to block_events when param_producer is present (I-94), but found {param_events_in_block:?}"
        );

        // The same render also tells the plugin the tempo, once, first (I-104).
        let transports: Vec<&Event> =
            locked.iter().filter(|e| matches!(e, Event::Transport(_))).collect();
        assert_eq!(transports.len(), 1, "one transport event: {transports:?}");
        let Some(Event::Transport(t)) = locked.first() else {
            panic!("the transport is the first event: {:?}", locked.first());
        };
        assert!(t.playing);
        assert_eq!((t.tempo_bpm, t.song_pos_beats), (120.0, 0.0));
    }
}

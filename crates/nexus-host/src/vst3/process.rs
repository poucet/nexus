//! VST3 realtime processing: the audio-thread half of a loaded plugin.
//!
//! [`Vst3AudioProcessor`] maps the façade's [`ProcessBlock`] onto VST3
//! `ProcessData`. VST3 requires the host to present buffers for **every** audio
//! bus the plugin exposes (Surge XT, for instance, has three output buses), so
//! we allocate per-bus buffers and set `numInputs`/`numOutputs` to the full bus
//! counts. The façade's single input/output map to the **main** bus (index 0).
//! Buffers are allocated once at activation and reused every block.

use std::collections::HashMap;
use std::ptr;
use std::sync::Arc;

use vst3_sdk::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, ControllerNumbers_, Event, Event_, IAudioProcessor,
    IAudioProcessorTrait, IEventList, IParameterChanges,
    NoteExpressionTypeID, NoteExpressionTypeIDs_, NoteExpressionValueEvent, NoteOffEvent,
    NoteOnEvent, PolyPressureEvent, ProcessContext, ProcessContext_, ProcessData, ProcessModes_,
    SymbolicSampleSizes_,
};
use vst3_sdk::{ComPtr, ComWrapper};

use crate::host::{AudioProcessor, HostError, ParamChange};
use crate::process::{
    Event as FacadeEvent, NoteEventKind, NoteExpression as FacadeNoteExpression, ProcessBlock,
    TimedEvent, Transport, TransportClock,
};
use crate::vst3::event_list::HostEventList;
use crate::vst3::module::LoadedModule;
use crate::vst3::param_changes::HostParameterChanges;
use crate::vst3::params::Normalizer;

/// Mapping from `(channel, ctrl_number)` to VST3 `ParamID`.
pub type MidiControllerMap = HashMap<(i16, i16), u32>;

/// Per-channel buffers for one audio bus, plus the channel-pointer array VST3
/// reads/writes (`channelBuffers32`).
struct BusBuffers {
    bufs: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl BusBuffers {
    fn new(channels: usize, max_frames: usize) -> Self {
        Self {
            bufs: (0..channels).map(|_| vec![0.0; max_frames]).collect(),
            ptrs: vec![ptr::null_mut(); channels],
        }
    }

    /// Zero the live region and refresh the pointer array. Returns the VST3
    /// `AudioBusBuffers` view (valid while `self` is not moved).
    fn prepare(&mut self, frames: usize) -> AudioBusBuffers {
        for (ch, buf) in self.bufs.iter_mut().enumerate() {
            buf[..frames].fill(0.0);
            self.ptrs[ch] = buf.as_mut_ptr();
        }
        AudioBusBuffers {
            numChannels: self.bufs.len() as i32,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 { channelBuffers32: self.ptrs.as_mut_ptr() },
        }
    }
}

/// A VST3 plugin's audio-thread half. The COM objects are driven from a single
/// audio thread, so the whole processor is `Send` (see the `unsafe impl`).
pub struct Vst3AudioProcessor {
    processor: ComPtr<IAudioProcessor>,
    /// Pins the plugin's module loaded for as long as this processor lives.
    _module: Arc<dyn LoadedModule>,
    max_frames: usize,
    in_buses: Vec<BusBuffers>,
    out_buses: Vec<BusBuffers>,
    /// Scratch `AudioBusBuffers` arrays handed to the plugin each block.
    audio_in: Vec<AudioBusBuffers>,
    audio_out: Vec<AudioBusBuffers>,
    event_list: ComWrapper<HostEventList>,
    vst_events: Vec<Event>,
    param_consumer: rtrb::Consumer<ParamChange>,
    param_changes: ComWrapper<HostParameterChanges>,
    /// Plain → normalised, sampled on the main thread (`I-100`).
    normalizer: Normalizer,
    midi_map: MidiControllerMap,
    transport: TransportClock,
    /// Handed to the plugin each block while there is a transport.
    context: ProcessContext,
    sample_rate: f64,
    /// Samples processed since activation (`continousTimeSamples`).
    steady: i64,
    started: bool,
}

// SAFETY: after activation the processor is owned exclusively by the audio
// thread; VST3 processing is defined for a single such thread.
unsafe impl Send for Vst3AudioProcessor {}

impl Vst3AudioProcessor {
    /// `input_bus_channels` / `output_bus_channels` give the channel count of
    /// each audio bus, in bus-index order (bus 0 is the main bus).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        processor: ComPtr<IAudioProcessor>,
        module: Arc<dyn LoadedModule>,
        input_bus_channels: &[usize],
        output_bus_channels: &[usize],
        max_frames: usize,
        param_consumer: rtrb::Consumer<ParamChange>,
        normalizer: Normalizer,
        midi_map: MidiControllerMap,
        sample_rate: f64,
    ) -> Self {
        let in_buses = input_bus_channels.iter().map(|&c| BusBuffers::new(c, max_frames)).collect();
        let out_buses =
            output_bus_channels.iter().map(|&c| BusBuffers::new(c, max_frames)).collect();
        Self {
            processor,
            _module: module,
            max_frames,
            in_buses,
            out_buses,
            audio_in: Vec::with_capacity(input_bus_channels.len()),
            audio_out: Vec::with_capacity(output_bus_channels.len()),
            event_list: HostEventList::new(64),
            vst_events: Vec::with_capacity(64),
            param_consumer,
            param_changes: HostParameterChanges::new(),
            normalizer,
            midi_map,
            transport: TransportClock::new(sample_rate),
            // SAFETY: a plain-data C struct; all-zero is a valid empty context.
            context: unsafe { std::mem::zeroed() },
            sample_rate,
            steady: 0,
            started: false,
        }
    }
}

/// Fill `context` from the façade transport. `steady` is the sample count since
/// activation.
pub fn fill_context(context: &mut ProcessContext, t: &Transport, sample_rate: f64, steady: i64) {
    use ProcessContext_::StatesAndFlags_::*;
    let mut state = kTempoValid
        | kTimeSigValid
        | kProjectTimeMusicValid
        | kBarPositionValid
        | kContTimeValid;
    if t.playing {
        state |= kPlaying;
    }
    if t.loop_active {
        state |= kCycleActive;
    }
    let (_, bar_start) = t.bar();
    context.state = state;
    context.sampleRate = sample_rate;
    context.projectTimeSamples = (t.song_pos_seconds * sample_rate).round() as i64;
    context.continousTimeSamples = steady;
    context.projectTimeMusic = t.song_pos_beats;
    context.barPositionMusic = bar_start;
    context.tempo = t.tempo_bpm;
    context.timeSigNumerator = i32::from(t.time_sig_numerator);
    context.timeSigDenominator = i32::from(t.time_sig_denominator);
}

/// A MIDI message the plugin's `IMidiMapping` routes to a parameter: CC,
/// pitch bend or channel pressure, as `(param id, normalised value)`. A channel
/// without its own mapping falls back to channel 0's.
fn midi_to_param(midi_map: &MidiControllerMap, data: &[u8; 3]) -> Option<(u32, f64)> {
    let channel = (data[0] & 0x0F) as i16;
    let (ctrl, norm) = match data[0] & 0xF0 {
        0xB0 => (data[1] as i16, data[2] as f64 / 127.0),
        0xE0 => {
            let raw = (data[1] as u32) | ((data[2] as u32) << 7);
            (ControllerNumbers_::kPitchBend as i16, raw as f64 / 16383.0)
        }
        0xD0 => (ControllerNumbers_::kAfterTouch as i16, data[1] as f64 / 127.0),
        _ => return None,
    };
    let id = midi_map.get(&(channel, ctrl)).or_else(|| midi_map.get(&(0, ctrl)))?;
    Some((*id, norm))
}

/// Translate one façade event into a VST3 event, if it maps to a note, poly pressure,
/// or per-note expression. Raw MIDI note-on/off and poly-pressure are converted too;
/// CC/other MIDI and param events are skipped here (routed via `IParameterChanges`).
fn to_vst_event(timed: &TimedEvent) -> Option<Event> {
    let on = |channel: i16, key: i16, velocity: f32, note_id: i32| {
        // SAFETY: `Event` is a POD union; zeroing then setting one variant + the
        // discriminant is how the C++ SDK constructs events.
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = timed.time as i32;
        e.r#type = Event_::EventTypes_::kNoteOnEvent as u16;
        e.__field0.noteOn =
            NoteOnEvent { channel, pitch: key, tuning: 0.0, velocity, length: 0, noteId: note_id };
        e
    };
    let off = |channel: i16, key: i16, velocity: f32, note_id: i32| {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = timed.time as i32;
        e.r#type = Event_::EventTypes_::kNoteOffEvent as u16;
        e.__field0.noteOff =
            NoteOffEvent { channel, pitch: key, velocity, noteId: note_id, tuning: 0.0 };
        e
    };
    let poly_pressure = |channel: i16, key: i16, pressure: f32, note_id: i32| {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = timed.time as i32;
        e.r#type = Event_::EventTypes_::kPolyPressureEvent as u16;
        e.__field0.polyPressure =
            PolyPressureEvent { channel, pitch: key, pressure, noteId: note_id };
        e
    };
    let note_expression = |type_id: NoteExpressionTypeID, note_id: i32, value: f64| {
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = timed.time as i32;
        e.r#type = Event_::EventTypes_::kNoteExpressionValueEvent as u16;
        e.__field0.noteExpressionValue =
            NoteExpressionValueEvent { typeId: type_id, noteId: note_id, value };
        e
    };

    match &timed.event {
        FacadeEvent::Note { kind, note_id, channel, key, velocity, .. } => match kind {
            NoteEventKind::On => Some(on(*channel, *key, *velocity as f32, *note_id)),
            NoteEventKind::Off | NoteEventKind::Choke => {
                Some(off(*channel, *key, *velocity as f32, *note_id))
            }
            NoteEventKind::End => None,
        },
        FacadeEvent::NoteExpression { expression, note_id, channel, key, value, .. } => {
            match expression {
                FacadeNoteExpression::Pressure => {
                    Some(poly_pressure(*channel, *key, (*value as f32).clamp(0.0, 1.0), *note_id))
                }
                FacadeNoteExpression::Volume => Some(note_expression(
                    NoteExpressionTypeIDs_::kVolumeTypeID,
                    *note_id,
                    value.clamp(0.0, 1.0),
                )),
                FacadeNoteExpression::Pan => Some(note_expression(
                    NoteExpressionTypeIDs_::kPanTypeID,
                    *note_id,
                    value.clamp(0.0, 1.0),
                )),
                FacadeNoteExpression::Tuning => {
                    // CLAP Tuning is relative semitones (-120.0..=120.0).
                    // VST3 kTuningTypeID is normalized (0.0..=1.0) with 0.5 as center (0 semitones).
                    let norm = ((*value / 240.0) + 0.5).clamp(0.0, 1.0);
                    Some(note_expression(NoteExpressionTypeIDs_::kTuningTypeID, *note_id, norm))
                }
                FacadeNoteExpression::Vibrato => Some(note_expression(
                    NoteExpressionTypeIDs_::kVibratoTypeID,
                    *note_id,
                    value.clamp(0.0, 1.0),
                )),
                FacadeNoteExpression::Expression => Some(note_expression(
                    NoteExpressionTypeIDs_::kExpressionTypeID,
                    *note_id,
                    value.clamp(0.0, 1.0),
                )),
                FacadeNoteExpression::Brightness => Some(note_expression(
                    NoteExpressionTypeIDs_::kBrightnessTypeID,
                    *note_id,
                    value.clamp(0.0, 1.0),
                )),
            }
        }
        FacadeEvent::Midi { data, .. } => {
            let status = data[0] & 0xF0;
            let channel = (data[0] & 0x0F) as i16;
            let key = data[1] as i16;
            let velocity = data[2] as f32 / 127.0;
            match status {
                0x90 if data[2] > 0 => Some(on(channel, key, velocity, -1)),
                0x80 | 0x90 => Some(off(channel, key, velocity, -1)),
                0xA0 => {
                    let pressure = (data[2] as f32 / 127.0).clamp(0.0, 1.0);
                    Some(poly_pressure(channel, key, pressure, -1))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

impl AudioProcessor for Vst3AudioProcessor {
    fn start_processing(&mut self) -> Result<(), HostError> {
        // SAFETY: FFI into the plugin; processor is a live IAudioProcessor.
        // Some plugins return kNotImplemented; treat non-error as success.
        unsafe { self.processor.setProcessing(1) };
        self.started = true;
        Ok(())
    }

    fn stop_processing(&mut self) {
        if self.started {
            unsafe { self.processor.setProcessing(0) };
            self.started = false;
        }
    }

    fn process(&mut self, block: &mut ProcessBlock<'_>) {
        let Self {
            processor,
            max_frames,
            in_buses,
            out_buses,
            audio_in,
            audio_out,
            event_list,
            vst_events,
            param_consumer,
            param_changes,
            normalizer,
            midi_map,
            transport,
            context,
            sample_rate,
            steady,
            ..
        } = self;
        let frames = block.frames.min(*max_frames);

        // --- parameter changes: preallocated queues, no controller call ---
        param_changes.begin();
        let last = frames.saturating_sub(1) as i32;
        while let Ok(change) = param_consumer.pop() {
            let offset = (change.offset_frames as i32).min(last);
            param_changes.push(change.id, offset, normalizer.normalize(change.id, change.value));
        }
        for timed in block.events.iter() {
            let offset = (timed.time as i32).min(last);
            match &timed.event {
                FacadeEvent::ParamValue { param_id, value, .. } => {
                    param_changes.push(*param_id, offset, normalizer.normalize(*param_id, *value));
                }
                FacadeEvent::Midi { data, .. } => {
                    if let Some((param_id, norm)) = midi_to_param(midi_map, data) {
                        param_changes.push(param_id, offset, norm);
                    }
                }
                _ => {}
            }
        }

        // --- events ---
        vst_events.clear();
        for timed in block.events.iter() {
            if let Some(e) = to_vst_event(timed) {
                vst_events.push(e);
            }
        }
        event_list.set(vst_events);

        // --- input buses: feed the façade's input into the main bus (0) ---
        audio_in.clear();
        for (bus_idx, bus) in in_buses.iter_mut().enumerate() {
            let abb = bus.prepare(frames);
            if bus_idx == 0 {
                // Fill the main input bus in place; `prepare` already pointed the
                // channel pointers at these buffers, and writing doesn't realloc.
                for ch in 0..bus.bufs.len() {
                    if let Some(src) = block.input.channel(ch) {
                        let n = src.len().min(frames);
                        bus.bufs[ch][..n].copy_from_slice(&src[..n]);
                    }
                }
            }
            audio_in.push(abb);
        }

        // --- output buses: zeroed, plugin writes into them ---
        audio_out.clear();
        for bus in out_buses.iter_mut() {
            audio_out.push(bus.prepare(frames));
        }

        // Keep the event-list and param-changes ComPtrs alive across the call.
        let events = event_list.to_com_ptr::<IEventList>();
        let events_ptr = events.as_ref().map_or(ptr::null_mut(), |e| e.as_ptr());

        let param_changes_com = param_changes.to_com_ptr::<IParameterChanges>();
        let param_changes_ptr = if param_changes.active_count() > 0 {
            param_changes_com.as_ref().map_or(ptr::null_mut(), |p| p.as_ptr())
        } else {
            ptr::null_mut()
        };

        let context_ptr = match transport.block(block.events, frames) {
            Some(t) => {
                fill_context(context, &t, *sample_rate, *steady);
                context as *mut ProcessContext
            }
            None => ptr::null_mut(),
        };
        *steady += frames as i64;

        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: in_buses.len() as i32,
            numOutputs: out_buses.len() as i32,
            inputs: if audio_in.is_empty() { ptr::null_mut() } else { audio_in.as_mut_ptr() },
            outputs: if audio_out.is_empty() { ptr::null_mut() } else { audio_out.as_mut_ptr() },
            inputParameterChanges: param_changes_ptr,
            outputParameterChanges: ptr::null_mut(),
            inputEvents: events_ptr,
            outputEvents: ptr::null_mut(),
            processContext: context_ptr,
        };

        // SAFETY: all pointers above are valid for the duration of the call.
        let _ = unsafe { processor.process(&mut data) };
        drop(param_changes_com);
        drop(events);

        // --- copy the main output bus (0) back into the façade ---
        if let Some(bus0) = out_buses.first() {
            for (ch, buf) in bus0.bufs.iter().enumerate() {
                if let Some(dst) = block.output.channel_mut(ch) {
                    let n = dst.len().min(frames);
                    dst[..n].copy_from_slice(&buf[..n]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::NoteEventKind;
    use vst3_sdk::Steinberg::Vst::Event_::EventTypes_;

    #[test]
    fn the_transport_becomes_a_process_context() {
        let t = Transport {
            playing: true,
            loop_active: false,
            tempo_bpm: 90.0,
            song_pos_beats: 9.5,
            song_pos_seconds: 9.5 * 60.0 / 90.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
        };
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        fill_context(&mut context, &t, 48_000.0, 1_234);
        use ProcessContext_::StatesAndFlags_::*;
        let state = context.state as i64;
        for flag in [kPlaying, kTempoValid, kTimeSigValid, kProjectTimeMusicValid, kBarPositionValid] {
            assert_ne!(state & flag as i64, 0, "flag {flag} set");
        }
        assert_eq!(state & kCycleActive as i64, 0);
        assert_eq!(context.tempo, 90.0);
        assert_eq!(context.projectTimeMusic, 9.5);
        assert_eq!(context.barPositionMusic, 8.0);
        assert_eq!(context.projectTimeSamples, 304_000);
        assert_eq!(context.continousTimeSamples, 1_234);
        assert_eq!((context.timeSigNumerator, context.timeSigDenominator), (4, 4));

        fill_context(&mut context, &Transport { playing: false, ..t }, 48_000.0, 0);
        assert_eq!(context.state as i64 & kPlaying as i64, 0);
    }

    #[test]
    fn test_to_vst_event_notes() {
        let on = TimedEvent {
            time: 10,
            event: FacadeEvent::Note {
                kind: NoteEventKind::On,
                note_id: 42,
                port_index: 0,
                channel: 1,
                key: 60,
                velocity: 0.8,
            },
        };
        let event = to_vst_event(&on).expect("should convert note on");
        assert_eq!(event.sampleOffset, 10);
        assert_eq!(event.r#type, EventTypes_::kNoteOnEvent as u16);
        let note_on = unsafe { event.__field0.noteOn };
        assert_eq!(note_on.channel, 1);
        assert_eq!(note_on.pitch, 60);
        assert_eq!(note_on.noteId, 42);
        assert!((note_on.velocity - 0.8).abs() < 1e-4);

        let off = TimedEvent {
            time: 20,
            event: FacadeEvent::Note {
                kind: NoteEventKind::Off,
                note_id: 42,
                port_index: 0,
                channel: 1,
                key: 60,
                velocity: 0.0,
            },
        };
        let event = to_vst_event(&off).expect("should convert note off");
        assert_eq!(event.sampleOffset, 20);
        assert_eq!(event.r#type, EventTypes_::kNoteOffEvent as u16);
        let note_off = unsafe { event.__field0.noteOff };
        assert_eq!(note_off.channel, 1);
        assert_eq!(note_off.pitch, 60);
        assert_eq!(note_off.noteId, 42);

        let choke = TimedEvent {
            time: 25,
            event: FacadeEvent::Note {
                kind: NoteEventKind::Choke,
                note_id: 42,
                port_index: 0,
                channel: 1,
                key: 60,
                velocity: 0.0,
            },
        };
        assert_eq!(
            to_vst_event(&choke).unwrap().r#type,
            EventTypes_::kNoteOffEvent as u16
        );

        let end = TimedEvent {
            time: 30,
            event: FacadeEvent::Note {
                kind: NoteEventKind::End,
                note_id: 42,
                port_index: 0,
                channel: 1,
                key: 60,
                velocity: 0.0,
            },
        };
        assert!(to_vst_event(&end).is_none());
    }

    #[test]
    fn test_to_vst_event_raw_midi() {
        // Raw Note On (0x90)
        let midi_on = TimedEvent {
            time: 5,
            event: FacadeEvent::Midi {
                port_index: 0,
                data: [0x91, 64, 100],
            },
        };
        let e = to_vst_event(&midi_on).expect("raw midi note on");
        assert_eq!(e.r#type, EventTypes_::kNoteOnEvent as u16);
        let n = unsafe { e.__field0.noteOn };
        assert_eq!(n.channel, 1);
        assert_eq!(n.pitch, 64);
        assert!((n.velocity - (100.0 / 127.0)).abs() < 1e-4);

        // Raw Note Off via 0x90 with vel 0
        let midi_off0 = TimedEvent {
            time: 6,
            event: FacadeEvent::Midi {
                port_index: 0,
                data: [0x91, 64, 0],
            },
        };
        let e = to_vst_event(&midi_off0).expect("raw midi note off");
        assert_eq!(e.r#type, EventTypes_::kNoteOffEvent as u16);

        // Raw Poly Pressure (0xA0)
        let poly_press = TimedEvent {
            time: 15,
            event: FacadeEvent::Midi {
                port_index: 0,
                data: [0xA2, 60, 96],
            },
        };
        let e = to_vst_event(&poly_press).expect("raw midi poly pressure");
        assert_eq!(e.r#type, EventTypes_::kPolyPressureEvent as u16);
        let p = unsafe { e.__field0.polyPressure };
        assert_eq!(p.channel, 2);
        assert_eq!(p.pitch, 60);
        assert!((p.pressure - (96.0 / 127.0)).abs() < 1e-4);

        // CC (0xB0), Pitch Bend (0xE0), Aftertouch (0xD0) return None from to_vst_event
        // because VST3 routes them via IParameterChanges / IMidiMapping.
        for status in [0xB0, 0xE0, 0xD0] {
            let midi = TimedEvent {
                time: 0,
                event: FacadeEvent::Midi {
                    port_index: 0,
                    data: [status, 64, 64],
                },
            };
            assert!(to_vst_event(&midi).is_none());
        }
    }

    #[test]
    fn test_to_vst_event_note_expressions() {
        // Pressure -> PolyPressureEvent
        let press = TimedEvent {
            time: 12,
            event: FacadeEvent::NoteExpression {
                expression: FacadeNoteExpression::Pressure,
                note_id: 10,
                port_index: 0,
                channel: 0,
                key: 48,
                value: 0.75,
            },
        };
        let e = to_vst_event(&press).expect("pressure note expression");
        assert_eq!(e.r#type, EventTypes_::kPolyPressureEvent as u16);
        let p = unsafe { e.__field0.polyPressure };
        assert_eq!(p.noteId, 10);
        assert_eq!(p.channel, 0);
        assert_eq!(p.pitch, 48);
        assert!((p.pressure - 0.75).abs() < 1e-4);

        // Tuning -> NoteExpressionValueEvent (kTuningTypeID)
        // 0 semitones -> 0.5 normalized
        let tune0 = TimedEvent {
            time: 14,
            event: FacadeEvent::NoteExpression {
                expression: FacadeNoteExpression::Tuning,
                note_id: 11,
                port_index: 0,
                channel: 0,
                key: 48,
                value: 0.0,
            },
        };
        let e = to_vst_event(&tune0).expect("tuning 0 semitones");
        assert_eq!(e.r#type, EventTypes_::kNoteExpressionValueEvent as u16);
        let ne = unsafe { e.__field0.noteExpressionValue };
        assert_eq!(ne.typeId, NoteExpressionTypeIDs_::kTuningTypeID);
        assert_eq!(ne.noteId, 11);
        assert!((ne.value - 0.5).abs() < 1e-6);

        // +120 semitones -> 1.0 normalized
        let tune_up = TimedEvent {
            time: 15,
            event: FacadeEvent::NoteExpression {
                expression: FacadeNoteExpression::Tuning,
                note_id: 11,
                port_index: 0,
                channel: 0,
                key: 48,
                value: 120.0,
            },
        };
        let ne = unsafe { to_vst_event(&tune_up).unwrap().__field0.noteExpressionValue };
        assert!((ne.value - 1.0).abs() < 1e-6);

        // -120 semitones -> 0.0 normalized
        let tune_down = TimedEvent {
            time: 16,
            event: FacadeEvent::NoteExpression {
                expression: FacadeNoteExpression::Tuning,
                note_id: 11,
                port_index: 0,
                channel: 0,
                key: 48,
                value: -120.0,
            },
        };
        let ne = unsafe { to_vst_event(&tune_down).unwrap().__field0.noteExpressionValue };
        assert!((ne.value - 0.0).abs() < 1e-6);

        // Volume, Pan, Vibrato, Expression, Brightness
        let exprs = [
            (FacadeNoteExpression::Volume, NoteExpressionTypeIDs_::kVolumeTypeID),
            (FacadeNoteExpression::Pan, NoteExpressionTypeIDs_::kPanTypeID),
            (FacadeNoteExpression::Vibrato, NoteExpressionTypeIDs_::kVibratoTypeID),
            (FacadeNoteExpression::Expression, NoteExpressionTypeIDs_::kExpressionTypeID),
            (FacadeNoteExpression::Brightness, NoteExpressionTypeIDs_::kBrightnessTypeID),
        ];
        for (facade_expr, expected_type_id) in exprs {
            let timed = TimedEvent {
                time: 1,
                event: FacadeEvent::NoteExpression {
                    expression: facade_expr,
                    note_id: 99,
                    port_index: 0,
                    channel: 0,
                    key: 60,
                    value: 0.65,
                },
            };
            let e = to_vst_event(&timed).expect("expression converted");
            assert_eq!(e.r#type, EventTypes_::kNoteExpressionValueEvent as u16);
            let ne = unsafe { e.__field0.noteExpressionValue };
            assert_eq!(ne.typeId, expected_type_id);
            assert_eq!(ne.noteId, 99);
            assert!((ne.value - 0.65).abs() < 1e-6);
        }
    }
}


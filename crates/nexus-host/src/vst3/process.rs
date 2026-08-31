//! VST3 realtime processing: the audio-thread half of a loaded plugin.
//!
//! [`Vst3AudioProcessor`] maps the façade's [`ProcessBlock`] onto VST3
//! `ProcessData`. VST3 requires the host to present buffers for **every** audio
//! bus the plugin exposes (Surge XT, for instance, has three output buses), so
//! we allocate per-bus buffers and set `numInputs`/`numOutputs` to the full bus
//! counts. The façade's single input/output map to the **main** bus (index 0).
//! Buffers are allocated once at activation and reused every block.

use std::collections::BTreeMap;
use std::ptr;
use std::sync::Arc;

use vst3_sdk::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, Event_, IAudioProcessor, IAudioProcessorTrait,
    IEditController, IEditControllerTrait, IEventList, IParameterChanges, NoteOffEvent, NoteOnEvent,
    ProcessData, ProcessModes_, SymbolicSampleSizes_,
};
use vst3_sdk::{ComPtr, ComWrapper};

use crate::host::{AudioProcessor, HostError, ParamChange};
use crate::process::{Event as FacadeEvent, NoteEventKind, ProcessBlock, TimedEvent};
use crate::vst3::event_list::HostEventList;
use crate::vst3::module::LoadedModule;
use crate::vst3::param_changes::{HostParameterChanges, ParamPoint};

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
    controller: Option<ComPtr<IEditController>>,
    started: bool,
}

// SAFETY: after activation the processor is owned exclusively by the audio
// thread; VST3 processing is defined for a single such thread.
unsafe impl Send for Vst3AudioProcessor {}

impl Vst3AudioProcessor {
    /// `input_bus_channels` / `output_bus_channels` give the channel count of
    /// each audio bus, in bus-index order (bus 0 is the main bus).
    pub fn new(
        processor: ComPtr<IAudioProcessor>,
        module: Arc<dyn LoadedModule>,
        input_bus_channels: &[usize],
        output_bus_channels: &[usize],
        max_frames: usize,
        param_consumer: rtrb::Consumer<ParamChange>,
        controller: Option<ComPtr<IEditController>>,
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
            param_changes: HostParameterChanges::new(16),
            controller,
            started: false,
        }
    }
}

/// Translate one façade event into a VST3 event, if it maps to a note. Raw MIDI
/// note-on/off are converted too; CC/other MIDI and param events are skipped
/// (params arrive via `IParameterChanges`, wired later).
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

    match &timed.event {
        FacadeEvent::Note { kind, note_id, channel, key, velocity, .. } => match kind {
            NoteEventKind::On => Some(on(*channel, *key, *velocity as f32, *note_id)),
            NoteEventKind::Off | NoteEventKind::Choke => {
                Some(off(*channel, *key, *velocity as f32, *note_id))
            }
            NoteEventKind::End => None,
        },
        FacadeEvent::Midi { data, .. } => {
            let status = data[0] & 0xF0;
            let channel = (data[0] & 0x0F) as i16;
            let key = data[1] as i16;
            let velocity = data[2] as f32 / 127.0;
            match status {
                0x90 if data[2] > 0 => Some(on(channel, key, velocity, -1)),
                0x80 | 0x90 => Some(off(channel, key, velocity, -1)),
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
            controller,
            ..
        } = self;
        let frames = block.frames.min(*max_frames);

        // --- parameter changes ---
        let mut param_map: BTreeMap<u32, Vec<ParamPoint>> = BTreeMap::new();

        while let Ok(change) = param_consumer.pop() {
            let offset = (change.offset_frames as i32).min(frames.saturating_sub(1) as i32);
            let norm = controller.as_ref().map_or(change.value, |c| unsafe {
                c.plainParamToNormalized(change.id, change.value)
            });
            param_map.entry(change.id).or_default().push(ParamPoint {
                sample_offset: offset,
                value: norm,
            });
        }

        for timed in block.events.iter() {
            if let FacadeEvent::ParamValue { param_id, value, .. } = &timed.event {
                let offset = (timed.time as i32).min(frames.saturating_sub(1) as i32);
                let norm = controller.as_ref().map_or(*value, |c| unsafe {
                    c.plainParamToNormalized(*param_id, *value)
                });
                param_map.entry(*param_id).or_default().push(ParamPoint {
                    sample_offset: offset,
                    value: norm,
                });
            }
        }

        let changes_vec: Vec<_> = param_map.into_iter().collect();
        param_changes.set_changes(&changes_vec);

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
            processContext: ptr::null_mut(),
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

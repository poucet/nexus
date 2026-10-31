//! CLAP realtime processing: the audio-thread half of a loaded plugin.
//!
//! [`ClapAudioProcessor`] maps the façade's [`ProcessBlock`] onto clack's audio
//! port buffers and event list, calls the plugin's `process`, and copies the
//! result back. Buffers are allocated once at activation (sized to the plugin's
//! [`IoLayout`]) and reused every block. Modeled on clack's cpal host example.

use clack_host::events::Match;
use clack_host::events::event_types::{
    Midi2Event, MidiEvent, NoteChokeEvent, NoteExpressionEvent, NoteExpressionType, NoteOffEvent, NoteOnEvent,
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamModEvent, ParamValueEvent, TransportEvent,
    TransportFlags,
};
use clack_host::events::{EventFlags, EventHeader};
use clack_host::utils::BeatTime;
use clack_host::prelude::*;
use clack_host::utils::Cookie;

use crate::clap::host::{AudioThread, NexusHost};
use crate::clap::ports::IoLayout;
use crate::host::{AudioProcessor, HostError, ParamChange};
use crate::process::{
    Event, NoteEventKind, NoteExpression, ProcessBlock, Transport, TransportClock,
};

/// The façade's expression dimensions map one-to-one onto CLAP's, which is what
/// [`crate::process::Event`] was modelled on. Units are CLAP's and are the
/// caller's contract: `Tuning` is **relative semitones, -120..=120**, and
/// `Volume` aside (`0 < x <= 4`, plain = `20·log x`) every other dimension is
/// `0..=1` — `Pan` with 0.5 as centre.
fn expression_type(expression: NoteExpression) -> NoteExpressionType {
    match expression {
        NoteExpression::Volume => NoteExpressionType::Volume,
        NoteExpression::Pan => NoteExpressionType::Pan,
        NoteExpression::Tuning => NoteExpressionType::Tuning,
        NoteExpression::Vibrato => NoteExpressionType::Vibrato,
        NoteExpression::Expression => NoteExpressionType::Expression,
        NoteExpression::Brightness => NoteExpressionType::Brightness,
        NoteExpression::Pressure => NoteExpressionType::Pressure,
    }
}

/// The façade transport as CLAP's `clap_event_transport` (no loop range: the
/// façade has none).
fn transport_event(t: &Transport) -> TransportEvent {
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_BEATS_TIMELINE
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_TIME_SIGNATURE;
    flags.set(TransportFlags::IS_PLAYING, t.playing);
    flags.set(TransportFlags::IS_LOOP_ACTIVE, t.loop_active);
    let (bar_number, bar_start) = t.bar();
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(t.song_pos_beats),
        song_pos_seconds: BeatTime::from_float(t.song_pos_seconds),
        tempo: t.tempo_bpm,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_int(0),
        loop_end_beats: BeatTime::from_int(0),
        loop_start_seconds: BeatTime::from_int(0),
        loop_end_seconds: BeatTime::from_int(0),
        bar_start: BeatTime::from_float(bar_start),
        bar_number,
        time_signature_numerator: t.time_sig_numerator,
        time_signature_denominator: t.time_sig_denominator,
    }
}

/// A CLAP plugin's audio-thread half. `Send`, so the engine can move it onto
/// the realtime callback.
pub struct ClapAudioProcessor {
    processor: PluginAudioProcessor<NexusHost>,
    io: IoLayout,
    max_frames: usize,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    /// One flat backing buffer per port: `max_frames * channel_count` samples,
    /// laid out channel-by-channel.
    input_backing: Vec<Vec<f32>>,
    output_backing: Vec<Vec<f32>>,
    events: EventBuffer,
    steady: u64,
    param_consumer: rtrb::Consumer<ParamChange>,
    transport: TransportClock,
}

impl ClapAudioProcessor {
    pub fn new(
        processor: PluginAudioProcessor<NexusHost>,
        io: IoLayout,
        max_frames: usize,
        param_consumer: rtrb::Consumer<ParamChange>,
        sample_rate: f64,
    ) -> Self {
        let input_backing = io
            .input_ports
            .iter()
            .map(|&ch| vec![0.0; max_frames * ch as usize])
            .collect();
        let output_backing = io
            .output_ports
            .iter()
            .map(|&ch| vec![0.0; max_frames * ch as usize])
            .collect();
        let input_ports = AudioPorts::with_capacity(
            io.input_ports.iter().map(|&c| c as usize).sum(),
            io.input_ports.len(),
        );
        let output_ports = AudioPorts::with_capacity(
            io.output_ports.iter().map(|&c| c as usize).sum(),
            io.output_ports.len(),
        );
        Self {
            processor,
            io,
            max_frames,
            input_ports,
            output_ports,
            input_backing,
            output_backing,
            events: EventBuffer::with_capacity(64),
            steady: 0,
            param_consumer,
            transport: TransportClock::new(sample_rate),
        }
    }
}

/// Map a façade note-id (`-1` = unset) onto a clack note-id [`Match`].
fn note_id_match(id: i32) -> Match<u32> {
    if id < 0 {
        Match::All
    } else {
        Match::Specific(id as u32)
    }
}

impl AudioProcessor for ClapAudioProcessor {
    fn start_processing(&mut self) -> Result<(), HostError> {
        let _audio = AudioThread::enter();
        self.processor
            .start_processing()
            .map(|_| ())
            .map_err(|e| HostError::Load(format!("start_processing failed: {e}")))
    }

    fn stop_processing(&mut self) {
        let _audio = AudioThread::enter();
        let _ = self.processor.stop_processing();
    }

    fn process(&mut self, block: &mut ProcessBlock<'_>) {
        let _audio = AudioThread::enter();
        // Disjoint field borrows: pull fields into locals so the borrow checker
        // can see the audio buffers, event list, and processor are independent.
        let Self {
            processor,
            io,
            max_frames,
            input_ports,
            output_ports,
            input_backing,
            output_backing,
            events,
            steady,
            param_consumer,
            transport,
        } = self;
        let max_frames = *max_frames;
        let frames = block.frames.min(max_frames);

        // --- input events: façade Event -> clack events ---
        events.clear();
        let note_port = io.note_port.unwrap_or(0);

        // Drain realtime parameter change ring from host
        while let Ok(change) = param_consumer.pop() {
            let pckn = Pckn::new(Match::All, Match::All, Match::All, Match::All);
            let offset = (change.offset_frames as u32).min(frames.saturating_sub(1) as u32);
            events.push(&ParamValueEvent::new(
                offset,
                ClapId::from(change.id),
                pckn,
                change.value,
                Cookie::empty(),
            ));
        }

        for ev in block.events.iter() {
            match &ev.event {
                Event::Note { kind, note_id, channel, key, velocity, .. } => {
                    let pckn = Pckn::new(
                        note_port,
                        *channel as u16,
                        *key as u16,
                        note_id_match(*note_id),
                    );
                    match kind {
                        NoteEventKind::On => events.push(&NoteOnEvent::new(ev.time, pckn, *velocity)),
                        NoteEventKind::Off => {
                            events.push(&NoteOffEvent::new(ev.time, pckn, *velocity))
                        }
                        NoteEventKind::Choke => events.push(&NoteChokeEvent::new(ev.time, pckn)),
                        NoteEventKind::End => {} // output-direction only
                    }
                }
                Event::NoteExpression { expression, note_id, channel, key, value, .. } => {
                    // Addressed on the plugin's note port, exactly like the note
                    // it modifies — an expression that names a different port
                    // matches no voice. The façade's own `port_index` is a host
                    // hint and the `Event::Note` arm above ignores it too.
                    let pckn = Pckn::new(
                        note_port,
                        *channel as u16,
                        *key as u16,
                        note_id_match(*note_id),
                    );
                    events.push(&NoteExpressionEvent::new(
                        ev.time,
                        pckn,
                        expression_type(*expression),
                        *value,
                    ));
                }
                Event::Midi { port_index, data } => {
                    events.push(&MidiEvent::new(ev.time, *port_index, *data))
                }
                Event::Midi2 { port_index, data } => {
                    events.push(&Midi2Event::new(ev.time, *port_index, *data))
                }
                Event::ParamValue { param_id, note_id, port_index, channel, key, value } => {
                    let port_m = if *port_index >= 0 { Match::Specific(*port_index as u16) } else { Match::All };
                    let chan_m = if *channel >= 0 { Match::Specific(*channel as u16) } else { Match::All };
                    let key_m = if *key >= 0 { Match::Specific(*key as u16) } else { Match::All };
                    let pckn = Pckn::new(port_m, chan_m, key_m, note_id_match(*note_id));
                    events.push(&ParamValueEvent::new(
                        ev.time,
                        ClapId::from(*param_id),
                        pckn,
                        *value,
                        Cookie::empty(),
                    ));
                }
                Event::ParamMod { param_id, note_id, port_index, channel, key, amount } => {
                    let port_m = if *port_index >= 0 { Match::Specific(*port_index as u16) } else { Match::All };
                    let chan_m = if *channel >= 0 { Match::Specific(*channel as u16) } else { Match::All };
                    let key_m = if *key >= 0 { Match::Specific(*key as u16) } else { Match::All };
                    let pckn = Pckn::new(port_m, chan_m, key_m, note_id_match(*note_id));
                    events.push(&ParamModEvent::new(
                        ev.time,
                        ClapId::from(*param_id),
                        pckn,
                        *amount,
                        Cookie::empty(),
                    ));
                }
                Event::ParamGestureBegin { param_id } => {
                    events.push(&ParamGestureBeginEvent::new(ev.time, ClapId::from(*param_id)));
                }
                Event::ParamGestureEnd { param_id } => {
                    events.push(&ParamGestureEndEvent::new(ev.time, ClapId::from(*param_id)));
                }
                _ => {}
            }
        }

        // --- input audio: copy the façade's input into the main input port ---
        for buf in input_backing.iter_mut() {
            buf.fill(0.0);
        }
        if let (Some(port_buf), Some(&channels)) =
            (input_backing.first_mut(), io.input_ports.first())
        {
            for ch in 0..channels as usize {
                if let Some(src) = block.input.channel(ch) {
                    let start = ch * max_frames;
                    let n = src.len().min(frames);
                    port_buf[start..start + n].copy_from_slice(&src[..n]);
                }
            }
        }
        for buf in output_backing.iter_mut() {
            buf.fill(0.0);
        }

        // --- build clack buffers and run the plugin ---
        let ins = input_ports.with_input_buffers(input_backing.iter_mut().map(|buf| {
            AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    buf.chunks_exact_mut(max_frames).map(|c| InputChannel {
                        buffer: &mut c[..frames],
                        is_constant: false,
                    }),
                ),
            }
        }));
        let mut outs = output_ports.with_output_buffers(output_backing.iter_mut().map(|buf| {
            AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    buf.chunks_exact_mut(max_frames).map(|c| &mut c[..frames]),
                ),
            }
        }));

        let transport = transport.block(block.events, frames).map(|t| transport_event(&t));
        let input_events = events.as_input();
        if let Ok(started) = processor.as_started_mut() {
            let _ = started.process(
                &ins,
                &mut outs,
                &input_events,
                &mut OutputEvents::void(),
                Some(*steady),
                transport.as_ref(),
            );
        }
        drop(outs);
        drop(ins);

        // --- copy the main output port back into the façade's output ---
        if let (Some(port_buf), Some(&channels)) =
            (output_backing.first(), io.output_ports.first())
        {
            for ch in 0..channels as usize {
                if let Some(dst) = block.output.channel_mut(ch) {
                    let start = ch * max_frames;
                    let n = dst.len().min(frames);
                    dst[..n].copy_from_slice(&port_buf[start..start + n]);
                }
            }
        }

        *steady += frames as u64;
    }
}

#[cfg(test)]
mod tests {
    use crate::process::{AudioBuffer, Event, EventBuffer, ProcessBlock};
    use crate::{AudioConfig, Host, PluginCatalog, PluginFormat};

    /// Load a real CLAP synth (Surge XT), feed it a note, and confirm it
    /// produces non-silent audio through the full process path. Ignored by
    /// default; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn renders_audio_from_a_clap_synth() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load");
        let max = 512;
        let mut processor = instance
            .activate(AudioConfig { sample_rate: 48_000.0, max_block_frames: max })
            .expect("activate");
        processor.start_processing().expect("start");

        let input = AudioBuffer::empty();
        let mut output = AudioBuffer::new(2, max);
        let mut peak = 0.0f32;
        for blk in 0..40 {
            output.clear();
            let mut events = EventBuffer::with_capacity(4);
            if blk == 0 {
                events.push(0, Event::note_on(0, 69, 1.0));
            }
            let mut block = ProcessBlock {
                frames: max,
                input: &input,
                output: &mut output,
                events: &events,
            };
            processor.process(&mut block);
            for ch in 0..output.channel_count() {
                for &s in output.channel(ch).unwrap() {
                    peak = peak.max(s.abs());
                }
            }
        }
        processor.stop_processing();

        println!("Surge XT peak output over 40 blocks: {peak}");
        assert!(peak > 0.0, "synth produced silence");
    }
}

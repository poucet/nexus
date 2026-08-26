//! The realtime engine: owns the audio output stream and MIDI input, and hands
//! the active plugin's [`AudioProcessor`] to the audio thread.
//!
//! This lives in `nexus-app`, not `nexus-host` — the host library is
//! device-agnostic, and this is where real hardware I/O happens.

mod audio;
mod midi;

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use crossbeam_channel::{Receiver, Sender};
use nexus_host::AudioProcessor;
use rtrb::Producer;

pub use audio::OutputInfo;

/// Commands handled by the audio callback between blocks.
pub(crate) enum ProcessorCommand {
    /// Install a new processor (the previous one is dropped on the audio thread).
    Set(Box<dyn AudioProcessor>),
    /// Drop the current processor and acknowledge, so the main thread knows the
    /// audio thread no longer touches the plugin before tearing it down.
    Clear,
}

/// Names of the available audio output devices.
pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Drives audio + MIDI for the standalone app.
pub struct Engine {
    _stream: cpal::Stream,
    _midi: Option<midi::MidiConnection>,
    proc_tx: Sender<ProcessorCommand>,
    /// Signalled by the audio thread after it handles a [`ProcessorCommand::Clear`].
    clear_ack: Receiver<()>,
    /// Producer for MIDI generated inside the app (the virtual keyboard).
    ui_midi: Producer<midi::MidiMessage>,
    output: OutputInfo,
    midi_name: Option<String>,
}

impl Engine {
    /// Open the default output device + MIDI input and start the stream.
    pub fn start() -> anyhow::Result<Self> {
        Self::start_with(None)
    }

    /// Open a specific output device (by name, or the default) + MIDI input.
    pub fn start_with(device_name: Option<&str>) -> anyhow::Result<Self> {
        let (proc_tx, proc_rx) = crossbeam_channel::unbounded();
        let (ack_tx, clear_ack) = crossbeam_channel::unbounded();
        let (midi_consumer, midi_conn) = midi::connect_default()?;
        let midi_name = midi_conn.as_ref().map(|c| c.name.clone());

        let (ui_producer, ui_consumer) = rtrb::RingBuffer::new(256);
        let (stream, output) =
            audio::build_output_stream(device_name, proc_rx, ack_tx, midi_consumer, ui_consumer)?;
        cpal::traits::StreamTrait::play(&stream)?;

        Ok(Self {
            _stream: stream,
            _midi: midi_conn,
            proc_tx,
            clear_ack,
            ui_midi: ui_producer,
            output,
            midi_name,
        })
    }

    /// Send a raw 3-byte MIDI message from the UI (virtual keyboard).
    pub fn send_midi(&mut self, message: midi::MidiMessage) {
        let _ = self.ui_midi.push(message);
    }

    /// Sample rate negotiated with the output device.
    pub fn sample_rate(&self) -> f64 {
        self.output.sample_rate
    }

    /// Maximum block size the engine will hand a plugin.
    pub fn max_block(&self) -> usize {
        self.output.max_block
    }

    /// Number of output channels on the current device.
    pub fn channels(&self) -> usize {
        self.output.channels
    }

    pub fn output_device_name(&self) -> &str {
        &self.output.device_name
    }

    pub fn midi_source_name(&self) -> Option<&str> {
        self.midi_name.as_deref()
    }

    /// Hand a freshly-activated plugin to the audio thread. The previous one is
    /// dropped on the audio thread once replaced.
    pub fn set_processor(&self, processor: Box<dyn AudioProcessor>) {
        let _ = self.proc_tx.send(ProcessorCommand::Set(processor));
    }

    /// Drop the active processor on the audio thread and wait until it is gone,
    /// so the caller may safely tear down the plugin behind it. The audio
    /// callback can be mid-`process` when this is called — destroying the
    /// plugin without this handshake segfaults (observed with VST3 on quit).
    /// Times out (with a warning) if the stream isn't running callbacks.
    pub fn clear_processor(&self) {
        // Drain acks left over from an earlier clear that timed out.
        while self.clear_ack.try_recv().is_ok() {}
        if self.proc_tx.send(ProcessorCommand::Clear).is_err() {
            return;
        }
        if self.clear_ack.recv_timeout(Duration::from_millis(500)).is_err() {
            log::warn!("audio thread did not confirm processor teardown");
        }
    }
}

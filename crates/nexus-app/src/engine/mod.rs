//! The realtime engine: owns the audio output stream and MIDI input, and hands
//! the active plugin's [`AudioProcessor`] to the audio thread.
//!
//! This lives in `nexus-app`, not `nexus-host` — the host library is
//! device-agnostic, and this is where real hardware I/O happens.

mod audio;
mod midi;

use crossbeam_channel::Sender;
use nexus_host::AudioProcessor;

pub use audio::OutputInfo;

/// Drives audio + MIDI for the standalone app.
pub struct Engine {
    _stream: cpal::Stream,
    _midi: Option<midi::MidiConnection>,
    proc_tx: Sender<Box<dyn AudioProcessor>>,
    output: OutputInfo,
    midi_name: Option<String>,
}

impl Engine {
    /// Open the default output device and MIDI input, and start the stream.
    pub fn start() -> anyhow::Result<Self> {
        let (proc_tx, proc_rx) = crossbeam_channel::unbounded();
        let (midi_consumer, midi_conn) = midi::connect_default()?;
        let midi_name = midi_conn.as_ref().map(|c| c.name.clone());

        let (stream, output) = audio::build_output_stream(proc_rx, midi_consumer)?;
        cpal::traits::StreamTrait::play(&stream)?;

        Ok(Self {
            _stream: stream,
            _midi: midi_conn,
            proc_tx,
            output,
            midi_name,
        })
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
        let _ = self.proc_tx.send(processor);
    }
}

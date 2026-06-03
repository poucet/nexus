//! CPAL output stream and the realtime callback that drives the active plugin.

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait};
use crossbeam_channel::Receiver;
use nexus_host::AudioProcessor;
use nexus_host::process::{AudioBuffer, Event, EventBuffer, ProcessBlock};

use super::midi::{MidiConsumer, MidiMessage};

/// Largest block we promise a plugin; the CPAL callback is clamped to it.
const MAX_BLOCK: usize = 8192;

/// Details of the running output stream, surfaced to the UI.
pub struct OutputInfo {
    pub device_name: String,
    pub sample_rate: f64,
    pub channels: usize,
    pub max_block: usize,
}

/// Build and return the default output stream. The returned stream is paused;
/// the caller plays it. New plugins are delivered to the audio thread by
/// sending an `AudioProcessor` on `proc_rx`.
pub fn build_output_stream(
    proc_rx: Receiver<Box<dyn AudioProcessor>>,
    mut midi: MidiConsumer,
) -> anyhow::Result<(cpal::Stream, OutputInfo)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no default audio output device")?;
    let device_name = device.name().unwrap_or_else(|_| "output".to_string());

    let supported = device
        .default_output_config()
        .context("no default output config")?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        anyhow::bail!(
            "unsupported sample format {:?} (only f32 for now)",
            supported.sample_format()
        );
    }

    let config: cpal::StreamConfig = supported.config();
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0 as f64;

    // State owned by the audio thread.
    let mut current: Option<Box<dyn AudioProcessor>> = None;
    let input = AudioBuffer::empty();
    let mut output = AudioBuffer::new(channels, MAX_BLOCK);
    let mut events = EventBuffer::with_capacity(256);

    let err_fn = |e| log::error!("audio stream error: {e}");

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Swap in any newly-loaded plugin and start it.
                while let Ok(mut processor) = proc_rx.try_recv() {
                    if let Err(e) = processor.start_processing() {
                        log::error!("start_processing: {e}");
                    }
                    current = Some(processor);
                }

                let frames = (data.len() / channels).min(MAX_BLOCK);

                // Collect MIDI received since the last callback.
                events.clear();
                while let Ok(msg) = midi.pop() {
                    if let Some(event) = parse_midi(msg) {
                        events.push(0, event);
                    }
                }

                if let Some(processor) = current.as_mut() {
                    output.set_frames(frames);
                    output.clear();
                    let mut block = ProcessBlock {
                        frames,
                        input: &input,
                        output: &mut output,
                        events: &events,
                    };
                    processor.process(&mut block);
                    interleave(&output, data, channels, frames);
                } else {
                    data.fill(0.0);
                }
            },
            err_fn,
            None,
        )
        .context("build output stream")?;

    Ok((
        stream,
        OutputInfo { device_name, sample_rate, channels, max_block: MAX_BLOCK },
    ))
}

/// Write the plugin's planar output into CPAL's interleaved buffer. A mono
/// plugin output is duplicated across all device channels.
fn interleave(output: &AudioBuffer, data: &mut [f32], channels: usize, frames: usize) {
    let produced = output.channel_count();
    for frame in 0..frames {
        for ch in 0..channels {
            let src_ch = if ch < produced { ch } else { 0 };
            let sample = output
                .channel(src_ch)
                .and_then(|c| c.get(frame))
                .copied()
                .unwrap_or(0.0);
            data[frame * channels + ch] = sample;
        }
    }
}

/// Parse a 3-byte MIDI channel-voice message into a façade [`Event`].
fn parse_midi(msg: MidiMessage) -> Option<Event> {
    let status = msg[0] & 0xF0;
    let channel = (msg[0] & 0x0F) as i16;
    match status {
        0x90 if msg[2] > 0 => Some(Event::note_on(channel, msg[1] as i16, msg[2] as f64 / 127.0)),
        0x80 | 0x90 => Some(Event::note_off(channel, msg[1] as i16, msg[2] as f64 / 127.0)),
        0x00 => None, // empty/blank
        _ => Some(Event::Midi { port_index: 0, data: msg }),
    }
}

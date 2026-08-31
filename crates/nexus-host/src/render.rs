//! Offline plugin audio rendering engine.
//!
//! Executes a [`PluginInstance`] / [`AudioProcessor`] at maximum CPU speed without
//! requiring an active hardware audio device or sound driver.

use std::io::Cursor;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fugue::{CompiledFugue, FugueEvent};
use crate::host::{AudioConfig, HostError, PluginInstance};
use crate::process::{AudioBuffer, Event, EventBuffer, NoteEventKind, ProcessBlock};

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
}

#[derive(Debug, Clone)]
struct ScheduledHostEvent {
    sample_offset: usize,
    event: Event,
}

/// Render `instance` offline according to `opts`.
pub fn render_offline(
    instance: &mut dyn PluginInstance,
    opts: &RenderOptions,
) -> Result<RenderResult, HostError> {
    let sample_rate = opts.sample_rate;
    let block_size = opts.block_size.max(64);
    let tempo = opts.tempo_bpm.max(1.0);
    let beats_per_second = tempo / 60.0;
    let samples_per_beat = sample_rate / beats_per_second;

    let total_frames = ((opts.fugue.duration_beats * samples_per_beat).round() as usize).max(block_size);
    let duration_s = total_frames as f64 / sample_rate;

    // 1. Compile timed events to sample positions
    let mut scheduled = Vec::new();
    for timed in &opts.fugue.events {
        let onset_sample = (timed.beat_offset * samples_per_beat).round() as usize;

        match &timed.event {
            FugueEvent::NoteOn { channel, note, velocity } => {
                scheduled.push(ScheduledHostEvent {
                    sample_offset: onset_sample,
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
                    event: Event::Note {
                        kind: NoteEventKind::On,
                        note_id: -1,
                        port_index: 0,
                        channel: *channel as i16,
                        key: *note as i16,
                        velocity: *velocity as f64 / 127.0,
                    },
                });

                let off_sample = ((timed.beat_offset + duration_beats) * samples_per_beat).round() as usize;
                scheduled.push(ScheduledHostEvent {
                    sample_offset: off_sample,
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
            FugueEvent::Cc { channel, controller, value, .. } => {
                scheduled.push(ScheduledHostEvent {
                    sample_offset: onset_sample,
                    event: Event::Midi {
                        port_index: 0,
                        data: [0xB0 | (channel & 0x0F), *controller, *value],
                    },
                });
            }
            FugueEvent::Param { id, value, .. } => {
                scheduled.push(ScheduledHostEvent {
                    sample_offset: onset_sample,
                    event: Event::ParamValue {
                        param_id: *id,
                        note_id: -1,
                        port_index: 0,
                        channel: 0,
                        key: 0,
                        value: *value as f64,
                    },
                });
            }
            _ => {}
        }
    }

    // Sort scheduled events chronologically
    scheduled.sort_by_key(|s| s.sample_offset);

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

    while current_frame < total_frames {
        let block_frames = (total_frames - current_frame).min(block_size);
        output_audio.set_frames(block_frames);
        output_audio.clear();
        input_events.clear();

        let block_end = current_frame + block_frames;

        while event_idx < scheduled.len() && scheduled[event_idx].sample_offset < block_end {
            let ev = &scheduled[event_idx];
            if ev.sample_offset >= current_frame {
                let frame_in_block = (ev.sample_offset - current_frame) as u32;
                input_events.push(frame_in_block, ev.event.clone());
            }
            event_idx += 1;
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
    use super::*;
    use crate::fugue::CompositeFugue;
    use crate::host::AudioProcessor;

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
        assert_eq!(result.frames, 48000); // 2 beats at 120 bpm = 1 second = 48000 samples
        assert!(!result.is_silent);
        assert!(result.peak_db > -7.0 && result.peak_db < -5.0); // ~0.5 amplitude ≈ -6 dBFS
        assert!(!result.wav_bytes.is_empty());
        assert_eq!(&result.wav_bytes[0..4], b"RIFF");
    }
}

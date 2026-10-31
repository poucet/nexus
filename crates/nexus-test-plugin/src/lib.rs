//! `nexus.test.gain`: a stereo CLAP gain that reports what the host gave it.
//!
//! Built from clack's `gain` example (MIT OR Apache-2.0). nexus-host's tests
//! load it by path (`crates/nexus-host/tests/common`), so the default gates
//! need no installed plugin. On top of the gain it:
//!
//! - **behaves like a JUCE plugin** on the audio thread: every parameter event
//!   it receives in `process()` calls the host's `request_callback()`, which
//!   is what made nexus allocate on the audio thread (`I-101`);
//! - **reports what the host sent** through read-only parameters the host can
//!   read with `param_value`: the transport's tempo, play state and position
//!   (`I-104`), the sample rate of the last activation (`I-102`), how
//!   many main-thread callbacks the host has run, and the last MIDI 2.0
//!   packet it delivered (`N8`), and what the host's `thread-check` said
//!   in `process()` and `on_main_thread` (`N10`).
#![deny(unsafe_code)]

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_extensions::thread_check::HostThreadCheck;
use clack_plugin::events::event_types::TransportFlags;
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

/// The plugin id `nexus-host`'s tests load.
pub const ID: &str = "nexus.test.gain";

/// The parameters, by id. Only `VOLUME` is writable; the rest are probes.
pub mod param {
    /// Gain, 0..=1 (default 1).
    pub const VOLUME: u32 = 1;
    /// Tempo of the last transport `process()` saw, in BPM (0 = none).
    pub const TEMPO: u32 = 2;
    /// 1 if the last transport said playing, 0 if stopped, -1 if no transport.
    pub const PLAYING: u32 = 3;
    /// Song position of the last transport, in beats.
    pub const BEATS: u32 = 4;
    /// Sample rate of the last activation.
    pub const SAMPLE_RATE: u32 = 5;
    /// How many times the host ran `on_main_thread`.
    pub const CALLBACKS: u32 = 6;
    /// The first word of the last MIDI 2.0 (UMP) packet `process()` saw.
    pub const MIDI2: u32 = 7;
    /// What the host's `thread-check` said inside the last `process()`:
    /// 1 for the audio thread + 2 for the main thread (-1: no `thread-check`).
    pub const THREAD_IN_PROCESS: u32 = 8;
    /// The same, inside the last `on_main_thread`.
    pub const THREAD_ON_MAIN: u32 = 9;
}

/// `(id, name, writable)`, in `params` index order.
const PARAMS: [(u32, &[u8], bool); 9] = [
    (param::VOLUME, b"Volume", true),
    (param::TEMPO, b"Tempo", false),
    (param::PLAYING, b"Playing", false),
    (param::BEATS, b"Beats", false),
    (param::SAMPLE_RATE, b"Sample rate", false),
    (param::CALLBACKS, b"Callbacks", false),
    (param::MIDI2, b"MIDI 2.0", false),
    (param::THREAD_IN_PROCESS, b"Thread in process", false),
    (param::THREAD_ON_MAIN, b"Thread on main", false),
];

/// One `f64` slot per parameter, shared by every thread of the plugin.
pub struct Shared {
    /// Indexed like [`PARAMS`].
    values: [AtomicU64; PARAMS.len()],
    /// The host's `thread-check`, if it offers one.
    thread_check: Option<HostThreadCheck>,
}

impl Shared {
    /// The slot index of parameter `id`.
    fn slot(id: u32) -> Option<usize> {
        PARAMS.iter().position(|(p, _, _)| *p == id)
    }

    /// Read parameter `id`.
    fn get(&self, id: u32) -> Option<f64> {
        Self::slot(id).map(|i| f64::from_bits(self.values[i].load(Ordering::Relaxed)))
    }

    /// Write parameter `id` (the probes are written by the plugin itself).
    fn set(&self, id: u32, value: f64) {
        if let Some(i) = Self::slot(id) {
            self.values[i].store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// What `thread-check` says about the calling thread (see
    /// [`param::THREAD_IN_PROCESS`]).
    fn thread(&self, host: &HostSharedHandle) -> f64 {
        let Some(check) = self.thread_check else { return -1.0 };
        let audio = check.is_audio_thread(host).unwrap_or(false);
        let main = check.is_main_thread(host).unwrap_or(false);
        f64::from(u8::from(audio) + 2 * u8::from(main))
    }

    /// Apply a host `ParamValue` event to `VOLUME`; true if it was one.
    fn handle(&self, event: &UnknownEvent) -> bool {
        if let Some(CoreEventSpace::ParamValue(event)) = event.as_core_event() {
            if event.param_id().map(|id| id.get()) == Some(param::VOLUME) {
                self.set(param::VOLUME, event.value().clamp(0.0, 1.0));
            }
            return true;
        }
        false
    }
}

impl PluginShared<'_> for Shared {}

/// The plugin type.
pub struct TestGain;

impl Plugin for TestGain {
    type AudioProcessor<'a> = Processor<'a>;
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginState>();
    }
}

impl DefaultPluginFactory for TestGain {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;
        PluginDescriptor::new(ID, "nexus test gain").with_features([AUDIO_EFFECT, STEREO])
    }

    fn new_shared(host: HostSharedHandle<'_>) -> Result<Shared, PluginError> {
        let shared =
            Shared { values: Default::default(), thread_check: host.get_extension() };
        shared.set(param::VOLUME, 1.0);
        shared.set(param::PLAYING, -1.0);
        Ok(shared)
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a Shared,
    ) -> Result<MainThread<'a>, PluginError> {
        Ok(MainThread { shared, host })
    }
}

/// The audio-thread half.
pub struct Processor<'a> {
    /// The plugin's shared slots.
    shared: &'a Shared,
    /// Used the way JUCE's CLAP wrapper does: `request_callback` per change.
    host: HostAudioProcessorHandle<'a>,
}

impl<'a> PluginAudioProcessor<'a, Shared, MainThread<'a>> for Processor<'a> {
    fn activate(
        host: HostAudioProcessorHandle<'a>,
        _main_thread: &mut MainThread,
        shared: &'a Shared,
        config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        shared.set(param::SAMPLE_RATE, config.sample_rate);
        Ok(Self { shared, host })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        self.shared.set(param::THREAD_IN_PROCESS, self.shared.thread(&self.host));
        match process.transport {
            Some(t) => {
                self.shared.set(param::TEMPO, t.tempo);
                self.shared.set(param::BEATS, t.song_pos_beats.to_float());
                let playing = t.flags.contains(TransportFlags::IS_PLAYING);
                self.shared.set(param::PLAYING, if playing { 1.0 } else { 0.0 });
            }
            None => self.shared.set(param::PLAYING, -1.0),
        }

        let mut port_pair =
            audio.port_pair(0).ok_or(PluginError::Message("No input/output ports found"))?;
        let mut channels =
            port_pair.channels()?.into_f32().ok_or(PluginError::Message("Expected f32"))?;
        let mut buffers = [None, None];
        for (pair, buf) in channels.iter_mut().zip(&mut buffers) {
            *buf = match pair {
                ChannelPair::InputOnly(_) | ChannelPair::OutputOnly(_) => None,
                ChannelPair::InPlace(b) => Some(b),
                ChannelPair::InputOutput(i, o) => {
                    o.copy_from_slice(i);
                    Some(o)
                }
            }
        }

        for batch in events.input.batch() {
            for event in batch.events() {
                if self.shared.handle(event) {
                    self.host.request_callback();
                }
                if let Some(CoreEventSpace::Midi2(midi)) = event.as_core_event() {
                    self.shared.set(param::MIDI2, f64::from(midi.data()[0]));
                }
            }
            // Scale only this batch's samples (clack's example scales the whole
            // buffer per batch, compounding the gain when events split a block).
            let volume = self.shared.get(param::VOLUME).unwrap_or(1.0) as f32;
            for buf in buffers.iter_mut().flatten() {
                let end = batch.next_batch_first_sample().unwrap_or(buf.len()).min(buf.len());
                for sample in &mut buf[batch.first_sample().min(end)..end] {
                    *sample *= volume;
                }
            }
        }
        Ok(ProcessStatus::ContinueIfNotQuiet)
    }
}

impl PluginAudioProcessorParams for Processor<'_> {
    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        for event in input {
            self.shared.handle(event);
        }
    }
}

/// The main-thread half.
pub struct MainThread<'a> {
    /// The plugin's shared slots.
    shared: &'a Shared,
    /// For `thread-check` on this side.
    host: HostMainThreadHandle<'a>,
}

impl<'a> PluginMainThread<'a, Shared> for MainThread<'a> {
    fn on_main_thread(&mut self) {
        self.shared.set(param::THREAD_ON_MAIN, self.shared.thread(&self.host));
        let n = self.shared.get(param::CALLBACKS).unwrap_or(0.0);
        self.shared.set(param::CALLBACKS, n + 1.0);
    }
}

impl PluginAudioPortsImpl for MainThread<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, _is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index == 0 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(0),
                name: b"main",
                channel_count: 2,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: None,
            });
        }
    }
}

impl PluginMainThreadParams for MainThread<'_> {
    fn count(&mut self) -> u32 {
        PARAMS.len() as u32
    }

    fn get_info(&mut self, index: u32, info: &mut ParamInfoWriter) {
        let Some(&(id, name, writable)) = PARAMS.get(index as usize) else {
            return;
        };
        let (flags, min, max) = if writable {
            (ParamInfoFlags::IS_AUTOMATABLE, 0.0, 1.0)
        } else {
            (ParamInfoFlags::IS_READONLY, f64::MIN, f64::MAX)
        };
        info.set(&ParamInfo {
            id: id.into(),
            flags,
            cookie: Default::default(),
            name,
            module: b"",
            min_value: min,
            max_value: max,
            default_value: if writable { 1.0 } else { 0.0 },
        })
    }

    fn get_value(&mut self, id: ClapId) -> Option<f64> {
        self.shared.get(id.get())
    }

    fn value_to_text(
        &mut self,
        _id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        write!(writer, "{value}")
    }

    fn text_to_value(&mut self, _id: ClapId, text: &CStr) -> Option<f64> {
        text.to_str().ok()?.trim().parse().ok()
    }

    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        for event in input {
            self.shared.handle(event);
        }
    }
}

impl PluginStateImpl for MainThread<'_> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let volume = self.shared.get(param::VOLUME).unwrap_or(1.0);
        output.write_all(&volume.to_le_bytes())?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut buf = [0; 8];
        input.read_exact(&mut buf)?;
        self.shared.set(param::VOLUME, f64::from_le_bytes(buf));
        Ok(())
    }
}

/// The plugin's CLAP entry, for a host that loads it in process
/// (`nexus_host::clap::instance::ClapPluginInstance::load_clack::<Entry>`).
pub type Entry = SinglePluginEntry<TestGain>;

clack_export_entry!(Entry);

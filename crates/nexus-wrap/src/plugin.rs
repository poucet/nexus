//! CLAP plugin implementation for nexus-wrap.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::sync::Arc;

use clack_extensions::audio_ports::*;
use clack_extensions::note_ports::*;
use clack_extensions::params::*;
use clack_extensions::state::*;
use clack_plugin::events::event_types::*;
use clack_plugin::events::{Event as ClapEvent, Match};
use clack_plugin::prelude::*;
use nexus_host::host::{AudioConfig, AudioProcessor, PluginFormat};
use nexus_host::process::{AudioBuffer, Event as HostEvent, EventBuffer, NoteEventKind, ProcessBlock};
use nexus_tools::bridge::{HostBridge, HostRequest, MainThreadHost};
use nexus_tools::{ToolRegistry, ToolsService};

use crate::bridge::InnerPluginState;
use crate::server::start_servers;
use crate::state::WrapStateEnvelope;

pub struct NexusWrapPlugin;

impl Plugin for NexusWrapPlugin {
    type AudioProcessor<'a> = NexusWrapAudioProcessor<'a>;
    type Shared<'a> = NexusWrapShared<'a>;
    type MainThread<'a> = NexusWrapMainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Self::Shared<'_>>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>()
            .register::<PluginParams>()
            .register::<PluginState>();
    }
}

impl DefaultPluginFactory for NexusWrapPlugin {
    fn get_descriptor() -> PluginDescriptor {
        PluginDescriptor::new("com.simply-chris.nexus", "Nexus")
            .with_vendor("Simply Chris")
            .with_version(env!("CARGO_PKG_VERSION"))
    }

    fn new_shared<'a>(host: HostSharedHandle<'a>) -> Result<Self::Shared<'a>, PluginError> {
        let (bridge, rx) = HostBridge::channel(Arc::new(|| {}));

        let service = Arc::new(ToolsService::new(bridge));
        let registry = Arc::new(ToolRegistry::standard(service));

        // Start background MCP and REST servers
        start_servers(registry.clone());

        Ok(NexusWrapShared {
            _host: host,
            rx,
            _registry: registry,
        })
    }


    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a Self::Shared<'a>,
    ) -> Result<Self::MainThread<'a>, PluginError> {
        Ok(NexusWrapMainThread {
            _host: host,
            shared,
            inner: InnerPluginState::new(),
        })
    }
}

pub struct NexusWrapShared<'a> {
    pub _host: HostSharedHandle<'a>,
    pub rx: crossbeam_channel::Receiver<HostRequest>,
    pub _registry: Arc<ToolRegistry>,
}

impl<'a> PluginShared<'a> for NexusWrapShared<'a> {}

pub struct NexusWrapMainThread<'a> {
    pub _host: HostMainThreadHandle<'a>,
    pub shared: &'a NexusWrapShared<'a>,
    pub inner: InnerPluginState,
}

impl<'a> PluginMainThread<'a, NexusWrapShared<'a>> for NexusWrapMainThread<'a> {
    fn on_main_thread(&mut self) {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
    }
}

pub struct NexusWrapAudioProcessor<'a> {
    pub _shared: &'a NexusWrapShared<'a>,
    pub inner_processor: Option<Box<dyn AudioProcessor>>,
    pub input_buf: AudioBuffer,
    pub output_buf: AudioBuffer,
    pub events_buf: EventBuffer,
    pub max_frames: usize,
}

impl<'a> PluginAudioProcessor<'a, NexusWrapShared<'a>, NexusWrapMainThread<'a>> for NexusWrapAudioProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        main_thread: &mut NexusWrapMainThread<'a>,
        shared: &'a NexusWrapShared<'a>,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        nexus_tools::bridge::drain(&shared.rx, &mut main_thread.inner);

        let max_frames = audio_config.max_frames_count as usize;
        let inner_processor = main_thread
            .inner
            .activate(AudioConfig {
                sample_rate: audio_config.sample_rate,
                max_block_frames: max_frames,
            })
            .ok();

        Ok(Self {
            _shared: shared,
            inner_processor,
            input_buf: AudioBuffer::new(2, max_frames),
            output_buf: AudioBuffer::new(2, max_frames),
            events_buf: EventBuffer::with_capacity(256),
            max_frames,
        })
    }

    fn process(
        &mut self,
        _process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {

        let frames = audio
            .port_pair(0)
            .and_then(|mut p| match p.channels() {
                Ok(SampleType::F32(paired)) => paired.into_iter().next().map(|cp| match cp {
                    ChannelPair::InputOnly(s) => s.len(),
                    ChannelPair::OutputOnly(s) => s.len(),
                    ChannelPair::InPlace(s) => s.len(),
                    ChannelPair::InputOutput(s, _) => s.len(),
                }),
                _ => None,
            })
            .unwrap_or(self.max_frames)
            .min(self.max_frames);


        self.input_buf.set_frames(frames);
        self.output_buf.set_frames(frames);
        self.input_buf.clear();
        self.output_buf.clear();

        // 1. Copy incoming audio to input buffer
        if let Some(mut pair) = audio.port_pair(0) {
            if let Ok(SampleType::F32(paired)) = pair.channels() {
                for (ch_idx, channel_pair) in paired.into_iter().enumerate() {
                    if ch_idx >= 2 {
                        break;
                    }
                    match channel_pair {
                        ChannelPair::InputOnly(in_slice) | ChannelPair::InputOutput(in_slice, _) => {
                            if let Some(dst) = self.input_buf.channel_mut(ch_idx) {
                                let n = in_slice.len().min(frames);
                                dst[..n].copy_from_slice(&in_slice[..n]);
                            }
                        }
                        ChannelPair::InPlace(slice) => {
                            if let Some(dst) = self.input_buf.channel_mut(ch_idx) {
                                let n = slice.len().min(frames);
                                dst[..n].copy_from_slice(&slice[..n]);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // 2. Parse incoming events
        self.events_buf.clear();
        for event in events.input {
            if let Some(note_on) = event.as_event::<NoteOnEvent>() {
                let time = note_on.header().time();
                let channel = match note_on.channel() {
                    Match::Specific(c) => c as i16,
                    _ => 0,
                };
                let key = match note_on.key() {
                    Match::Specific(k) => k as i16,
                    _ => 60,
                };
                let velocity = note_on.velocity();
                let note_id = match note_on.note_id() {
                    Match::Specific(id) => id as i32,
                    _ => -1,
                };
                self.events_buf.push(
                    time,
                    HostEvent::Note {
                        kind: NoteEventKind::On,
                        note_id,
                        port_index: 0,
                        channel,
                        key,
                        velocity,
                    },
                );
            } else if let Some(note_off) = event.as_event::<NoteOffEvent>() {
                let time = note_off.header().time();
                let channel = match note_off.channel() {
                    Match::Specific(c) => c as i16,
                    _ => 0,
                };
                let key = match note_off.key() {
                    Match::Specific(k) => k as i16,
                    _ => 60,
                };
                let velocity = note_off.velocity();
                let note_id = match note_off.note_id() {
                    Match::Specific(id) => id as i32,
                    _ => -1,
                };
                self.events_buf.push(
                    time,
                    HostEvent::Note {
                        kind: NoteEventKind::Off,
                        note_id,
                        port_index: 0,
                        channel,
                        key,
                        velocity,
                    },
                );
            } else if let Some(choke) = event.as_event::<NoteChokeEvent>() {
                let time = choke.header().time();
                let channel = match choke.channel() {
                    Match::Specific(c) => c as i16,
                    _ => 0,
                };
                let key = match choke.key() {
                    Match::Specific(k) => k as i16,
                    _ => 60,
                };
                let note_id = match choke.note_id() {
                    Match::Specific(id) => id as i32,
                    _ => -1,
                };
                self.events_buf.push(
                    time,
                    HostEvent::Note {
                        kind: NoteEventKind::Choke,
                        note_id,
                        port_index: 0,
                        channel,
                        key,
                        velocity: 0.0,
                    },
                );
            } else if let Some(midi) = event.as_event::<MidiEvent>() {
                self.events_buf.push(
                    midi.header().time(),
                    HostEvent::Midi {
                        port_index: midi.port_index(),
                        data: midi.data(),
                    },
                );
            } else if let Some(param_val) = event.as_event::<ParamValueEvent>() {
                let param_id = param_val.param_id().unwrap_or(ClapId::new(0)).get();
                self.events_buf.push(
                    param_val.header().time(),
                    HostEvent::ParamValue {
                        param_id,
                        note_id: -1,
                        port_index: 0,
                        channel: 0,
                        key: 0,
                        value: param_val.value(),
                    },
                );
            }
        }



        // 3. Process through inner processor or pass-through
        if let Some(processor) = self.inner_processor.as_mut() {
            let mut block = ProcessBlock {
                frames,
                input: &self.input_buf,
                output: &mut self.output_buf,
                events: &self.events_buf,
            };
            processor.process(&mut block);
        } else {
            // Pass-through when no inner plugin is loaded
            for ch in 0..2 {
                if let (Some(src), Some(dst)) = (self.input_buf.channel(ch), self.output_buf.channel_mut(ch)) {
                    let n = src.len().min(dst.len());
                    dst[..n].copy_from_slice(&src[..n]);
                }
            }
        }

        // 4. Copy rendered audio back to DAW output buffers
        if let Some(mut pair) = audio.port_pair(0) {
            if let Ok(SampleType::F32(paired)) = pair.channels() {
                for (ch_idx, channel_pair) in paired.into_iter().enumerate() {
                    if ch_idx >= 2 {
                        break;
                    }
                    if let Some(src) = self.output_buf.channel(ch_idx) {
                        let n = src.len().min(frames);
                        match channel_pair {
                            ChannelPair::OutputOnly(out_slice) | ChannelPair::InputOutput(_, out_slice) => {
                                out_slice[..n].copy_from_slice(&src[..n]);
                            }
                            ChannelPair::InPlace(slice) => {
                                slice[..n].copy_from_slice(&src[..n]);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(ProcessStatus::ContinueIfNotQuiet)
    }

    fn deactivate(self, main_thread: &mut NexusWrapMainThread<'a>) {
        main_thread.inner.deactivate();
    }
}

impl PluginAudioPortsImpl for NexusWrapMainThread<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index == 0 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(if is_input { 0 } else { 1 }),
                name: if is_input { b"Audio Input\0" } else { b"Audio Output\0" },
                channel_count: 2,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: None,
            });
        }
    }
}

impl PluginNotePortsImpl for NexusWrapMainThread<'_> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input { 1 } else { 0 }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter) {
        if is_input && index == 0 {
            writer.set(&NotePortInfo {
                id: ClapId::new(0),
                name: b"Note Input\0",
                preferred_dialect: Some(NoteDialect::Clap),
                supported_dialects: NoteDialects::CLAP | NoteDialects::MIDI,
            });
        }
    }
}

impl PluginMainThreadParams for NexusWrapMainThread<'_> {
    fn count(&mut self) -> u32 {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
        self.inner.params().len() as u32
    }

    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
        let params = self.inner.params();
        if let Some(param) = params.get(param_index as usize) {
            let mut flags = ParamInfoFlags::empty();
            if param.is_stepped {
                flags.insert(ParamInfoFlags::IS_STEPPED);
            }
            if param.is_readonly {
                flags.insert(ParamInfoFlags::IS_READONLY);
            }
            if param.is_hidden {
                flags.insert(ParamInfoFlags::IS_HIDDEN);
            }
            if param.is_bypass {
                flags.insert(ParamInfoFlags::IS_BYPASS);
            }
            if param.is_automatable {
                flags.insert(ParamInfoFlags::IS_AUTOMATABLE);
            }
            if param.is_modulatable {
                flags.insert(ParamInfoFlags::IS_MODULATABLE);
            }

            let mut name_bytes = [0u8; 256];
            let n = param.name.as_bytes().len().min(255);
            name_bytes[..n].copy_from_slice(&param.name.as_bytes()[..n]);

            let mut module_bytes = [0u8; 1024];
            let m = param.module.as_bytes().len().min(1023);
            module_bytes[..m].copy_from_slice(&param.module.as_bytes()[..m]);

            info.set(&clack_extensions::params::ParamInfo {
                id: ClapId::from_raw(param.id).unwrap_or(ClapId::new(param.id)),
                name: &name_bytes[..n],
                module: &module_bytes[..m],
                min_value: param.min_value,
                max_value: param.max_value,
                default_value: param.default_value,
                flags,
                cookie: Default::default(),
            });
        }
    }

    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
        self.inner.param_value(param_id.get())
    }

    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        output: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
        if let Some(text) = self.inner.param_value_to_text(param_id.get(), value) {
            write!(output, "{text}")
        } else {
            write!(output, "{value:.3}")
        }
    }

    fn text_to_value(&mut self, _param_id: ClapId, text: &CStr) -> Option<f64> {
        text.to_str().ok()?.trim().parse::<f64>().ok()
    }

    fn flush(&mut self, _input_events: &InputEvents, _output_events: &mut OutputEvents) {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
    }
}

impl<'a> PluginAudioProcessorParams for NexusWrapAudioProcessor<'a> {
    fn flush(&mut self, _input_events: &InputEvents, _output_events: &mut OutputEvents) {}
}

impl PluginStateImpl for NexusWrapMainThread<'_> {
    fn save(&mut self, output: &mut clack_plugin::stream::OutputStream) -> Result<(), PluginError> {
        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);

        let inner_state = self
            .inner
            .instance
            .as_mut()
            .and_then(|i| i.save_state().ok())
            .unwrap_or_default();

        let envelope = WrapStateEnvelope::new(
            self.inner.loaded.as_ref().map(|p| p.descriptor.id.clone()),
            self.inner.loaded.as_ref().map(|p| match p.format {
                PluginFormat::Clap => "clap".into(),
                PluginFormat::Vst3 => "vst3".into(),
            }),
            self.inner.loaded.as_ref().and_then(|p| p.descriptor.name.clone()),
            inner_state,
        );

        envelope.save(output)
    }

    fn load(&mut self, input: &mut clack_plugin::stream::InputStream) -> Result<(), PluginError> {
        let envelope = WrapStateEnvelope::load(input)?;

        if let Some(ref target) = envelope.plugin_id {
            let already_loaded = self.inner.loaded.as_ref().is_some_and(|p| p.descriptor.id == *target);
            if !already_loaded {
                let _ = self.inner.load_plugin(target);
            }

            if let Some(instance) = self.inner.instance.as_mut() {
                let _ = instance.load_state(&envelope.inner_state);
            }
        }

        nexus_tools::bridge::drain(&self.shared.rx, &mut self.inner);
        Ok(())
    }
}

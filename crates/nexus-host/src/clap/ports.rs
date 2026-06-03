//! Reading a CLAP plugin's audio- and note-port layout via its extensions.
//!
//! The host needs the per-port channel counts to size processing buffers, and
//! the main note port to route note/MIDI events.

use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::note_ports::{NoteDialects, NotePortInfoBuffer, PluginNotePorts};
use clack_host::prelude::PluginMainThreadHandle;

/// A plugin's I/O layout, read once after instantiation.
#[derive(Debug, Clone, Default)]
pub struct IoLayout {
    /// Channel count of each input audio port.
    pub input_ports: Vec<u16>,
    /// Channel count of each output audio port.
    pub output_ports: Vec<u16>,
    /// Index of the main note input port, if the plugin accepts notes.
    pub note_port: Option<u16>,
    /// Whether that note port prefers raw MIDI over CLAP note events.
    pub prefers_midi: bool,
}

impl IoLayout {
    /// Query the layout from the plugin's `audio-ports` and `note-ports`
    /// extensions. Missing extensions yield empty port lists / no note port.
    pub fn query(handle: &mut PluginMainThreadHandle) -> Self {
        let mut layout = IoLayout::default();

        if let Some(ports) = handle.get_extension::<PluginAudioPorts>() {
            let mut buffer = AudioPortInfoBuffer::new();
            for is_input in [true, false] {
                let count = ports.count(handle, is_input);
                for i in 0..count {
                    if let Some(info) = ports.get(handle, i, is_input, &mut buffer) {
                        let channels = info.channel_count as u16;
                        if is_input {
                            layout.input_ports.push(channels);
                        } else {
                            layout.output_ports.push(channels);
                        }
                    }
                }
            }
        }

        if let Some(note_ports) = handle.get_extension::<PluginNotePorts>() {
            let mut buffer = NotePortInfoBuffer::new();
            let count = note_ports.count(handle, true).min(u16::MAX as u32);
            for i in 0..count {
                let Some(info) = note_ports.get(handle, i, true, &mut buffer) else {
                    continue;
                };
                if info
                    .supported_dialects
                    .intersects(NoteDialects::CLAP | NoteDialects::MIDI)
                {
                    layout.note_port = Some(i as u16);
                    layout.prefers_midi = !info.supported_dialects.intersects(NoteDialects::CLAP);
                    break;
                }
            }
        }

        layout
    }
}

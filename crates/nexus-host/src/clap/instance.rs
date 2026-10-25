//! Instantiating and activating a CLAP plugin behind the host façade.
//!
//! [`ClapPluginInstance`] is the main-thread half (implements
//! [`crate::host::PluginInstance`]); [`activate`](ClapPluginInstance::activate)
//! produces the `Send` audio half ([`ClapAudioProcessor`]) that the engine
//! moves onto the realtime thread. The block-processing body is filled in by
//! T9; this task covers load → activate → start/stop.

use std::ffi::CString;
use std::path::Path;

use clack_host::prelude::*;
use crossbeam_channel::{Receiver, unbounded};

#[cfg(target_os = "macos")]
use crate::clap::gui::ClapEditor;
use crate::clap::host::{
    MainThreadMessage, NexusHost, NexusHostMainThread, NexusHostShared, host_info,
};
use crate::clap::ports::IoLayout;
use crate::clap::process::ClapAudioProcessor;
use crate::host::{
    AudioConfig, AudioProcessor, EventSupport, HostError, PluginInstance as HostPluginInstance,
};

/// A loaded CLAP plugin (main-thread side).
pub struct ClapPluginInstance {
    /// The loaded bundle, kept alive for as long as the instance.
    _entry: PluginEntry,
    /// The clack plugin instance.
    instance: PluginInstance<NexusHost>,
    /// Drains main-thread callback requests posted by the plugin.
    messages: Receiver<MainThreadMessage>,
    /// The plugin's floating editor window, if open.
    #[cfg(target_os = "macos")]
    editor: Option<ClapEditor>,
    /// Parameter change ring producer, created on activate.
    param_producer: Option<rtrb::Producer<crate::host::ParamChange>>,
}

impl ClapPluginInstance {
    /// Load the `.clap` bundle at `path` and instantiate the plugin whose
    /// factory id is `plugin_id`.
    pub fn load(path: &Path, plugin_id: &str) -> Result<Self, HostError> {
        // SAFETY: loading runs the bundle's entry initializer; `path` comes from
        // scanning the standard plugin directories.
        let entry = unsafe { PluginEntry::load(path) }
            .map_err(|e| HostError::Load(format!("{}: {e}", path.display())))?;

        let plugin_id = CString::new(plugin_id)
            .map_err(|_| HostError::Load("plugin id contains a NUL byte".into()))?;

        let (sender, receiver) = unbounded();
        let instance = PluginInstance::<NexusHost>::new(
            |_| NexusHostShared::new(sender.clone()),
            |shared| NexusHostMainThread::new(shared),
            &entry,
            &plugin_id,
            &host_info(),
        )
        .map_err(|e| HostError::Load(format!("{}: {e}", path.display())))?;

        Ok(Self {
            _entry: entry,
            instance,
            messages: receiver,
            #[cfg(target_os = "macos")]
            editor: None,
            param_producer: None,
        })
    }
}

impl HostPluginInstance for ClapPluginInstance {
    /// CLAP carries both: every `Event::Midi` is forwarded verbatim as a clack
    /// `MidiEvent`, and `Event::NoteExpression` becomes a `NoteExpressionEvent`
    /// (`clap/process.rs`).
    fn event_support(&self) -> EventSupport {
        EventSupport { raw_midi: true, note_expression: true }
    }

    fn activate(&mut self, config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError> {
        let io = {
            let mut handle = self.instance.plugin_handle();
            IoLayout::query(&mut handle)
        };

        let clap_config = PluginAudioConfiguration {
            sample_rate: config.sample_rate,
            min_frames_count: 1,
            max_frames_count: config.max_block_frames as u32,
        };

        let (producer, consumer) = rtrb::RingBuffer::new(1024);
        self.param_producer = Some(producer);

        let stopped = self
            .instance
            .activate(|_, _| (), clap_config)
            .map_err(|e| HostError::Load(format!("activate failed: {e}")))?;

        Ok(Box::new(ClapAudioProcessor::new(
            stopped.into(),
            io,
            config.max_block_frames,
            consumer,
        )))
    }

    fn deactivate(&mut self) {
        self.param_producer = None;
        // clack tears the plugin down when the instance is dropped; the engine
        // drops the (stopped) audio processor before this. Explicit
        // processor-returning deactivation can be revisited if needed.
    }

    fn param_producer(&mut self) -> Option<&mut rtrb::Producer<crate::host::ParamChange>> {
        self.param_producer.as_mut()
    }

    fn take_param_producer(&mut self) -> Option<rtrb::Producer<crate::host::ParamChange>> {
        self.param_producer.take()
    }

    fn set_param(&mut self, id: u32, value: f64) -> Result<(), HostError> {
        if let Some(producer) = &mut self.param_producer {
            let _ = producer.push(crate::host::ParamChange {
                id,
                value,
                offset_frames: 0,
            });
        }
        Ok(())
    }

    fn params(&mut self) -> Vec<crate::host::ParamInfo> {
        crate::clap::params::enumerate(&mut self.instance)
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        crate::clap::params::value(&mut self.instance, id)
    }

    fn param_value_to_text(&mut self, id: u32, value: f64) -> Option<String> {
        crate::clap::params::value_to_text(&mut self.instance, id, value)
    }

    fn param_text_to_value(&mut self, id: u32, text: &str) -> Option<f64> {
        crate::clap::params::text_to_value(&mut self.instance, id, text)
    }

    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        crate::clap::state::save(&mut self.instance)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), HostError> {
        crate::clap::state::load(&mut self.instance, data)
    }

    fn latency(&mut self) -> u32 {
        crate::clap::info::latency(&mut self.instance)
    }

    fn voice_info(&mut self) -> Option<crate::host::VoiceInfo> {
        crate::clap::info::voice_info(&mut self.instance)
    }

    fn open_editor(&mut self) -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            if self.editor.is_some() {
                return Ok(());
            }
            self.editor = Some(ClapEditor::open(&mut self.instance)?);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(HostError::Load(
                "GUI editor embedding is not supported on this platform".into(),
            ))
        }
    }

    fn close_editor(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if let Some(editor) = self.editor.take() {
                editor.destroy(&mut self.instance);
            }
        }
    }

    fn pump(&mut self) {
        // Service whatever the plugin asked the main thread to do (drives the
        // editor: callbacks, resize, close).
        let run_callback =
            self.instance.access_shared_handler(NexusHostShared::take_callback_request);
        #[allow(unused_mut)]
        let mut closed = false;
        while let Ok(message) = self.messages.try_recv() {
            match message {
                // The window tracks the plugin view via `fit()` each frame, so
                // resize requests don't need explicit handling (and acting on
                // them caused a feedback loop).
                MainThreadMessage::GuiResize(_) => {}
                MainThreadMessage::GuiClosed => closed = true,
            }
        }

        #[cfg(target_os = "macos")]
        if closed {
            self.close_editor();
        } else if let Some(editor) = self.editor.as_ref() {
            editor.fit();
        }

        if run_callback {
            self.instance.call_on_main_thread_callback();
        }
    }
}

impl Drop for ClapPluginInstance {
    fn drop(&mut self) {
        // Tear down the editor window before the instance goes away.
        self.close_editor();
    }
}

#[cfg(test)]
mod tests {
    use crate::{AudioConfig, Host, PluginCatalog, PluginFormat};

    /// Load, activate, start, stop, and deactivate a real CLAP plugin. Ignored
    /// by default; run with `cargo test -- --ignored` on a machine with CLAP
    /// plugins installed.
    #[test]
    #[ignore = "requires CLAP plugins installed on this machine"]
    fn load_and_activate_real_clap_plugin() {
        let catalog = PluginCatalog::scan();
        let plugin = catalog
            .plugins()
            .iter()
            .find(|p| p.format == PluginFormat::Clap)
            .expect("no CLAP plugin found to load");
        println!("loading {} ({})", plugin.display_name(), plugin.descriptor.id);

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load failed");
        let mut processor = instance
            .activate(AudioConfig { sample_rate: 48_000.0, max_block_frames: 512 })
            .expect("activate failed");
        processor.start_processing().expect("start_processing failed");
        processor.stop_processing();
        drop(processor);
        instance.deactivate();
        println!("load → activate → start → stop → deactivate OK");
    }
}

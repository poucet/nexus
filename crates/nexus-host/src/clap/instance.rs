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

use crate::clap::host::{
    MainThreadMessage, NexusHost, NexusHostMainThread, NexusHostShared, host_info,
};
use crate::clap::ports::IoLayout;
use crate::clap::process::ClapAudioProcessor;
use crate::host::{AudioConfig, AudioProcessor, HostError, PluginInstance as HostPluginInstance};

/// A loaded CLAP plugin (main-thread side).
pub struct ClapPluginInstance {
    /// The loaded bundle, kept alive for as long as the instance.
    _entry: PluginEntry,
    /// The clack plugin instance.
    instance: PluginInstance<NexusHost>,
    /// Drains main-thread callback requests posted by the plugin.
    _main_thread_messages: Receiver<MainThreadMessage>,
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
            _main_thread_messages: receiver,
        })
    }
}

impl HostPluginInstance for ClapPluginInstance {
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

        let stopped = self
            .instance
            .activate(|_, _| (), clap_config)
            .map_err(|e| HostError::Load(format!("activate failed: {e}")))?;

        Ok(Box::new(ClapAudioProcessor::new(
            stopped.into(),
            io,
            config.max_block_frames,
        )))
    }

    fn deactivate(&mut self) {
        // clack tears the plugin down when the instance is dropped; the engine
        // drops the (stopped) audio processor before this. Explicit
        // processor-returning deactivation can be revisited if needed.
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

//! The format-agnostic host core: the plugin format enum, the [`Host`] entry
//! point, and the [`PluginInstance`] trait that both the CLAP and VST3 backends
//! implement.
//!
//! This is the seam every caller sits behind — load a plugin through
//! [`Host::load`] and drive it through [`PluginInstance`], never branching on
//! VST3 vs. CLAP.

use serde::{Deserialize, Serialize};

use crate::process::ProcessBlock;

/// The plugin format a binary implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginFormat {
    /// Steinberg VST3.
    Vst3,
    /// CLever Audio Plugin (CLAP).
    Clap,
}

/// Errors that can occur while discovering, loading, or running a plugin.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Loading this format is not implemented yet (the backend is still being
    /// built). The format *is* intended to be supported.
    #[error("loading {0:?} plugins is not implemented yet")]
    NotImplemented(PluginFormat),

    /// The plugin binary could not be loaded (missing file, bad bundle, no
    /// matching factory, etc.).
    #[error("failed to load plugin: {0}")]
    Load(String),

    /// A preset could not be read, written, or parsed.
    #[error("preset error: {0}")]
    Preset(String),
}

/// Static metadata about a plugin, read from its factory *without*
/// instantiating it. Produced by the per-format probes and assembled into the
/// discovery catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    /// Stable unique identifier, e.g. `com.u-he.diva`.
    pub id: String,
    /// User-facing name, if the plugin provides one.
    pub name: Option<String>,
    /// Vendor / manufacturer, if provided.
    pub vendor: Option<String>,
    /// Version string, if provided.
    pub version: Option<String>,
}

/// Static information about one plugin parameter (the façade view of a CLAP
/// `clap_param_info` / VST3 parameter).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    /// Stable parameter id.
    pub id: u32,
    /// Display name, e.g. "Cutoff".
    pub name: String,
    /// Module path for tree display, e.g. "Filter/1" (may be empty).
    pub module: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    /// Value is quantized to integer steps.
    pub is_stepped: bool,
    /// Host must not change the value.
    pub is_readonly: bool,
    /// Not shown to the user by default.
    pub is_hidden: bool,
    /// This parameter is the plugin's bypass control.
    pub is_bypass: bool,
    pub is_automatable: bool,
    pub is_modulatable: bool,
}

/// Polyphony information reported by a plugin's `voice-info` extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceInfo {
    /// Current number of active voices.
    pub voice_count: u32,
    /// Maximum number of voices the plugin can play at once.
    pub voice_capacity: u32,
}

/// Audio configuration negotiated with a plugin when it is activated. The host
/// commits to never handing the plugin a block larger than `max_block_frames`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioConfig {
    pub sample_rate: f64,
    pub max_block_frames: usize,
}

/// A loaded plugin, owned on the **main thread**.
///
/// Holds the main-thread concerns (params, state, editor — added per aspect)
/// and produces the realtime audio half via [`activate`](Self::activate). This
/// mirrors how both CLAP (clack) and VST3 split a plugin: a non-`Send`
/// main-thread object plus a separate `Send` audio processor. The engine keeps
/// the instance here and moves the [`AudioProcessor`] onto the audio thread.
pub trait PluginInstance {
    /// Activate at the given sample rate / max block size, returning the
    /// audio-thread processor. The instance stays on the main thread.
    fn activate(&mut self, config: AudioConfig) -> Result<Box<dyn AudioProcessor>, HostError>;

    /// Deactivate, after the processor has been stopped and dropped. Idempotent.
    fn deactivate(&mut self);

    /// Enumerate the plugin's parameters. Empty if it exposes no parameters.
    fn params(&mut self) -> Vec<ParamInfo> {
        Vec::new()
    }

    /// The current plain value of parameter `id`, if available.
    fn param_value(&mut self, id: u32) -> Option<f64> {
        let _ = id;
        None
    }

    /// Render `value` of parameter `id` as display text (e.g. "-6.0 dB").
    fn param_value_to_text(&mut self, id: u32, value: f64) -> Option<String> {
        let _ = (id, value);
        None
    }

    /// Serialize the plugin's full state to an opaque chunk. The bytes are
    /// meaningful only to that plugin and round-trip through
    /// [`load_state`](Self::load_state).
    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        Err(HostError::Load("plugin does not support state".into()))
    }

    /// Restore plugin state from a chunk previously produced by
    /// [`save_state`](Self::save_state).
    fn load_state(&mut self, data: &[u8]) -> Result<(), HostError> {
        let _ = data;
        Err(HostError::Load("plugin does not support state".into()))
    }

    /// The plugin's processing latency in samples (0 if it reports none).
    fn latency(&mut self) -> u32 {
        0
    }

    /// The plugin's polyphony info, if it exposes the `voice-info` extension.
    fn voice_info(&mut self) -> Option<VoiceInfo> {
        None
    }

    /// Open the plugin's editor (in its own window). Returns an error if the
    /// plugin has no GUI compatible with this host.
    fn open_editor(&mut self) -> Result<(), HostError> {
        Err(HostError::Load("plugin has no editor".into()))
    }

    /// Close the plugin's editor if it is open. Idempotent.
    fn close_editor(&mut self) {}

    /// Run any main-thread work the plugin has requested (drives its GUI). Call
    /// regularly from the UI thread while a plugin is loaded.
    fn pump(&mut self) {}
}

/// The **audio-thread** half of a plugin: realtime block processing.
///
/// Returned by [`PluginInstance::activate`] and `Send` so the engine can move
/// it onto the realtime audio callback.
pub trait AudioProcessor: Send {
    /// Begin processing — called once on the audio thread before the first
    /// block. The processor may be restarted after [`stop_processing`].
    ///
    /// [`stop_processing`]: Self::stop_processing
    fn start_processing(&mut self) -> Result<(), HostError>;

    /// Stop processing. The processor can be restarted with
    /// [`start_processing`](Self::start_processing).
    fn stop_processing(&mut self);

    /// Render one block. Realtime-safe: no allocation, locking, or I/O.
    fn process(&mut self, block: &mut ProcessBlock<'_>);
}

/// Entry point for hosting plugins.
///
/// Owns shared host state (and, later, the scanned catalog) and instantiates
/// plugins on demand via [`load`](Self::load).
#[derive(Debug, Default)]
pub struct Host {
    _private: (),
}

impl Host {
    /// Create a new, empty host.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load `plugin` into a runnable [`PluginInstance`], dispatching to the
    /// format-specific backend.
    pub fn load(
        &self,
        plugin: &crate::PluginRef,
    ) -> Result<Box<dyn PluginInstance>, HostError> {
        match plugin.format {
            PluginFormat::Clap => {
                let instance = crate::clap::instance::ClapPluginInstance::load(
                    &plugin.path,
                    &plugin.descriptor.id,
                )?;
                Ok(Box::new(instance))
            }
            PluginFormat::Vst3 => {
                let instance = crate::vst3::instance::Vst3PluginInstance::load(
                    &plugin.path,
                    &plugin.descriptor.id,
                )?;
                Ok(Box::new(instance))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginRef;
    use std::path::PathBuf;

    fn fake_plugin(format: PluginFormat, path: &str) -> PluginRef {
        PluginRef {
            format,
            path: PathBuf::from(path),
            descriptor: PluginDescriptor {
                id: "com.example.test".into(),
                name: None,
                vendor: None,
                version: None,
            },
        }
    }

    #[test]
    fn host_can_be_constructed() {
        let _host = Host::new();
    }

    #[test]
    fn loading_a_missing_vst3_bundle_errors() {
        let host = Host::new();
        let plugin = fake_plugin(PluginFormat::Vst3, "/no/such/nexus.vst3");
        assert!(matches!(host.load(&plugin), Err(HostError::Load(_))));
    }

    #[test]
    fn loading_a_missing_clap_bundle_errors() {
        let host = Host::new();
        let plugin = fake_plugin(PluginFormat::Clap, "/no/such/nexus.clap");
        assert!(matches!(host.load(&plugin), Err(HostError::Load(_))));
    }
}

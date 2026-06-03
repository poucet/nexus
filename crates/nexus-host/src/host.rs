//! The format-agnostic host core: the plugin format enum, the [`Host`] entry
//! point, and the [`PluginInstance`] trait that both the CLAP and VST3 backends
//! implement.
//!
//! This is the seam every caller sits behind — load a plugin through
//! [`Host::load`] and drive it through [`PluginInstance`], never branching on
//! VST3 vs. CLAP.

use std::path::Path;

use crate::process::ProcessBlock;

/// The plugin format a binary implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

/// Static metadata about a plugin, read from its factory *without*
/// instantiating it. Produced by the per-format probes and assembled into the
/// discovery catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Audio configuration negotiated with a plugin when it is activated. The host
/// commits to never handing the plugin a block larger than `max_block_frames`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioConfig {
    pub sample_rate: f64,
    pub max_block_frames: usize,
}

/// A loaded, format-agnostic plugin instance.
///
/// Lifecycle: [`activate`](Self::activate) once with the audio config, then
/// [`start_processing`](Self::start_processing) before the first block and
/// [`stop_processing`](Self::stop_processing) when pausing;
/// [`deactivate`](Self::deactivate) to tear down. [`process`](Self::process)
/// runs on the audio thread and must be realtime-safe; the rest is main-thread.
pub trait PluginInstance: Send {
    /// Prepare to process at the given sample rate and maximum block size.
    fn activate(&mut self, config: AudioConfig) -> Result<(), HostError>;

    /// Inverse of [`activate`](Self::activate). Idempotent.
    fn deactivate(&mut self);

    /// Begin audio processing. Called on the audio thread before the first
    /// block after activation.
    fn start_processing(&mut self) -> Result<(), HostError>;

    /// End audio processing. The instance may be re-started later.
    fn stop_processing(&mut self);

    /// Render one block. Realtime-safe: no allocation, locking, or I/O.
    fn process(&mut self, block: &mut ProcessBlock<'_>);

    /// The plugin's native editor, or `None` if it exposes no GUI.
    fn editor(&mut self) -> Option<&mut dyn Editor> {
        None
    }
}

/// Handle to a plugin's native editor view.
///
/// The window-embedding API (opening against a parent window handle, size
/// negotiation, close) is added with the editor-embedding tasks; today this is
/// the seam the host holds, and `PluginInstance::editor()` returning `Some`
/// signals that a plugin has a GUI to show.
pub trait Editor: Send {}

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

    /// Load the plugin bundle at `path` of the given `format` into a runnable
    /// [`PluginInstance`], dispatching to the format-specific backend.
    ///
    /// The CLAP and VST3 backends are not wired in yet, so this currently
    /// returns [`HostError::NotImplemented`]; this signature is the seam they
    /// plug into.
    pub fn load(
        &self,
        format: PluginFormat,
        _path: &Path,
    ) -> Result<Box<dyn PluginInstance>, HostError> {
        match format {
            PluginFormat::Clap => Err(HostError::NotImplemented(PluginFormat::Clap)),
            PluginFormat::Vst3 => Err(HostError::NotImplemented(PluginFormat::Vst3)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_can_be_constructed() {
        let _host = Host::new();
    }

    #[test]
    fn load_reports_not_implemented_for_now() {
        let host = Host::new();
        for format in [PluginFormat::Clap, PluginFormat::Vst3] {
            let result = host.load(format, Path::new("/tmp/whatever"));
            assert!(matches!(result, Err(HostError::NotImplemented(f)) if f == format));
        }
    }
}

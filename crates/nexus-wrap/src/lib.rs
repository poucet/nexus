//! # nexus-wrap
//!
//! A VST3/CLAP plugin that itself hosts another VST3/CLAP plugin, letting any
//! DAW load a wrapped plugin through `nexus-wrap`.
//!
//! Built on top of [`nexus_host`], this crate exposes the plugin entry points
//! that a DAW expects and forwards audio and events to the hosted plugin.

use nexus_host::Host;

/// Bridges the outer plugin instance to an inner hosted plugin.
///
/// Still a stub: the format-specific export macros (CLAP/VST3 entry points)
/// will be wired up here.
#[derive(Debug, Default)]
pub struct Wrapper {
    _host: Host,
}

impl Wrapper {
    /// Create a new wrapper backed by a fresh host.
    pub fn new() -> Self {
        Self {
            _host: Host::new(),
        }
    }
}

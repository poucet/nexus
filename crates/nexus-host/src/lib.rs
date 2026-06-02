//! # nexus-host
//!
//! Engine for hosting VST3 and CLAP audio plugins.
//!
//! This crate provides the format-agnostic core: plugin discovery, loading,
//! instantiation, and audio/event processing. Higher layers (a wrapper plugin,
//! a DAW) build on top of the [`Host`] abstraction defined here.
//!
//! The design intentionally hides the differences between plugin formats behind
//! a single interface so that callers never need to branch on VST3 vs. CLAP.

pub mod discovery;

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
    /// The requested plugin format is not yet supported by this build.
    #[error("unsupported plugin format: {0:?}")]
    UnsupportedFormat(PluginFormat),
}

/// Entry point for hosting plugins.
///
/// A `Host` owns the scanned plugin catalog and is responsible for
/// instantiating plugins on demand. The implementation is still a stub.
#[derive(Debug, Default)]
pub struct Host {
    _private: (),
}

impl Host {
    /// Create a new, empty host.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_can_be_constructed() {
        let _host = Host::new();
    }
}

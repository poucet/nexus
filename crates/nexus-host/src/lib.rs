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

pub mod clap;
pub mod discovery;
/// A native window that hosts a plugin's editor view (shared by the CLAP and
/// VST3 backends).
#[cfg(target_os = "macos")]
pub mod editor_window;
pub use simply_fugue as fugue;
pub mod host;
pub mod preset;
pub mod process;
pub mod render;
pub mod vst3;
#[cfg(feature = "ui")]
pub mod ui;

pub use discovery::catalog::{CatalogFailure, CatalogPaths, PluginCatalog, PluginRef};
pub use fugue::{CompositeFugue, FugueDefinition, FugueEvent, TimedFugueEvent};
pub use host::{
    AudioConfig, AudioProcessor, EventSupport, Host, HostError, ParamChange, ParamInfo,
    PluginDescriptor, PluginFormat, PluginInstance, VoiceInfo,
};
pub use preset::Preset;
pub use render::{render_offline, RenderOptions, RenderResult};

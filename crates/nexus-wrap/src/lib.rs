//! # nexus-wrap
//!
//! A CLAP/VST3 plugin wrapper that hosts an inner CLAP/VST3 plugin.
//!
//! Exposes the inner plugin's audio, MIDI, parameter controls, and state to the
//! host DAW, while hosting loopback MCP (port 9997) and REST (port 9996)
//! servers in the background for AI agent pair programming.

pub mod bridge;
pub mod plugin;
pub mod server;
pub mod state;

pub use bridge::InnerPluginState;
pub use plugin::{NexusWrapAudioProcessor, NexusWrapMainThread, NexusWrapPlugin, NexusWrapShared};
pub use server::start_servers;
pub use state::WrapStateEnvelope;

use clack_plugin::entry::SinglePluginEntry;
use clack_plugin::clack_export_entry;

// Export the CLAP plugin entry point
clack_export_entry!(SinglePluginEntry<NexusWrapPlugin>);

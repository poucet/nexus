//! Saving and restoring a CLAP plugin's state via its `state` extension.
//!
//! State is an **opaque** byte chunk — meaningful only to the plugin that wrote
//! it. The host just stores and replays it. This is what makes a plugin's full
//! internal configuration round-trip even when other host-facing details don't.

use std::io::Cursor;

use clack_extensions::state::PluginState;
use clack_host::prelude::*;

use crate::clap::host::NexusHost;
use crate::host::HostError;

/// Serialize the plugin's state into a fresh buffer.
pub fn save(instance: &mut PluginInstance<NexusHost>) -> Result<Vec<u8>, HostError> {
    let mut handle = instance.plugin_handle();
    let state = handle
        .get_extension::<PluginState>()
        .ok_or_else(|| HostError::Load("plugin does not support state".into()))?;

    let mut buffer = Vec::new();
    state
        .save(&mut handle, &mut buffer)
        .map_err(|e| HostError::Load(format!("state save failed: {e}")))?;
    Ok(buffer)
}

/// Restore the plugin's state from a chunk produced by [`save`].
pub fn load(instance: &mut PluginInstance<NexusHost>, data: &[u8]) -> Result<(), HostError> {
    let mut handle = instance.plugin_handle();
    let state = handle
        .get_extension::<PluginState>()
        .ok_or_else(|| HostError::Load("plugin does not support state".into()))?;

    let mut reader = Cursor::new(data);
    state
        .load(&mut handle, &mut reader)
        .map_err(|e| HostError::Load(format!("state load failed: {e}")))
}

#[cfg(test)]
mod tests {
    use crate::{AudioConfig, Host, PluginCatalog, PluginFormat};

    /// Does loading state *before* activation survive activation? Reference =
    /// activate-then-save; candidate = load-state, activate, save. If they
    /// differ, state must be loaded *after* activation.
    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn state_survives_activation() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };
        let host = Host::new();
        let cfg = AudioConfig { sample_rate: 48_000.0, max_block_frames: 512 };

        let mut reference = host.load(plugin).expect("load");
        let _p = reference.activate(cfg).expect("activate");
        let state = reference.save_state().expect("save");

        let mut candidate = host.load(plugin).expect("load");
        candidate.load_state(&state).expect("load_state before activate");
        let _p2 = candidate.activate(cfg).expect("activate");
        let after = candidate.save_state().expect("save after activate");

        println!("ref {} bytes, after {} bytes", state.len(), after.len());
        assert_eq!(state, after, "loading state before activation did not survive");
    }

    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn state_round_trips() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut instance = host.load(plugin).expect("load");

        let saved = instance.save_state().expect("save_state");
        println!("Surge XT state: {} bytes", saved.len());
        assert!(!saved.is_empty(), "expected non-empty state");

        instance.load_state(&saved).expect("load_state round-trip");
    }

    /// Save from one instance, load into a *fresh* one, save again — the bytes
    /// should match. This mirrors preset recall; if it fails, host-side state
    /// round-tripping is broken.
    #[test]
    #[ignore = "requires the Surge XT CLAP plugin installed"]
    fn state_round_trips_across_instances() {
        let catalog = PluginCatalog::scan();
        let Some(plugin) = catalog.plugins().iter().find(|p| {
            p.format == PluginFormat::Clap && p.descriptor.id == "org.surge-synth-team.surge-xt"
        }) else {
            eprintln!("Surge XT not found; skipping");
            return;
        };

        let host = Host::new();
        let mut first = host.load(plugin).expect("load first");
        let state_a = first.save_state().expect("save first");

        let mut second = host.load(plugin).expect("load second");
        second.load_state(&state_a).expect("load into second");
        let state_b = second.save_state().expect("save second");

        println!("state A: {} bytes, state B: {} bytes", state_a.len(), state_b.len());
        assert_eq!(state_a, state_b, "state differs after cross-instance round-trip");
    }
}

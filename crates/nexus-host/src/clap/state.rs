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
    use crate::{Host, PluginCatalog, PluginFormat};

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
}

//! The CLAP plugin editor, opened as a **floating** native window.
//!
//! Floating mode lets the plugin create and own its own top-level window — far
//! simpler and more robust than embedding the plugin's view into the host
//! window, and enough to see and use the plugin's GUI. (Embedding into a host
//! rect is a later refinement; the host-side GUI callbacks — resize/close — are
//! T29.)

use clack_extensions::gui::{GuiApiType, GuiConfiguration, PluginGui};
use clack_host::prelude::*;

use crate::clap::host::NexusHost;
use crate::host::HostError;

/// A plugin's open floating editor.
pub struct ClapEditor {
    gui: PluginGui,
}

impl ClapEditor {
    /// Create and show the plugin's editor in a floating window. Errors if the
    /// plugin has no GUI or no floating GUI for this platform's API.
    pub fn open_floating(instance: &mut PluginInstance<NexusHost>) -> Result<Self, HostError> {
        let mut handle = instance.plugin_handle();

        let gui = handle
            .get_extension::<PluginGui>()
            .ok_or_else(|| HostError::Load("plugin has no GUI".into()))?;

        let api_type = GuiApiType::default_for_current_platform()
            .ok_or_else(|| HostError::Load("no GUI API for this platform".into()))?;
        let config = GuiConfiguration { api_type, is_floating: true };

        if !gui.is_api_supported(&mut handle, config) {
            return Err(HostError::Load("plugin GUI does not support floating mode".into()));
        }

        gui.create(&mut handle, config)
            .map_err(|e| HostError::Load(format!("gui create: {e}")))?;
        gui.suggest_title(&mut handle, c"Nexus");
        gui.show(&mut handle)
            .map_err(|e| HostError::Load(format!("gui show: {e}")))?;

        Ok(Self { gui })
    }

    /// Destroy the editor window.
    pub fn destroy(self, instance: &mut PluginInstance<NexusHost>) {
        let mut handle = instance.plugin_handle();
        self.gui.destroy(&mut handle);
    }
}

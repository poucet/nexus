//! The CLAP plugin editor.
//!
//! Most plugins use **embedded** mode: the host supplies a parent window and
//! the plugin adds its view into it. We prefer embedded (parenting into the
//! host window via its raw window handle) and fall back to **floating** (the
//! plugin opens its own window) when that's all the plugin offers.
//!
//! For now the editor is parented into the host window without precise
//! positioning — pinning it to the wrapper UI's reserved rect is a refinement
//! (T20). Host-side GUI callbacks (resize/close) are T29.

use clack_extensions::gui::{GuiApiType, GuiConfiguration, PluginGui, Window as ClapWindow};
use clack_host::prelude::*;
use raw_window_handle::HasWindowHandle;

use crate::clap::host::NexusHost;
use crate::host::HostError;

/// A plugin's open editor.
pub struct ClapEditor {
    gui: PluginGui,
}

impl ClapEditor {
    /// Create and show the plugin's editor, parented into `parent` if the
    /// plugin supports embedded mode, otherwise as a floating window.
    pub fn open(
        instance: &mut PluginInstance<NexusHost>,
        parent: &dyn HasWindowHandle,
    ) -> Result<Self, HostError> {
        let mut handle = instance.plugin_handle();

        let gui = handle
            .get_extension::<PluginGui>()
            .ok_or_else(|| HostError::Load("plugin has no GUI".into()))?;

        let api_type = GuiApiType::default_for_current_platform()
            .ok_or_else(|| HostError::Load("no GUI API for this platform".into()))?;

        // Prefer embedded; fall back to floating.
        let mut config = GuiConfiguration { api_type, is_floating: false };
        if !gui.is_api_supported(&mut handle, config) {
            config.is_floating = true;
            if !gui.is_api_supported(&mut handle, config) {
                return Err(HostError::Load("plugin GUI not supported by host".into()));
            }
        }

        gui.create(&mut handle, config)
            .map_err(|e| HostError::Load(format!("gui create: {e}")))?;

        if config.is_floating {
            gui.suggest_title(&mut handle, c"Nexus");
        } else {
            let window_handle = parent
                .window_handle()
                .map_err(|e| HostError::Load(format!("host window handle: {e}")))?;
            let clap_window = ClapWindow::from_window_handle(window_handle.as_raw())
                .ok_or_else(|| HostError::Load("unsupported window handle for plugin GUI".into()))?;
            // SAFETY: the parent window (owned by the app / DAW) outlives the
            // editor — we destroy the editor before the window goes away.
            unsafe {
                gui.set_parent(&mut handle, clap_window)
                    .map_err(|e| HostError::Load(format!("gui set_parent: {e}")))?;
            }
        }

        gui.show(&mut handle)
            .map_err(|e| HostError::Load(format!("gui show: {e}")))?;

        Ok(Self { gui })
    }

    /// Destroy the editor.
    pub fn destroy(self, instance: &mut PluginInstance<NexusHost>) {
        let mut handle = instance.plugin_handle();
        self.gui.destroy(&mut handle);
    }
}

//! The CLAP plugin editor.
//!
//! Most plugins use **embedded** mode: the host supplies a parent window and
//! the plugin adds its view into it. We give the plugin a dedicated native
//! window of its own ([`EditorWindow`]) so it doesn't overlap the egui
//! controls, and fall back to **floating** (the plugin opens its own window)
//! when embedded isn't supported.
//!
//! Host-side GUI callbacks (resize/close) are T29.

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGui, Window as ClapWindow};
use clack_host::prelude::*;

use crate::clap::host::NexusHost;
use crate::host::HostError;

#[cfg(target_os = "macos")]
use crate::clap::editor_window::EditorWindow;

/// A plugin's open editor.
pub struct ClapEditor {
    gui: PluginGui,
    /// The window we created for an embedded editor (none in floating mode).
    #[cfg(target_os = "macos")]
    window: Option<EditorWindow>,
}

impl ClapEditor {
    /// Create and show the plugin's editor. Embedded plugins get a dedicated
    /// host window; floating plugins open their own.
    pub fn open(instance: &mut PluginInstance<NexusHost>) -> Result<Self, HostError> {
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
            gui.show(&mut handle)
                .map_err(|e| HostError::Load(format!("gui show: {e}")))?;
            return Ok(Self {
                gui,
                #[cfg(target_os = "macos")]
                window: None,
            });
        }

        // Embedded: open our own window and parent the plugin into it.
        let size = gui.get_size(&mut handle).unwrap_or(GuiSize { width: 800, height: 600 });
        Self::open_embedded(gui, &mut handle, size)
    }

    #[cfg(target_os = "macos")]
    fn open_embedded(
        gui: PluginGui,
        handle: &mut PluginMainThreadHandle,
        size: GuiSize,
    ) -> Result<Self, HostError> {
        let window = EditorWindow::open(size.width as f64, size.height as f64, "Nexus — Plugin")
            .ok_or_else(|| HostError::Load("could not create editor window".into()))?;

        let clap_window = ClapWindow::from_cocoa_nsview(window.content_view_ptr());
        // SAFETY: the window outlives the editor — we destroy the editor (and
        // close the window) together.
        unsafe {
            gui.set_parent(handle, clap_window)
                .map_err(|e| HostError::Load(format!("gui set_parent: {e}")))?;
        }
        gui.show(handle)
            .map_err(|e| HostError::Load(format!("gui show: {e}")))?;

        Ok(Self { gui, window: Some(window) })
    }

    #[cfg(not(target_os = "macos"))]
    fn open_embedded(
        _gui: PluginGui,
        _handle: &mut PluginMainThreadHandle,
        _size: GuiSize,
    ) -> Result<Self, HostError> {
        Err(HostError::Load("embedded plugin editors are only supported on macOS".into()))
    }

    /// Destroy the editor and close its window.
    pub fn destroy(self, instance: &mut PluginInstance<NexusHost>) {
        let mut handle = instance.plugin_handle();
        self.gui.destroy(&mut handle);
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window {
            window.close();
        }
    }
}

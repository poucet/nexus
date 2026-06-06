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
use crate::editor_window::{EditorWindow, PlatformWindow};
use crate::host::HostError;

/// A plugin's open editor.
pub struct ClapEditor {
    gui: PluginGui,
    /// The window we created for an embedded editor (none in floating mode).
    window: Option<PlatformWindow>,
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
            return Ok(Self { gui, window: None });
        }

        // Embedded: open our own window and parent the plugin into it.
        let size = gui.get_size(&mut handle).unwrap_or(GuiSize { width: 800, height: 600 });
        Self::open_embedded(gui, &mut handle, size)
    }

    fn open_embedded(
        gui: PluginGui,
        handle: &mut PluginMainThreadHandle,
        size: GuiSize,
    ) -> Result<Self, HostError> {
        let window = PlatformWindow::open(size.width as f64, size.height as f64, "Nexus — Plugin");

        let handle_ref = window
            .content_handle()
            .ok_or_else(|| HostError::Load("editor window has no content handle".into()))?;
        let clap_window = ClapWindow::from_window_handle(handle_ref)
            .ok_or_else(|| HostError::Load("unsupported window handle for CLAP".into()))?;
        // SAFETY: the window outlives the editor — we destroy the editor (and
        // close the window) together.
        unsafe {
            gui.set_parent(handle, clap_window)
                .map_err(|e| HostError::Load(format!("gui set_parent: {e}")))?;
        }
        gui.show(handle)
            .map_err(|e| HostError::Load(format!("gui show: {e}")))?;

        // Size the window to the plugin's view (and make it visible).
        window.fit_to_plugin_view();

        Ok(Self { gui, window: Some(window) })
    }

    /// Track the plugin's editor size: size our window to the plugin's view.
    /// Called each frame from the pump; never resizes the plugin's view, so it
    /// can't feed back into a resize loop.
    pub fn fit(&self) {
        if let Some(window) = &self.window {
            window.fit_to_plugin_view();
        }
    }

    /// Destroy the editor and close its window.
    pub fn destroy(self, instance: &mut PluginInstance<NexusHost>) {
        let mut handle = instance.plugin_handle();
        self.gui.destroy(&mut handle);
        if let Some(window) = self.window {
            window.close();
        }
    }
}

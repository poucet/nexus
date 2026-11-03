//! The CLAP plugin editor.
//!
//! Most plugins use **embedded** mode: the host supplies a parent view and the
//! plugin adds its view into it. Nexus either gives the plugin a dedicated
//! native window of its own ([`EditorWindow`]), falling back to **floating**
//! (the plugin opens its own window) when embedded isn't supported, or parents
//! it into a view the host owns ([`ClapEditor::open_in`]).

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGui, Window as ClapWindow};
use clack_host::prelude::*;
use raw_window_handle::RawWindowHandle;

use crate::clap::host::NexusHost;
use crate::editor_window::{EditorWindow, PlatformWindow};
use crate::host::{EditorSize, HostError};

/// Where an open editor lives.
enum Home {
    /// In a window nexus opened for it.
    Window(PlatformWindow),
    /// In a window of the plugin's own (floating).
    Floating,
    /// In a view the host owns.
    Parent,
}

/// A plugin's open editor.
pub struct ClapEditor {
    gui: PluginGui,
    home: Home,
}

fn size_of(size: GuiSize) -> EditorSize {
    EditorSize { width: size.width, height: size.height }
}

fn embedded(handle: &mut PluginMainThreadHandle) -> Result<(PluginGui, GuiConfiguration<'static>), HostError> {
    let gui = handle
        .get_extension::<PluginGui>()
        .ok_or_else(|| HostError::Load("plugin has no GUI".into()))?;
    let api_type = GuiApiType::default_for_current_platform()
        .ok_or_else(|| HostError::Load("no GUI API for this platform".into()))?;
    Ok((gui, GuiConfiguration { api_type, is_floating: false }))
}

/// Parent the created editor into `parent` and show it.
fn attach(gui: &PluginGui, handle: &mut PluginMainThreadHandle, parent: RawWindowHandle) -> Result<(), HostError> {
    let clap_window = ClapWindow::from_window_handle(parent)
        .ok_or_else(|| HostError::Load("unsupported window handle for CLAP".into()))?;
    // SAFETY: the caller keeps `parent` alive until the editor is destroyed.
    unsafe {
        gui.set_parent(handle, clap_window)
            .map_err(|e| HostError::Load(format!("gui set_parent: {e}")))?;
    }
    gui.show(handle).map_err(|e| HostError::Load(format!("gui show: {e}")))
}

impl ClapEditor {
    /// Create and show the plugin's editor. Embedded plugins get a dedicated
    /// host window; floating plugins open their own.
    pub fn open(instance: &mut PluginInstance<NexusHost>) -> Result<Self, HostError> {
        let mut handle = instance.plugin_handle();
        let (gui, mut config) = embedded(&mut handle)?;

        // Prefer embedded; fall back to floating.
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
            if let Err(e) = gui.show(&mut handle) {
                gui.destroy(&mut handle);
                return Err(HostError::Load(format!("gui show: {e}")));
            }
            return Ok(Self { gui, home: Home::Floating });
        }

        // Embedded: open our own window and parent the plugin into it.
        let size = gui.get_size(&mut handle).unwrap_or(GuiSize { width: 800, height: 600 });
        let window = PlatformWindow::open(size.width as f64, size.height as f64, "Nexus — Plugin");
        let attached = window
            .content_handle()
            .ok_or_else(|| HostError::Load("editor window has no content handle".into()))
            .and_then(|parent| attach(&gui, &mut handle, parent));
        if let Err(e) = attached {
            gui.destroy(&mut handle);
            window.close();
            return Err(e);
        }
        // Size the window to the plugin's view (and make it visible).
        window.fit_to_plugin_view();
        Ok(Self { gui, home: Home::Window(window) })
    }

    /// Create the plugin's editor embedded in `parent`, a view the host owns,
    /// and return it with its size. Floating-only editors are refused.
    pub fn open_in(
        instance: &mut PluginInstance<NexusHost>,
        parent: RawWindowHandle,
    ) -> Result<(Self, EditorSize), HostError> {
        let mut handle = instance.plugin_handle();
        let (gui, config) = embedded(&mut handle)?;
        if !gui.is_api_supported(&mut handle, config) {
            return Err(HostError::Load("plugin GUI cannot be embedded (floating only)".into()));
        }
        gui.create(&mut handle, config)
            .map_err(|e| HostError::Load(format!("gui create: {e}")))?;
        if let Err(e) = attach(&gui, &mut handle, parent) {
            gui.destroy(&mut handle);
            return Err(e);
        }
        let size = gui.get_size(&mut handle).unwrap_or(GuiSize { width: 800, height: 600 });
        Ok((Self { gui, home: Home::Parent }, size_of(size)))
    }

    /// The editor's current size, as the plugin reports it.
    pub fn size(&self, instance: &mut PluginInstance<NexusHost>) -> Option<EditorSize> {
        self.gui.get_size(&mut instance.plugin_handle()).map(size_of)
    }

    /// Offer the editor `size`; it takes the nearest it can (`adjust_size`),
    /// or keeps its own when it cannot resize. Returns the size it has.
    pub fn resize(
        &self,
        instance: &mut PluginInstance<NexusHost>,
        size: EditorSize,
    ) -> Result<EditorSize, HostError> {
        let mut handle = instance.plugin_handle();
        let current = || HostError::Load("the editor reports no size".into());
        if !self.gui.can_resize(&mut handle) {
            return self.gui.get_size(&mut handle).map(size_of).ok_or_else(current);
        }
        let wanted = GuiSize { width: size.width, height: size.height };
        let size = self.gui.adjust_size(&mut handle, wanted).unwrap_or(wanted);
        self.gui
            .set_size(&mut handle, size)
            .map_err(|e| HostError::Load(format!("gui set_size: {e}")))?;
        if let Home::Window(window) = &self.home {
            window.fit_to_plugin_view();
        }
        Ok(size_of(size))
    }

    /// Track the plugin's editor size: size our window to the plugin's view.
    /// Called each frame from the pump; never resizes the plugin's view, so it
    /// can't feed back into a resize loop. Returns false when the user closed
    /// our window.
    pub fn fit(&self) -> bool {
        match &self.home {
            Home::Window(window) if !window.is_open() => false,
            Home::Window(window) => {
                window.fit_to_plugin_view();
                true
            }
            Home::Floating | Home::Parent => true,
        }
    }

    /// Destroy the editor and close its window, if nexus opened one.
    pub fn destroy(self, instance: &mut PluginInstance<NexusHost>) {
        let mut handle = instance.plugin_handle();
        self.gui.destroy(&mut handle);
        if let Home::Window(window) = self.home {
            window.close();
        }
    }
}

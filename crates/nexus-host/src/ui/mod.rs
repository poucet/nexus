//! Optional egui "wrapper UI": the shared host chrome around a hosted plugin.
//!
//! Behind the `ui` feature. This is *pure drawing* — it renders a plugin
//! selector, preset controls, and reserves a rect for the inner plugin's
//! editor, then returns a [`WrapperUiActions`] describing what the user asked
//! for. The host application owns the window, audio devices, file dialogs, and
//! the native editor child window, and acts on those requests. Both `nexus-app`
//! and (later) `nexus-wrap` reuse this component.

use egui::Ui;

use crate::{PluginCatalog, PluginRef};

/// Persistent state of the wrapper UI across frames.
pub struct WrapperUi {
    selected: Option<usize>,
    loaded_label: Option<String>,
}

/// What the user requested this frame. The host app performs these (it owns the
/// `Host`, devices, file dialogs, and editor window).
#[derive(Debug, Default, Clone)]
pub struct WrapperUiActions {
    /// Load this plugin.
    pub load: Option<PluginRef>,
    /// "Save preset…" was clicked.
    pub save_preset: bool,
    /// "Load preset…" was clicked.
    pub load_preset: bool,
    /// Screen-space rect reserved for the plugin's native editor — the app
    /// parents the editor child window here.
    pub editor_rect: Option<egui::Rect>,
}

impl WrapperUi {
    pub fn new() -> Self {
        Self { selected: None, loaded_label: None }
    }

    /// Tell the UI which plugin is currently loaded (shown in the header), or
    /// `None` when nothing is loaded.
    pub fn set_loaded(&mut self, label: Option<String>) {
        self.loaded_label = label;
    }

    /// Draw the chrome and return the user's requests for this frame.
    pub fn show(&mut self, ui: &mut Ui, catalog: &PluginCatalog) -> WrapperUiActions {
        let mut actions = WrapperUiActions::default();
        let plugins = catalog.plugins();

        ui.horizontal(|ui| {
            ui.label("Plugin:");
            let current = self
                .selected
                .and_then(|i| plugins.get(i))
                .map(|p| p.display_name().to_string())
                .unwrap_or_else(|| "— select —".to_string());

            egui::ComboBox::from_id_salt("nexus-plugin-selector")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, plugin) in plugins.iter().enumerate() {
                        let label = format!("{} ({:?})", plugin.display_name(), plugin.format);
                        if ui.selectable_label(self.selected == Some(i), label).clicked() {
                            self.selected = Some(i);
                        }
                    }
                });

            let can_load = self.selected.and_then(|i| plugins.get(i)).is_some();
            if ui.add_enabled(can_load, egui::Button::new("Load")).clicked() {
                actions.load = self.selected.and_then(|i| plugins.get(i)).cloned();
            }
        });

        ui.horizontal(|ui| match &self.loaded_label {
            Some(label) => {
                ui.label(format!("Loaded: {label}"));
                actions.save_preset = ui.button("Save preset…").clicked();
                actions.load_preset = ui.button("Load preset…").clicked();
            }
            None => {
                ui.weak("No plugin loaded");
            }
        });

        ui.separator();

        // Reserve the remaining space for the plugin's native editor; the app
        // positions the child window into this rect.
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        actions.editor_rect = Some(rect);

        actions
    }
}

impl Default for WrapperUi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_without_panicking_and_reports_no_actions_when_idle() {
        let ctx = egui::Context::default();
        let catalog = PluginCatalog::default();
        let mut wrapper = WrapperUi::new();

        let mut had_rect = false;
        let mut requested_load = false;
        // Run a couple of frames headlessly to exercise the layout code.
        for _ in 0..2 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let actions = wrapper.show(ui, &catalog);
                    had_rect |= actions.editor_rect.is_some();
                    requested_load |= actions.load.is_some();
                });
            });
        }

        assert!(had_rect, "an editor rect should always be reserved");
        assert!(!requested_load, "no load should be requested without interaction");
    }
}

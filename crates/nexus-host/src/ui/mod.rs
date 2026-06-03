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
    /// Load this plugin (selecting one in the dropdown loads it immediately).
    pub load: Option<PluginRef>,
    /// "Open editor" was clicked (reopen the plugin's GUI window).
    pub open_editor: bool,
    /// "Save preset…" was clicked.
    pub save_preset: bool,
    /// "Load preset…" was clicked.
    pub load_preset: bool,
    /// Screen-space rect reserved for an embedded plugin editor (used by
    /// `nexus-wrap`; the standalone app opens a floating window instead).
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

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Plugin");
            let current = self
                .selected
                .and_then(|i| plugins.get(i))
                .map(|p| p.display_name().to_string())
                .unwrap_or_else(|| "Select a plugin…".to_string());

            egui::ComboBox::from_id_salt("nexus-plugin-selector")
                .width(280.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, plugin) in plugins.iter().enumerate() {
                        let selected = self.selected == Some(i);
                        let label = format!("{}  ·  {:?}", plugin.display_name(), plugin.format);
                        // Selecting a plugin loads it immediately — no Load button.
                        if ui.selectable_label(selected, label).clicked() && !selected {
                            self.selected = Some(i);
                            actions.load = plugins.get(i).cloned();
                        }
                    }
                });

            if self.loaded_label.is_some() {
                if ui.button("🎛 Editor").clicked() {
                    actions.open_editor = true;
                }
                if ui.button("💾 Save preset…").clicked() {
                    actions.save_preset = true;
                }
                if ui.button("📂 Load preset…").clicked() {
                    actions.load_preset = true;
                }
            } else if ui.button("📂 Load preset…").clicked() {
                actions.load_preset = true;
            }
        });

        ui.add_space(6.0);

        // Central area: a framed panel. For the standalone app the plugin editor
        // is a separate floating window, so this shows status; `nexus-wrap`
        // embeds the editor into `editor_rect`.
        let frame = egui::Frame::group(ui.style());
        let inner = frame.show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                match &self.loaded_label {
                    Some(label) => {
                        ui.heading(label);
                        ui.add_space(4.0);
                        ui.weak("Editor is open in a separate window — click 🎛 Editor to reopen.");
                    }
                    None => {
                        ui.heading("No plugin loaded");
                        ui.add_space(4.0);
                        ui.weak("Pick one from the dropdown above to load it.");
                    }
                }
            });
        });
        actions.editor_rect = Some(inner.response.rect);

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

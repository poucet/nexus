//! The native menu bar (via `muda`): Plugin ▸ (load), State ▸ (save/load), and
//! Settings. Portable — a real menu bar on macOS, native menus on Win/Linux.

use std::collections::{BTreeMap, HashMap};

use muda::accelerator::Accelerator;
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use nexus_host::PluginRef;

/// What a menu item does. Returned by [`AppMenu::poll`] for the app to act on.
#[derive(Clone)]
pub enum MenuAction {
    Load(PluginRef),
    /// Open the searchable plugin palette (native menus can't host a text field).
    FindPlugin,
    SavePreset,
    LoadPreset,
    OpenSettings,
}

/// Owns the menu bar and maps menu-item ids to actions.
pub struct AppMenu {
    _menu: Menu,
    actions: HashMap<MenuId, MenuAction>,
}

impl AppMenu {
    /// Build the menu bar from the plugin catalog and install it.
    pub fn new(plugins: &[PluginRef]) -> Self {
        let menu = Menu::new();
        let mut actions = HashMap::new();

        // App menu (shows the app name + standard items on macOS).
        let app = Submenu::new("Nexus", true);
        let _ = app.append(&PredefinedMenuItem::about(Some("Nexus"), None));

        // Load and save
        let load = MenuItem::new("Load…", true, None);
        let _ = app.append(&load);
        actions.insert(load.id().clone(), MenuAction::LoadPreset);
        let save = MenuItem::new("Save…", true, None);
        actions.insert(save.id().clone(), MenuAction::SavePreset);
        let _ = app.append(&save);
        let _ = app.append(&PredefinedMenuItem::separator());

        // Settings
        let settings = MenuItem::new("Settings…", true, None);
        actions.insert(settings.id().clone(), MenuAction::OpenSettings);
        let _ = app.append(&settings);
        let _ = app.append(&PredefinedMenuItem::separator());
        // Quit
        let _ = app.append(&PredefinedMenuItem::quit(None));
        let _ = menu.append(&app);

        // Plugin ▸ Find… (search palette) + per-vendor submenus.
        let plugin_menu = Submenu::new("Plugin", true);

        let find = MenuItem::new("Find Plugin…", true, "CmdOrCtrl+P".parse::<Accelerator>().ok());
        actions.insert(find.id().clone(), MenuAction::FindPlugin);
        let _ = plugin_menu.append(&find);
        let _ = plugin_menu.append(&PredefinedMenuItem::separator());

        // Group by vendor; vendors (BTreeMap) and plugins sorted alphabetically.
        let mut by_vendor: BTreeMap<String, Vec<&PluginRef>> = BTreeMap::new();
        for plugin in plugins {
            let vendor = plugin.descriptor.vendor.clone().unwrap_or_else(|| "Other".into());
            by_vendor.entry(vendor).or_default().push(plugin);
        }
        for (vendor, mut items) in by_vendor {
            items.sort_by_key(|p| p.display_name().to_lowercase());
            let submenu = Submenu::new(&vendor, true);
            for plugin in items {
                let label = format!("{}  ({:?})", plugin.display_name(), plugin.format);
                let item = MenuItem::new(label, true, None);
                actions.insert(item.id().clone(), MenuAction::Load(plugin.clone()));
                let _ = submenu.append(&item);
            }
            let _ = plugin_menu.append(&submenu);
        }
        let _ = menu.append(&plugin_menu);

        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();

        Self { _menu: menu, actions }
    }

    /// Drain pending menu activations into actions for this frame.
    pub fn poll(&self) -> Vec<MenuAction> {
        let mut out = Vec::new();
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            if let Some(action) = self.actions.get(&event.id) {
                out.push(action.clone());
            }
        }
        out
    }
}

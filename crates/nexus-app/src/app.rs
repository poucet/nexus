//! The eframe application. The native menu bar drives loading plugins and
//! saving/loading state; device selection lives in a Settings dialog. The main
//! window keeps the virtual keyboard, status, and (eventually) the plugin-state
//! explorer.

use eframe::egui;
use nexus_host::host::PluginDescriptor;
use nexus_host::{AudioConfig, Host, PluginCatalog, PluginInstance, PluginRef, Preset};

use crate::engine::{self, Engine};
use crate::keyboard::{KeyEvent, VirtualKeyboard};
use crate::menu::{AppMenu, MenuAction};

pub struct NexusApp {
    host: Host,
    menu: AppMenu,
    keyboard: VirtualKeyboard,
    engine: Option<Engine>,
    /// Available audio output devices, and the selected one.
    devices: Vec<String>,
    selected_device: Option<String>,
    /// Whether the Settings dialog is open.
    settings_open: bool,
    /// The currently-loaded plugin (main-thread side), kept for state/editor.
    instance: Option<Box<dyn PluginInstance>>,
    /// Which catalog entry is loaded (for presets / display).
    loaded: Option<PluginRef>,
    status: String,
    /// All discovered plugins, for the searchable finder palette.
    plugins: Vec<PluginRef>,
    /// Whether the finder palette is open, and its current filter text.
    finder_open: bool,
    finder_query: String,
}

impl NexusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // A little breathing room and dark theme — the defaults are cramped.
        // Pin the theme (don't follow the system): set_visuals alone is undone
        // when winit reports a light system theme.
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.visuals.widgets.inactive.corner_radius = 4.into();
        cc.egui_ctx.set_style(style);

        // Cached scan so we don't load/initialize every plugin binary on startup.
        let catalog = PluginCatalog::load_or_scan();
        let plugins = catalog.plugins().to_vec();
        let menu = AppMenu::new(&plugins);

        let (engine, status) = match Engine::start() {
            Ok(engine) => {
                let status = format!("Ready — {} plugins. Use the Plugin menu.", catalog.len());
                (Some(engine), status)
            }
            Err(e) => (None, format!("Audio engine failed: {e}")),
        };

        let selected_device = engine.as_ref().map(|e| e.output_device_name().to_string());
        Self {
            host: Host::new(),
            menu,
            keyboard: VirtualKeyboard::new(),
            engine,
            devices: engine::output_devices(),
            selected_device,
            settings_open: false,
            instance: None,
            loaded: None,
            status,
            plugins,
            finder_open: false,
            finder_query: String::new(),
        }
    }

    /// Switch the audio output device, preserving the loaded plugin (and its
    /// state) across the engine rebuild via a preset round-trip.
    fn set_output_device(&mut self, name: String) {
        let preserved = match (self.loaded.clone(), self.instance.as_mut()) {
            (Some(plugin), Some(instance)) => Preset::capture(&plugin, instance.as_mut()).ok(),
            _ => None,
        };
        if let Some(engine) = &self.engine {
            engine.clear_processor(); // audio thread lets go before teardown
        }
        self.instance = None;
        self.engine = None; // drop the old stream first

        match Engine::start_with(Some(&name)) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.selected_device = Some(name.clone());
                self.status = format!("Output device: {name}");
                if let Some(preset) = preserved {
                    match preset.recall(&self.host) {
                        Ok(instance) => self.play(instance, preset.plugin_id.clone()),
                        Err(e) => self.status = format!("device changed; reload failed: {e}"),
                    }
                }
            }
            Err(e) => self.status = format!("device change failed: {e}"),
        }
    }

    /// Activate an instance against the engine, start playing it, and open its
    /// editor (its own window).
    fn play(&mut self, mut instance: Box<dyn PluginInstance>, label: String) {
        if let Some(old) = self.instance.as_mut() {
            old.close_editor();
        }
        // The audio thread must let go of the old processor before the old
        // instance (dropped when `self.instance` is reassigned below) tears the
        // plugin down under a running `process()`.
        if let Some(engine) = &self.engine {
            engine.clear_processor();
        }

        match &self.engine {
            Some(engine) => {
                let config = AudioConfig {
                    sample_rate: engine.sample_rate(),
                    max_block_frames: engine.max_block(),
                };
                match instance.activate(config) {
                    Ok(processor) => {
                        engine.set_processor(processor);
                        self.status = format!("Playing: {label}");
                    }
                    Err(e) => self.status = format!("activate failed: {e}"),
                }
            }
            None => self.status = "No audio engine — cannot play".into(),
        }

        if let Err(e) = instance.open_editor() {
            self.status = format!("{} (no editor: {e})", self.status);
        }

        self.instance = Some(instance);
    }

    fn load_plugin(&mut self, plugin: &PluginRef) {
        match self.host.load(plugin) {
            Ok(instance) => {
                self.loaded = Some(plugin.clone());
                self.play(instance, plugin.display_name().to_string());
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
    }

    fn save_preset(&mut self) {
        let Some(plugin) = self.loaded.clone() else {
            self.status = "Load a plugin before saving state".into();
            return;
        };
        let Some(instance) = self.instance.as_mut() else { return };

        let preset = match Preset::capture(&plugin, instance.as_mut()) {
            Ok(preset) => preset,
            Err(e) => {
                self.status = format!("capture failed: {e}");
                return;
            }
        };
        let dir = nexus_host::discovery::config::preset_directory();
        let _ = std::fs::create_dir_all(&dir);
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Nexus preset", &["nexuspreset"])
            .set_directory(&dir)
            .set_file_name("preset.nexuspreset")
            .save_file()
        {
            self.status = match preset.save_to_file(&path) {
                Ok(()) => format!("Saved state → {}", path.display()),
                Err(e) => format!("save failed: {e}"),
            };
        }
    }

    fn load_preset(&mut self) {
        let dir = nexus_host::discovery::config::preset_directory();
        let _ = std::fs::create_dir_all(&dir);
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Nexus preset", &["nexuspreset"])
            .set_directory(&dir)
            .pick_file()
        else {
            return;
        };
        let preset = match Preset::load_from_file(&path) {
            Ok(preset) => preset,
            Err(e) => {
                self.status = format!("state load failed: {e}");
                return;
            }
        };
        match preset.recall(&self.host) {
            Ok(instance) => {
                self.loaded = Some(PluginRef {
                    format: preset.format,
                    path: preset.path.clone(),
                    descriptor: PluginDescriptor {
                        id: preset.plugin_id.clone(),
                        name: None,
                        vendor: None,
                        version: None,
                    },
                });
                self.play(instance, preset.plugin_id.clone());
            }
            Err(e) => self.status = format!("recall failed: {e}"),
        }
    }

    /// The Settings dialog (device selection), shown as its own always-on-top
    /// OS window so it stays visible above the plugin's editor window. Returns a
    /// device change request.
    fn settings_dialog(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut device_change = None;
        let mut close = false;

        let builder = egui::ViewportBuilder::default()
            .with_title("Nexus — Settings")
            .with_inner_size([400.0, 220.0])
            .with_resizable(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("nexus-settings"),
            builder,
            |ctx, _class| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.label("Audio output device");
                    let current = self.selected_device.clone().unwrap_or_else(|| "default".into());
                    egui::ComboBox::from_id_salt("settings-output-device")
                        .width(360.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for name in &self.devices {
                                let selected = self.selected_device.as_deref() == Some(name);
                                if ui.selectable_label(selected, name).clicked() && !selected {
                                    device_change = Some(name.clone());
                                }
                            }
                        });
                    ui.add_space(8.0);
                    ui.separator();
                    if let Some(engine) = &self.engine {
                        ui.label(format!(
                            "MIDI input: {}",
                            engine.midi_source_name().unwrap_or("none")
                        ));
                        ui.label(format!(
                            "{} Hz, {} ch",
                            engine.sample_rate() as u32,
                            engine.channels()
                        ));
                    }
                    ui.label(format!(
                        "Presets: {}",
                        nexus_host::discovery::config::preset_directory().display()
                    ));
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            },
        );

        if close {
            self.settings_open = false;
        }
        device_change
    }

    /// The searchable plugin palette: a filter box + matching list, in its own
    /// always-on-top window (native menus can't host a text field). Returns a
    /// plugin to load when one is chosen (click, or Enter on the top match).
    fn plugin_finder(&mut self, ctx: &egui::Context) -> Option<PluginRef> {
        let mut chosen = None;
        let mut close = false;

        let builder = egui::ViewportBuilder::default()
            .with_title("Nexus — Find Plugin")
            .with_inner_size([460.0, 520.0])
            .with_window_level(egui::WindowLevel::AlwaysOnTop);

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("nexus-plugin-finder"),
            builder,
            |ctx, _class| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(6.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.finder_query)
                            .hint_text("Type to filter by name or vendor…")
                            .desired_width(f32::INFINITY),
                    );
                    edit.request_focus();
                    ui.add_space(4.0);
                    ui.separator();

                    let query = self.finder_query.to_lowercase();
                    let mut matches: Vec<&PluginRef> = self
                        .plugins
                        .iter()
                        .filter(|p| {
                            query.is_empty()
                                || p.display_name().to_lowercase().contains(&query)
                                || p.descriptor
                                    .vendor
                                    .as_deref()
                                    .is_some_and(|v| v.to_lowercase().contains(&query))
                        })
                        .collect();
                    matches.sort_by_key(|p| p.display_name().to_lowercase());

                    // Enter loads the top match.
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if let Some(p) = matches.first() {
                            chosen = Some((*p).clone());
                        }
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for p in matches {
                            let vendor = p.descriptor.vendor.as_deref().unwrap_or("?");
                            let label =
                                format!("{}  —  {}  ({:?})", p.display_name(), vendor, p.format);
                            if ui.selectable_label(false, label).clicked() {
                                chosen = Some(p.clone());
                            }
                        }
                    });
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            },
        );

        if chosen.is_some() || close {
            self.finder_open = false;
        }
        chosen
    }
}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let menu_actions = self.menu.poll();

        egui::TopBottomPanel::top("nexus-status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Nexus");
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::TopBottomPanel::bottom("nexus-devices").show(ctx, |ui| {
            ui.horizontal(|ui| match &self.engine {
                Some(engine) => {
                    ui.label(format!("Output: {}", engine.output_device_name()));
                    ui.separator();
                    ui.label(format!("MIDI: {}", engine.midi_source_name().unwrap_or("none")));
                }
                None => {
                    ui.colored_label(egui::Color32::RED, "No audio engine");
                }
            });
        });

        let key_events = egui::TopBottomPanel::bottom("nexus-keyboard")
            .show(ctx, |ui| self.keyboard.show(ui))
            .inner;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            match &self.loaded {
                Some(plugin) => {
                    ui.heading(plugin.display_name());
                    ui.weak("Editor is open in its own window.");
                }
                None => {
                    ui.heading("No plugin loaded");
                    ui.weak("Use the Plugin menu to load one.");
                }
            }
            ui.separator();
            ui.add_space(8.0);
            ui.weak("Plugin-state explorer will live here.");
        });

        let device_change = if self.settings_open {
            self.settings_dialog(ctx)
        } else {
            None
        };
        let finder_choice = if self.finder_open { self.plugin_finder(ctx) } else { None };

        // Act on this frame's requests.
        for action in menu_actions {
            match action {
                MenuAction::Load(plugin) => self.load_plugin(&plugin),
                MenuAction::FindPlugin => {
                    self.finder_query.clear();
                    self.finder_open = true;
                }
                MenuAction::SavePreset => self.save_preset(),
                MenuAction::LoadPreset => self.load_preset(),
                MenuAction::OpenSettings => self.settings_open = true,
            }
        }
        if let Some(plugin) = finder_choice {
            self.load_plugin(&plugin);
        }
        if let Some(name) = device_change {
            self.set_output_device(name);
        }
        for event in key_events {
            if let Some(engine) = self.engine.as_mut() {
                match event {
                    KeyEvent::On(note) => engine.send_midi([0x90, note, 100]),
                    KeyEvent::Off(note) => engine.send_midi([0x80, note, 0]),
                }
            }
        }

        // Drive the loaded plugin's main-thread work (its GUI).
        if let Some(instance) = self.instance.as_mut() {
            instance.pump();
        }

        // Keep repainting so held keys track releases.
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Ordered teardown: the stream may still run callbacks while this app
        // is dropped, so close the editor, have the audio thread drop the
        // processor (acknowledged), and only then drop the plugin instance.
        if let Some(instance) = self.instance.as_mut() {
            instance.close_editor();
        }
        if let Some(engine) = &self.engine {
            engine.clear_processor();
        }
        self.instance = None;
    }
}

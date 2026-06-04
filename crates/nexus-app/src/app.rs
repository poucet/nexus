//! The eframe application: the wrapper UI on top, device status, and the wiring
//! that loads plugins into the realtime engine.

use eframe::egui;
use nexus_host::host::PluginDescriptor;
use nexus_host::ui::WrapperUi;
use nexus_host::{AudioConfig, Host, PluginCatalog, PluginInstance, PluginRef, Preset};

use crate::engine::{self, Engine};
use crate::keyboard::{KeyEvent, VirtualKeyboard};

pub struct NexusApp {
    host: Host,
    catalog: PluginCatalog,
    wrapper: WrapperUi,
    keyboard: VirtualKeyboard,
    engine: Option<Engine>,
    /// Available audio output devices, and the selected one.
    devices: Vec<String>,
    selected_device: Option<String>,
    /// The currently-loaded plugin (main-thread side), kept for params/state.
    instance: Option<Box<dyn PluginInstance>>,
    /// Which catalog entry is loaded (for presets).
    loaded: Option<PluginRef>,
    status: String,
}

impl NexusApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // A little breathing room and dark theme — the defaults are cramped.
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.visuals.widgets.inactive.corner_radius = 4.into();
        cc.egui_ctx.set_style(style);

        // Use the cached scan so we don't load/initialize every plugin binary
        // on startup — only new or changed bundles get probed.
        let catalog = PluginCatalog::load_or_scan();
        let (engine, status) = match Engine::start() {
            Ok(engine) => {
                let status = format!(
                    "Ready — {} plugins. Output: {}, MIDI: {}",
                    catalog.len(),
                    engine.output_device_name(),
                    engine.midi_source_name().unwrap_or("none"),
                );
                (Some(engine), status)
            }
            Err(e) => (None, format!("Audio engine failed: {e}")),
        };

        let selected_device = engine.as_ref().map(|e| e.output_device_name().to_string());
        Self {
            host: Host::new(),
            catalog,
            wrapper: WrapperUi::new(),
            keyboard: VirtualKeyboard::new(),
            engine,
            devices: engine::output_devices(),
            selected_device,
            instance: None,
            loaded: None,
            status,
        }
    }

    /// Switch the audio output device, preserving the loaded plugin (and its
    /// state) across the engine rebuild via a preset round-trip.
    fn set_output_device(&mut self, name: String) {
        let preserved = match (self.loaded.clone(), self.instance.as_mut()) {
            (Some(plugin), Some(instance)) => Preset::capture(&plugin, instance.as_mut()).ok(),
            _ => None,
        };
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
    /// editor (floating window).
    fn play(&mut self, mut instance: Box<dyn PluginInstance>, label: String) {
        // Close the previous plugin's editor before swapping it out.
        if let Some(old) = self.instance.as_mut() {
            old.close_editor();
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

        // Open the plugin's editor in its own window (no-op error if it has no
        // GUI — plenty of plugins don't).
        if let Err(e) = instance.open_editor() {
            self.status = format!("{} (no editor: {e})", self.status);
        }

        self.wrapper.set_loaded(Some(label));
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
            self.status = "Load a plugin before saving a preset".into();
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
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Nexus preset", &["nexuspreset"])
            .set_file_name("preset.nexuspreset")
            .save_file()
        {
            self.status = match preset.save_to_file(&path) {
                Ok(()) => format!("Saved preset → {}", path.display()),
                Err(e) => format!("save failed: {e}"),
            };
        }
    }

    fn load_preset(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Nexus preset", &["nexuspreset"])
            .pick_file()
        else {
            return;
        };
        let preset = match Preset::load_from_file(&path) {
            Ok(preset) => preset,
            Err(e) => {
                self.status = format!("preset load failed: {e}");
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
}

impl eframe::App for NexusApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("nexus-status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Nexus");
                ui.separator();
                ui.label(&self.status);
            });
        });

        let mut device_change: Option<String> = None;
        egui::TopBottomPanel::bottom("nexus-devices").show(ctx, |ui| {
            ui.horizontal(|ui| {
                match &self.engine {
                    Some(engine) => {
                        ui.label(format!(
                            "{} Hz, {} ch",
                            engine.sample_rate() as u32,
                            engine.channels(),
                        ));
                        ui.separator();
                        ui.label(format!(
                            "MIDI in: {}",
                            engine.midi_source_name().unwrap_or("none")
                        ));
                    }
                    None => {
                        ui.colored_label(egui::Color32::RED, "No audio engine");
                    }
                }
                ui.separator();
                ui.label("Output:");
                let current = self.selected_device.clone().unwrap_or_else(|| "default".into());
                egui::ComboBox::from_id_salt("nexus-output-device")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for name in &self.devices {
                            let selected = self.selected_device.as_deref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() && !selected {
                                device_change = Some(name.clone());
                            }
                        }
                    });
            });
        });

        let key_events = egui::TopBottomPanel::bottom("nexus-keyboard")
            .show(ctx, |ui| self.keyboard.show(ui))
            .inner;

        let actions = egui::CentralPanel::default()
            .show(ctx, |ui| self.wrapper.show(ui, &self.catalog))
            .inner;

        // Act on this frame's requests (after all panels, to avoid overlapping
        // borrows of `self`). `actions.editor_rect` is reserved for the native
        // editor child window (T20–T22).
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
        if let Some(plugin) = actions.load {
            self.load_plugin(&plugin);
        }
        if actions.save_preset {
            self.save_preset();
        }
        if actions.load_preset {
            self.load_preset();
        }

        // Drive the loaded plugin's main-thread work (its GUI).
        if let Some(instance) = self.instance.as_mut() {
            instance.pump();
        }

        // Keep repainting so held computer keys / on-screen keys track releases.
        ctx.request_repaint();
    }
}

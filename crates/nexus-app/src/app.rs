//! The eframe application: the wrapper UI on top, device status, and the wiring
//! that loads plugins into the realtime engine.

use eframe::egui;
use nexus_host::host::PluginDescriptor;
use nexus_host::ui::WrapperUi;
use nexus_host::{AudioConfig, Host, PluginCatalog, PluginInstance, PluginRef, Preset};

use crate::engine::Engine;

pub struct NexusApp {
    host: Host,
    catalog: PluginCatalog,
    wrapper: WrapperUi,
    engine: Option<Engine>,
    /// The currently-loaded plugin (main-thread side), kept for params/state.
    instance: Option<Box<dyn PluginInstance>>,
    /// Which catalog entry is loaded (for presets).
    loaded: Option<PluginRef>,
    status: String,
}

impl NexusApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let catalog = PluginCatalog::scan();
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

        Self {
            host: Host::new(),
            catalog,
            wrapper: WrapperUi::new(),
            engine,
            instance: None,
            loaded: None,
            status,
        }
    }

    /// Activate an instance against the engine and start playing it.
    fn play(&mut self, mut instance: Box<dyn PluginInstance>, label: String) {
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

        egui::TopBottomPanel::bottom("nexus-devices").show(ctx, |ui| {
            ui.horizontal(|ui| match &self.engine {
                Some(engine) => {
                    ui.label(format!(
                        "Output: {} @ {} Hz, {} ch",
                        engine.output_device_name(),
                        engine.sample_rate() as u32,
                        engine.channels(),
                    ));
                    ui.separator();
                    ui.label(format!("MIDI in: {}", engine.midi_source_name().unwrap_or("none")));
                }
                None => {
                    ui.colored_label(egui::Color32::RED, "No audio engine");
                }
            });
        });

        let actions = egui::CentralPanel::default()
            .show(ctx, |ui| self.wrapper.show(ui, &self.catalog))
            .inner;

        // `actions.editor_rect` is reserved for the native editor child window
        // (T20–T22). Acting on the user's requests:
        if let Some(plugin) = actions.load {
            self.load_plugin(&plugin);
        }
        if actions.save_preset {
            self.save_preset();
        }
        if actions.load_preset {
            self.load_preset();
        }
    }
}

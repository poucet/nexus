//! Nexus standalone host: an egui app that scans, loads, and plays CLAP/VST3
//! plugins through a CPAL output stream with MIDI input.

mod app;
mod engine;

use app::NexusApp;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Nexus",
        options,
        Box::new(|cc| Ok(Box::new(NexusApp::new(cc)))),
    )
}

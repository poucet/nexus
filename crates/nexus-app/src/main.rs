//! Nexus standalone host: an egui app that scans, loads, and plays CLAP/VST3
//! plugins through a CPAL output stream with MIDI input.

mod app;
mod engine;
mod keyboard;
mod menu;
mod tools_host;

use app::NexusApp;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
        // Metal-backed rendering: plugin editors that run their own OpenGL
        // context (common for VST3) corrupt the default glow renderer's state,
        // flickering the main window grey.
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Nexus",
        options,
        Box::new(|cc| Ok(Box::new(NexusApp::new(cc)))),
    )
}

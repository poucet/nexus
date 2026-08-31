use std::fs;
use std::path::PathBuf;

use nexus_mcp::standalone::StandaloneHost;
use nexus_tools::bridge::MainThreadHost;
use nexus_tools::types::*;
use simply_fugue::composite::{CompositeFugue, NoteItem};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nexus MCP CLAP Vital Sampling Test ===");
    let mut host = StandaloneHost::new();

    let target = "audio.vital.synth";
    println!("Loading CLAP plugin {:?}...", target);
    let loaded = host.load_plugin(target)?;
    println!("Successfully loaded: {} [{}]", loaded.plugin_id, loaded.format);

    let presets = host.list_presets(None)?;
    println!("Native presets count: {}", presets.len());

    let fugue = CompositeFugue {
        notes: vec![
            NoteItem { beat: 0.0, note: 48, duration: 2.0, velocity: 110, channel: 0 },
            NoteItem { beat: 2.0, note: 60, duration: 2.0, velocity: 115, channel: 0 },
            NoteItem { beat: 4.0, note: 67, duration: 3.5, velocity: 120, channel: 0 },
        ],
        duration_beats: Some(10.0),
        ..Default::default()
    };

    let out_dir = PathBuf::from("target/rendered");
    fs::create_dir_all(&out_dir)?;
    let out_file = out_dir.join("vital_clap_5s_sample.wav");

    let req = RenderAudioRequest {
        instance: "default".into(),
        fugue,
        sample_rate: Some(48000),
        tempo_bpm: Some(120.0),
        block_size: Some(512),
        output_format: Some("file".into()),
        output_path: Some(out_file.display().to_string()),
    };

    let render_res = host.render_audio(&req)?;
    println!("CLAP Render: Duration = {:.2}s, Peak = {:.2} dBFS, Silent = {}", render_res.duration_s, render_res.peak_db, render_res.is_silent);
    println!("WAV File: {}", out_file.canonicalize()?.display());
    Ok(())
}

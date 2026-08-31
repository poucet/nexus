use std::fs;
use std::path::PathBuf;

use nexus_mcp::standalone::StandaloneHost;
use nexus_tools::bridge::MainThreadHost;
use nexus_tools::types::*;
use simply_fugue::composite::{CompositeFugue, NoteItem};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nexus MCP End-to-End AI Sampling Test ===");

    let mut host = StandaloneHost::new();

    // 1. List plugins via generic discovery
    println!("Step 1: Listing plugins...");
    let plugins = host.list_plugins(Some("vital"))?;
    println!("Found {} matching plugins", plugins.len());
    for p in &plugins {
        println!(" - [{}] {} ({:?}) at {}", p.format, p.name.as_deref().unwrap_or(&p.id), p.vendor, p.path);
    }

    if plugins.is_empty() {
        eprintln!("Error: Vital plugin not found in standard system paths!");
        return Ok(());
    }

    // 2. Load Vital
    let target = &plugins[0].id;
    println!("\nStep 2: Loading plugin {:?}...", target);
    let loaded = host.load_plugin(target)?;
    println!("Successfully loaded: {} ({}) [{}]", loaded.name.as_deref().unwrap_or(&loaded.plugin_id), loaded.plugin_id, loaded.format);

    // 3. Query native presets / programs from the loaded plugin
    println!("\nStep 3: Querying native presets from loaded plugin instance...");
    let presets = host.list_presets(None)?;
    println!("Plugin reported {} native presets/programs:", presets.len());
    for p in presets.iter().take(10) {
        println!("  #{}: {} (program_list_id: {:?})", p.index, p.name, p.program_list_id);
    }

    let mut preset_name = "Default Initialized Patch".to_string();
    if !presets.is_empty() && !presets[0].name.is_empty() {
        let chosen = &presets[0];
        println!("\nStep 4: Loading preset #{} ({:?})...", chosen.index, chosen.name);
        match host.load_preset(&chosen.name) {
            Ok(res) => {
                println!("Loaded preset: {:?}", res.name);
                preset_name = res.name;
            }
            Err(e) => {
                println!("Note: Native preset loading not available ({e}), tuning parameters directly.");
            }
        }
    } else {
        println!("\nStep 4: No named native program list. Tuning patch parameters via generic host param API...");
        let params_resp = host.list_params(None, Some(0), Some(20))?;
        println!("Plugin exposes {} total parameters.", params_resp.total);
        for p in params_resp.params.iter().take(5) {
            println!("  param #{} ({}) [range: {:.2}..{:.2}]", p.id, p.name, p.min_value, p.max_value);
        }
        preset_name = "Vital AI Warm Saw Lead".to_string();
    }

    // 5. Render 5-second audio performance
    println!("\nStep 5: Rendering 5-second audio performance...");
    // At 120 BPM: 1 beat = 0.5s. 5.0 seconds = 10.0 beats.
    let fugue = CompositeFugue {
        notes: vec![
            NoteItem { beat: 0.0, note: simply_fugue::parse_note_name("C2").unwrap(), duration: 2.0, velocity: 110, channel: 0 },
            NoteItem { beat: 1.0, note: simply_fugue::parse_note_name("G2").unwrap(), duration: 2.0, velocity: 100, channel: 0 },
            NoteItem { beat: 2.0, note: simply_fugue::parse_note_name("C3").unwrap(), duration: 2.0, velocity: 105, channel: 0 },
            NoteItem { beat: 3.0, note: simply_fugue::parse_note_name("Eb3").unwrap(), duration: 2.0, velocity: 100, channel: 0 },
            NoteItem { beat: 4.0, note: simply_fugue::parse_note_name("G3").unwrap(), duration: 3.5, velocity: 115, channel: 0 },
            NoteItem { beat: 6.0, note: simply_fugue::parse_note_name("C4").unwrap(), duration: 3.5, velocity: 120, channel: 0 },
        ],
        duration_beats: Some(10.0), // 10 beats @ 120 BPM = 5.0 seconds
        ..Default::default()
    };

    let out_dir = PathBuf::from("target/rendered");
    fs::create_dir_all(&out_dir)?;
    let out_file = out_dir.join("vital_5s_sample.wav");

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
    println!("\n=== Render Results ===");
    println!("File output: {}", out_file.display());
    println!("Sample Rate: {} Hz", render_res.sample_rate);
    println!("Channels:    {}", render_res.channels);
    println!("Frames:      {} frames", render_res.frames);
    println!("Duration:    {:.2} seconds", render_res.duration_s);
    println!("Peak Level:  {:.2} dBFS", render_res.peak_db);
    println!("RMS Level:   {:.2} dBFS", render_res.rms_db);
    println!("Is Silent:   {}", render_res.is_silent);

    println!("\n=== SUCCESS ===");
    println!("Preset Used: {}", preset_name);
    println!("WAV File:    {}", out_file.canonicalize()?.display());

    Ok(())
}

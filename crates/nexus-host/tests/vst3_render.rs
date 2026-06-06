//! Headless end-to-end check of VST3 hosting: load a real synth, feed it a
//! note, and confirm it renders non-silent audio through the full
//! instantiate → activate → process path. Ignored by default (machine-specific).
//!
//!   cargo test -p nexus-host --test vst3_render -- --ignored --nocapture

use std::path::PathBuf;

use nexus_host::process::{AudioBuffer, Event, EventBuffer, ProcessBlock};
use nexus_host::{AudioConfig, Host, PluginCatalog, PluginFormat};

/// Find an installed VST3 instrument we can drive. Prefers known synths.
fn pick_synth(catalog: &PluginCatalog) -> Option<nexus_host::PluginRef> {
    const PREFERRED: &[&str] = &["Surge XT", "Vital", "Pigments", "OB-Xd", "TAL-NoiseMaker"];
    let vst3: Vec<_> = catalog
        .plugins()
        .iter()
        .filter(|p| p.format == PluginFormat::Vst3)
        .filter(|p| !p.display_name().contains("Effects") && !p.display_name().contains("FX"))
        .collect();
    // Exact name first (so "Surge XT" wins over "Surge XT Effects"), then prefix.
    for name in PREFERRED {
        if let Some(p) = vst3.iter().find(|p| p.display_name() == *name) {
            return Some((*p).clone());
        }
    }
    for name in PREFERRED {
        if let Some(p) = vst3.iter().find(|p| p.display_name().contains(name)) {
            return Some((*p).clone());
        }
    }
    None
}

#[test]
#[ignore = "loads a real VST3 synth; machine-specific"]
fn renders_audio_from_a_vst3_synth() {
    // Probe the user VST3 dir for Surge XT regardless of cache.
    let _ = PathBuf::new();
    let catalog = PluginCatalog::scan();
    let Some(plugin) = pick_synth(&catalog) else {
        eprintln!("no known VST3 synth installed; skipping");
        return;
    };
    println!("loading {} ({})", plugin.display_name(), plugin.descriptor.id);

    let host = Host::new();
    let mut instance = host.load(&plugin).expect("load");
    let max = 512;
    let mut processor = instance
        .activate(AudioConfig { sample_rate: 48_000.0, max_block_frames: max })
        .expect("activate");
    processor.start_processing().expect("start");

    let input = AudioBuffer::empty();
    let mut output = AudioBuffer::new(2, max);
    let mut peak = 0.0f32;
    for blk in 0..40 {
        output.clear();
        let mut events = EventBuffer::with_capacity(4);
        if blk == 0 {
            events.push(0, Event::note_on(0, 69, 1.0));
        }
        let mut block =
            ProcessBlock { frames: max, input: &input, output: &mut output, events: &events };
        processor.process(&mut block);
        for ch in 0..output.channel_count() {
            for &s in output.channel(ch).unwrap() {
                peak = peak.max(s.abs());
            }
        }
    }
    processor.stop_processing();
    drop(processor);
    instance.deactivate();

    println!("{} peak output over 40 blocks: {peak}", plugin.display_name());
    assert!(peak > 0.0, "synth produced silence");
}

#[test]
#[ignore = "loads a real VST3 plugin; machine-specific"]
fn vst3_state_round_trips() {
    let catalog = PluginCatalog::scan();
    let Some(plugin) = pick_synth(&catalog) else {
        eprintln!("no known VST3 synth installed; skipping");
        return;
    };
    let host = Host::new();

    // Capture state from one instance.
    let mut a = host.load(&plugin).expect("load a");
    let s1 = a.save_state().expect("save_state");
    println!("{}: state is {} bytes", plugin.display_name(), s1.len());
    assert!(!s1.is_empty(), "expected non-empty VST3 state");

    // Restore it into a fresh instance and re-save — must be byte-identical.
    let mut b = host.load(&plugin).expect("load b");
    b.load_state(&s1).expect("load_state");
    let s2 = b.save_state().expect("re-save");
    assert_eq!(s1, s2, "VST3 state did not round-trip byte-for-byte");
    println!("VST3 state round-trips byte-for-byte ({} bytes)", s1.len());
}

#[test]
#[ignore = "loads a real VST3 plugin; machine-specific"]
fn vst3_enumerates_parameters() {
    let catalog = PluginCatalog::scan();
    let Some(plugin) = pick_synth(&catalog) else {
        eprintln!("no known VST3 synth installed; skipping");
        return;
    };
    let host = Host::new();
    let mut instance = host.load(&plugin).expect("load");
    let params = instance.params();
    println!("{}: {} parameters", plugin.display_name(), params.len());
    for p in params.iter().take(6) {
        let val = instance.param_value(p.id);
        let text = val.and_then(|v| instance.param_value_to_text(p.id, v));
        println!(
            "  #{:<6} {:<28} [{:.2}..{:.2}] = {:?} ({:?})",
            p.id, p.name, p.min_value, p.max_value, val, text
        );
    }
    assert!(!params.is_empty(), "expected the synth to expose parameters");
}

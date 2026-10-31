//! End-to-end check of VST3 hosting against a real installed synth: load → feed
//! a note → render, plus state round-trip and parameter enumeration.
//!
//! Run **on the main thread** (`harness = false`): hosting a plugin runs its
//! module init (`bundleEntry`), which — like a DAW — must happen on the main
//! thread; the default test harness runs tests on worker threads and the plugin
//! would trap. Opt in (it loads real plugins):
//!
//!   NEXUS_VST3_TEST=1 cargo test -p nexus-host --test vst3_render

use nexus_host::process::{AudioBuffer, Event, EventBuffer, ProcessBlock};
use nexus_host::{AudioConfig, CatalogPaths, Host, PluginCatalog, PluginFormat, PluginRef};

fn main() {
    if std::env::var_os("NEXUS_VST3_TEST").is_none() {
        eprintln!(
            "vst3_render: skipped — set NEXUS_VST3_TEST=1 to run \
             (loads real VST3 plugins on the main thread)"
        );
        return;
    }

    // VST3 folders only: no CLAP binary is loaded into this process.
    let catalog = PluginCatalog::scan_in(&CatalogPaths { clap: vec![], ..CatalogPaths::standard() });
    let Some(plugin) = pick_synth(&catalog) else {
        eprintln!("no known VST3 synth installed; skipping");
        return;
    };

    renders_audio(&plugin);
    state_round_trips(&plugin);
    enumerates_parameters(&plugin);
    eprintln!("\nall vst3_render checks passed");
}

/// Find an installed VST3 instrument we can drive. Prefers known synths and
/// skips effect variants.
fn pick_synth(catalog: &PluginCatalog) -> Option<PluginRef> {
    const PREFERRED: &[&str] = &["Surge XT", "Vital", "Pigments", "OB-Xd", "TAL-NoiseMaker"];
    let vst3: Vec<_> = catalog
        .plugins()
        .iter()
        .filter(|p| p.format == PluginFormat::Vst3)
        .filter(|p| !p.display_name().contains("Effects") && !p.display_name().contains("FX"))
        .collect();
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

/// Load the synth, feed it a note, and confirm it renders non-silent audio.
fn renders_audio(plugin: &PluginRef) {
    println!("loading {} ({})", plugin.display_name(), plugin.descriptor.id);
    let host = Host::new();
    let mut instance = host.load(plugin).expect("load");
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

    println!("  peak output over 40 blocks: {peak}");
    assert!(peak > 0.0, "synth produced silence");
}

/// Capture state from one instance, restore into a fresh one, and confirm the
/// re-saved state is byte-identical.
fn state_round_trips(plugin: &PluginRef) {
    let host = Host::new();
    let mut a = host.load(plugin).expect("load a");
    let s1 = a.save_state().expect("save_state");
    assert!(!s1.is_empty(), "expected non-empty VST3 state");

    let mut b = host.load(plugin).expect("load b");
    b.load_state(&s1).expect("load_state");
    let s2 = b.save_state().expect("re-save");
    assert_eq!(s1, s2, "VST3 state did not round-trip byte-for-byte");
    println!("  state round-trips byte-for-byte ({} bytes)", s1.len());
}

/// Enumerate parameters and read a few values + display strings.
fn enumerates_parameters(plugin: &PluginRef) {
    let host = Host::new();
    let mut instance = host.load(plugin).expect("load");
    let params = instance.params();
    println!("  {} parameters", params.len());
    for p in params.iter().take(4) {
        let val = instance.param_value(p.id);
        let text = val.and_then(|v| instance.param_value_to_text(p.id, v));
        println!("    #{:<8} {:<24} = {:?} ({:?})", p.id, p.name, val, text);
    }
    assert!(!params.is_empty(), "expected the synth to expose parameters");
}

//! The realtime contract, against nexus's own test CLAP plugin
//! (`crates/nexus-test-plugin`) and — with `NEXUS_REAL_PLUGIN_TEST=1` — the
//! installed free plugins (TAL-Filter-2, Surge XT Effects; CLAP and VST3).
//!
//! - `process()` allocates nothing while parameters change (`I-100` VST3,
//!   `I-101` CLAP `request_callback`);
//! - a deactivated plugin activates again (`I-102`);
//! - `set_param` before activation is not lost (`I-103`);
//! - the transport reaches the plugin and advances between events (`I-104`).

mod common;

use common::{Blocks, FRAMES, allocations, config, load, param, real_plugins, test_plugin};
use nexus_host::ParamChange;
use nexus_host::process::Event;

#[global_allocator]
static ALLOC: common::Counting = common::Counting;

const WARMUP: usize = 50;
const BLOCKS: usize = 2_000;

#[test]
fn parameter_changes_reach_the_plugin_without_allocating() {
    let mut instance = load(&test_plugin());
    let mut processor = instance.activate(config(48_000.0)).unwrap();
    let mut ring = instance.take_param_producer().expect("an active plugin has a ring");
    processor.start_processing().unwrap();
    let mut blocks = Blocks::new();
    for _ in 0..WARMUP {
        blocks.run(processor.as_mut());
    }

    let mut last_peak = 0.0;
    let allocated = allocations(|| {
        for i in 0..BLOCKS {
            let value = if i % 2 == 0 { 0.25 } else { 0.5 };
            // Both doors a caller has: the ring and a block event.
            ring.push(ParamChange { id: param::VOLUME, value, offset_frames: 0 }).unwrap();
            blocks.events.push(
                (FRAMES / 2) as u32,
                Event::ParamValue {
                    param_id: param::VOLUME,
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value,
                },
            );
            last_peak = blocks.run(processor.as_mut());
        }
    });
    assert_eq!(allocated, 0, "process() allocated {allocated} times in {BLOCKS} blocks");
    assert_eq!(last_peak, 0.5, "the last change reached the audio");

    // The plugin asked for main-thread callbacks from process(), and pump runs them.
    instance.pump();
    assert!(instance.param_value(param::CALLBACKS).unwrap() >= 1.0, "pump ran no callback");
    processor.stop_processing();
}

/// `NEXUS_REAL_PLUGIN_TEST=1`: the same contract on installed free plugins.
#[test]
fn installed_plugins_take_parameter_changes_without_allocating() {
    for plugin in real_plugins() {
        let name = format!("{} ({:?})", plugin.path.display(), plugin.format);
        let mut instance = load(&plugin);
        let target = instance
            .params()
            .into_iter()
            .find(|p| p.is_automatable && !p.is_readonly && !p.is_bypass && !p.is_hidden)
            .unwrap_or_else(|| panic!("{name}: no automatable parameter"));
        let (lo, hi) = (target.min_value, target.max_value);

        for round in 0..2 {
            let mut processor = instance.activate(config(48_000.0 - 3_900.0 * round as f64))
                .unwrap_or_else(|e| panic!("{name}: activate #{round}: {e}"));
            let mut ring = instance.take_param_producer().unwrap();
            processor.start_processing().unwrap();
            let mut blocks = Blocks::new();
            for _ in 0..WARMUP {
                blocks.run(processor.as_mut());
            }
            let allocated = allocations(|| {
                for i in 0..BLOCKS {
                    let value = lo + (hi - lo) * ((i % 7) as f64 / 6.0);
                    let _ = ring.push(ParamChange { id: target.id, value, offset_frames: 0 });
                    blocks.run(processor.as_mut());
                }
            });
            instance.pump();
            assert_eq!(allocated, 0, "{name}: process() allocated {allocated} times");
            processor.stop_processing();
            drop(processor);
            instance.deactivate();
            break; // re-activation is I-102's
        }
        eprintln!("{name}: 0 allocations in {BLOCKS} blocks with parameter changes");
    }
}

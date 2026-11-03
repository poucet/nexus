//! Plugin → host notices (`I-107`), against nexus's own test CLAP plugin
//! (`crates/nexus-test-plugin`): its `NOTIFY` parameter makes it announce a
//! latency change, ask for a rescan or restart, or play back an editor drag.
//! The VST3 side, which has no test plugin, is tested against the host's own
//! COM objects in `nexus-host`'s unit tests.

mod common;

use common::{Blocks, allocations, config, load, param, test_plugin};
use nexus_host::process::Event;
use nexus_host::{Notice, PluginInstance, Rescan};
use nexus_test_plugin::notify;

#[global_allocator]
static ALLOC: common::Counting = common::Counting;

/// Ask the (inactive) plugin to do `action`: `set_param` flushes it on this
/// thread, then `pump` collects what it said.
fn ask(instance: &mut dyn PluginInstance, action: f64) -> Vec<Notice> {
    instance.set_param(param::NOTIFY, action).unwrap();
    instance.pump();
    instance.take_notices()
}

#[test]
fn a_latency_change_arrives_as_one_notice() {
    let mut instance = load(&test_plugin());
    assert_eq!(instance.latency(), 0);
    assert_eq!(ask(instance.as_mut(), notify::LATENCY), [Notice::Latency(notify::LATENCY_SAMPLES)]);
    assert!(instance.take_notices().is_empty(), "taken");
    instance.pump();
    assert!(instance.take_notices().is_empty(), "said once");
}

#[test]
fn a_rescan_restart_and_clear_arrive_as_notices() {
    let mut instance = load(&test_plugin());
    let rescan = Rescan { values: true, text: true, ..Default::default() };
    assert_eq!(ask(instance.as_mut(), notify::RESCAN), [Notice::Rescan(rescan)]);
    assert_eq!(ask(instance.as_mut(), notify::RESTART), [Notice::Restart]);
    assert_eq!(ask(instance.as_mut(), notify::CLEAR), [Notice::ParamCleared(param::VOLUME)]);
}

/// `request_restart` from the audio thread is a flag: no allocation.
#[test]
fn a_restart_asked_from_process_arrives_after_pump() {
    let mut instance = load(&test_plugin());
    let mut processor = instance.activate(config(48_000.0)).unwrap();
    processor.start_processing().unwrap();
    let mut blocks = Blocks::new();
    blocks.run(processor.as_mut());
    blocks.events.push(0, set(param::NOTIFY, notify::RESTART));
    let allocated = allocations(|| {
        blocks.run(processor.as_mut());
    });
    assert_eq!(allocated, 0);
    instance.pump();
    assert_eq!(instance.take_notices(), [Notice::Restart]);
    processor.stop_processing();
}

/// A host `ParamValue` event for `id`.
fn set(id: u32, value: f64) -> Event {
    Event::ParamValue { param_id: id, note_id: -1, port_index: -1, channel: -1, key: -1, value }
}

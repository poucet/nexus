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

/// The notices an editor drag on `VOLUME` makes, from the test plugin.
fn drag() -> Vec<Notice> {
    let mut out = vec![Notice::GestureBegin(param::VOLUME)];
    for value in notify::EDIT_VALUES {
        out.push(Notice::ParamValue { id: param::VOLUME, value });
    }
    out.push(Notice::GestureEnd(param::VOLUME));
    out
}

/// Output events from the main-thread `params.flush` of an inactive plugin.
#[test]
fn an_edit_flushed_on_the_owning_thread_arrives_with_its_gesture() {
    let mut instance = load(&test_plugin());
    assert_eq!(ask(instance.as_mut(), notify::EDIT), drag());
    assert_eq!(instance.param_value(param::VOLUME), Some(0.3));
}

/// Output events from `process()`: in order, through the preallocated ring,
/// with no allocation on the audio thread.
#[test]
fn output_events_from_process_arrive_in_order_without_allocating() {
    let mut instance = load(&test_plugin());
    let mut processor = instance.activate(config(48_000.0)).unwrap();
    processor.start_processing().unwrap();
    let mut blocks = Blocks::new();
    for _ in 0..10 {
        blocks.run(processor.as_mut());
    }
    const DRAGS: usize = 100; // 500 notices: within the ring between two pumps
    let allocated = allocations(|| {
        for _ in 0..DRAGS {
            blocks.events.push(0, set(param::NOTIFY, notify::EDIT));
            blocks.run(processor.as_mut());
        }
    });
    assert_eq!(allocated, 0, "process() allocated {allocated} times");
    instance.pump();
    assert_eq!(instance.take_notices(), drag().repeat(DRAGS));
    processor.stop_processing();
}

/// `params.request_flush` on an inactive plugin: `pump` flushes it, and what
/// the flush writes out arrives.
#[test]
fn a_requested_flush_runs_on_pump() {
    let mut instance = load(&test_plugin());
    instance.set_param(param::VOLUME, 0.75).unwrap();
    assert_eq!(ask(instance.as_mut(), notify::FLUSH), [Notice::ParamValue { id: param::VOLUME, value: 0.75 }]);
}

/// A processor that was dropped still has its notices read: the next
/// activation drains them first.
#[test]
fn notices_written_before_a_reactivation_are_kept() {
    let mut instance = load(&test_plugin());
    let mut processor = instance.activate(config(48_000.0)).unwrap();
    processor.start_processing().unwrap();
    let mut blocks = Blocks::new();
    blocks.events.push(0, set(param::NOTIFY, notify::EDIT));
    blocks.run(processor.as_mut());
    processor.stop_processing();
    drop(processor);
    instance.deactivate();
    let _processor = instance.activate(config(44_100.0)).unwrap();
    instance.pump();
    assert_eq!(instance.take_notices(), drag());
}

/// VST3 `outputParameterChanges`, without a VST3 plugin: what a plugin writes
/// in a block is read back as plain values in offset order and queued for the
/// owning thread, without allocating.
#[test]
fn vst3_output_parameter_changes_become_notices_without_allocating() {
    use nexus_host::vst3::param_changes::HostParameterChanges;
    use nexus_host::vst3::params::Normalizer;

    let changes = HostParameterChanges::output();
    let normalizer = Normalizer::sample([(1, 0), (2, 0)], |id, x| if id == 1 { x } else { 20.0 * 1000f64.powf(x) });
    let (mut sender, mut receiver) = nexus_host::notice::ring(nexus_host::notice::RING);
    const BLOCKS: usize = 200;
    let allocated = allocations(|| {
        for _ in 0..BLOCKS {
            changes.begin();
            // The plugin, in process(): a meter at the end, a knob it moved twice.
            changes.push(2, 255, 1.0);
            changes.push(1, 0, 0.25);
            changes.push(1, 128, 0.5);
            changes.drain_notices(&normalizer, &mut sender);
        }
    });
    assert_eq!(allocated, 0, "reading output changes allocated {allocated} times");
    let mut out = Vec::new();
    receiver.drain_into(&mut out);
    let block = [
        Notice::ParamValue { id: 1, value: 0.25 },
        Notice::ParamValue { id: 1, value: 0.5 },
        Notice::ParamValue { id: 2, value: 20_000.0 },
    ];
    assert_eq!(out, block.repeat(BLOCKS));
    assert_eq!(receiver.dropped(), 0);
}

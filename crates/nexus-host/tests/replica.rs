//! An instance made again on another owning thread ([`Replica`]): loaded on
//! one thread, replicated onto a second where its (fake) editor opens, then
//! back, with `thread-check` naming each instance's own thread and nothing
//! of its state lost. Against nexus's test CLAP plugin, by path and in
//! process; with `NEXUS_REAL_PLUGIN_TEST=1`, also installed CLAP and VST3
//! plugins, timed.

mod common;

use std::time::{Duration, Instant};

use common::{Blocks, Owner, config, param, real_plugins, test_plugin};
use nexus_host::process::Event;
use nexus_host::{AudioProcessor, Host, Notice, PluginInstance};
use nexus_test_plugin::notify;

fn set(param_id: u32, value: f64) -> Event {
    Event::ParamValue { param_id, note_id: -1, port_index: -1, channel: -1, key: -1, value }
}

/// Start `processor` and play one block with `events`; the output's peak.
fn play(processor: &mut dyn AudioProcessor, events: &[Event]) -> f32 {
    let mut blocks = Blocks::new();
    for event in events {
        blocks.events.push(0, event.clone());
    }
    blocks.run(processor)
}

/// A parent view for the fake editor, which never touches it.
#[cfg(target_os = "macos")]
fn fake_parent() -> raw_window_handle::RawWindowHandle {
    let view = std::ptr::NonNull::<std::ffi::c_void>::dangling();
    raw_window_handle::RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(view))
}

#[test]
fn an_instance_moves_to_another_thread_for_its_editor_and_back() {
    let worker = Owner::spawn("worker");
    let main = Owner::spawn("main");

    // Loaded on the worker, set and playing there.
    let plugin = test_plugin();
    worker.run(move |slot| *slot = Some(Host::new().load(&plugin).unwrap()));
    let mut playing = worker.on(|p| {
        p.set_param(param::VOLUME, 0.25).unwrap();
        let mut processor = p.activate(config(48_000.0)).unwrap();
        processor.start_processing().unwrap();
        processor
    });
    assert_eq!(play(playing.as_mut(), &[]), 0.25);

    // Made again on the main thread while the worker's copy still plays.
    let replica = worker.replica();
    main.take(replica);
    assert_eq!(main.on(|p| p.param_value(param::VOLUME)), Some(0.25), "the state crossed");
    assert_eq!(play(playing.as_mut(), &[]), 0.25, "the original is untouched");

    // Its editor opens there, and the user drags VOLUME in it.
    #[cfg(target_os = "macos")]
    {
        let size = main.on(|p| p.open_editor_in(fake_parent()).unwrap());
        assert_eq!(size.width, nexus_test_plugin::EDITOR_SIZE.0);
        assert_eq!(main.on(|p| p.param_value(param::EDITOR)), Some(2.0), "created on main, not audio");
    }
    let notices = main.on(|p| {
        p.set_param(param::NOTIFY, notify::EDIT).unwrap(); // the drag, through the editor's flush
        p.pump();
        p.take_notices()
    });
    assert_eq!(notices.first(), Some(&Notice::GestureBegin(param::VOLUME)));
    assert_eq!(main.on(|p| p.param_value(param::VOLUME)), Some(0.3));

    // The host swaps processors: the main thread's copy plays now.
    let mut processor = main.on(|p| {
        let mut processor = p.activate(config(48_000.0)).unwrap();
        processor.start_processing().unwrap();
        processor
    });
    playing.stop_processing();
    drop(playing);
    assert_eq!(worker.on(|p| p.param_value(param::MISPLACED)), Some(0.0));
    worker.drop_instance();
    assert!((play(processor.as_mut(), &[]) - 0.3).abs() < 1e-6);

    // The editor closes: back to the worker, with the edit.
    main.on(|p| p.close_editor());
    let replica = main.replica();
    worker.take(replica);
    processor.stop_processing();
    drop(processor);
    assert_eq!(main.on(|p| p.param_value(param::MISPLACED)), Some(0.0));
    main.drop_instance();

    assert_eq!(worker.on(|p| p.param_value(param::VOLUME)), Some(0.3), "the edit came back");
    let mut processor = worker.on(|p| {
        let mut processor = p.activate(config(48_000.0)).unwrap();
        processor.start_processing().unwrap();
        processor
    });
    // A parameter event asks for a main-thread callback: the worker is the
    // new copy's main thread, and the processing thread its audio thread.
    assert!((play(processor.as_mut(), &[set(param::VOLUME, 0.5)]) - 0.5).abs() < 1e-6);
    worker.on(|p| p.pump());
    assert_eq!(worker.on(|p| p.param_value(param::THREAD_ON_MAIN)), Some(2.0), "main, not audio");
    assert_eq!(worker.on(|p| p.param_value(param::THREAD_IN_PROCESS)), Some(1.0), "audio, not main");
    assert_eq!(worker.on(|p| p.param_value(param::MISPLACED)), Some(0.0));
    processor.stop_processing();
    drop(processor);
    worker.drop_instance();
}

#[test]
fn an_in_process_plugin_replicates_too() {
    let worker = Owner::spawn("worker");
    let main = Owner::spawn("main");
    worker.run(|slot| {
        let mut p = Host::new().load_clack::<nexus_test_plugin::Entry>(nexus_test_plugin::ID).unwrap();
        p.set_param(param::VOLUME, 0.75).unwrap();
        *slot = Some(p);
    });
    main.take(worker.replica());
    assert_eq!(main.on(|p| p.param_value(param::VOLUME)), Some(0.75));
    assert_eq!(main.on(|p| p.param_value(param::MISPLACED)), Some(0.0));
}

/// What a switch costs, per plugin: the test plugin always, and with
/// `NEXUS_REAL_PLUGIN_TEST=1` installed CLAP and VST3 plugins too. A switch
/// is the replica on one thread, then its load and activation on the other;
/// timed over [`SWITCHES`] moves back and forth, with the state checked on
/// every move.
#[test]
fn a_switch_keeps_the_values_and_costs_about_one_load() {
    for plugin in std::iter::once(test_plugin()).chain(real_plugins()) {
        let name = plugin.path.file_name().unwrap().to_string_lossy().into_owned();
        let threads = [Owner::spawn("worker"), Owner::spawn("main")];
        let plugin_ = plugin.clone();
        threads[0].run(move |slot| *slot = Some(Host::new().load(&plugin_).unwrap()));

        // A value off its default, on the plugin's own curve.
        threads[0].on(|p| {
            let param = p
                .params()
                .into_iter()
                .find(|q| q.is_automatable && !q.is_readonly && !q.is_bypass && !q.is_stepped)
                .expect("a continuous parameter");
            p.set_param_normalized(param.id, 0.3).unwrap();
        });
        let mut steps = [Vec::new(), Vec::new(), Vec::new()]; // replica, load, activate
        for n in 0..SWITCHES {
            let (from, to) = (&threads[n % 2], &threads[1 - n % 2]);
            // The same replica made on the thread it comes from: the one on
            // the other thread must give the same values. (A state round
            // trip need not be byte-equal: Surge XT's is not.)
            let expected = from.on(|p| values(p.replica().unwrap().load().unwrap().as_mut()));
            let started = Instant::now();
            let replica = from.replica();
            steps[0].push(started.elapsed());
            let started = Instant::now();
            to.take(replica);
            steps[1].push(started.elapsed());
            // Before activating: Surge XT Effects CLAP sets its empty FX
            // slot's values when activated, whichever thread it is on.
            assert_values_eq(&to.on(values), &expected, &name);
            let activate = to.on(|p| {
                let started = Instant::now();
                let processor = p.activate(config(48_000.0)).unwrap();
                let took = started.elapsed();
                drop(processor);
                p.deactivate();
                took
            });
            steps[2].push(activate);
            from.drop_instance();
        }

        let [replica, load, activate] = steps.map(median);
        eprintln!(
            "{name}: a switch {} (median of {SWITCHES}): replica {}, load with state {}, activate {}",
            ms(replica + load + activate),
            ms(replica),
            ms(load),
            ms(activate)
        );
    }
}

const SWITCHES: usize = 9;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// Every writable parameter's value.
fn values(p: &mut dyn PluginInstance) -> Vec<(u32, f64)> {
    let params = p.params();
    params.iter().filter(|q| !q.is_readonly).filter_map(|q| Some((q.id, p.param_value(q.id)?))).collect()
}

fn assert_values_eq(there: &[(u32, f64)], here: &[(u32, f64)], name: &str) {
    assert_eq!(there.len(), here.len(), "{name}");
    for ((id, a), (_, b)) in there.iter().zip(here) {
        assert!((a - b).abs() < 1e-6, "{name}: parameter {id} is {a} there, {b} here");
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

/// A replica whose load fails can be loaded again: the host keeps a clone
/// before handing one to `load`. The plugin's binary goes missing between
/// the replica and its load, so the load fails; once the binary is back, the
/// clone loads with the state the original had.
#[test]
fn a_cloned_replica_loads_after_a_failed_load() {
    let dir = std::env::temp_dir().join(format!("nexus-replica-retry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut plugin = test_plugin();
    let binary = dir.join(plugin.path.file_name().unwrap());
    std::fs::copy(&plugin.path, &binary).unwrap();
    plugin.path = binary.clone();

    let worker = Owner::spawn("worker");
    let main = Owner::spawn("main");
    worker.run(move |slot| {
        let mut p = Host::new().load(&plugin).unwrap();
        p.set_param(param::VOLUME, 0.4).unwrap();
        *slot = Some(p);
    });
    let replica = worker.replica();
    let kept = replica.clone();
    assert_eq!(kept.state(), replica.state());
    assert_eq!(kept.binary(), replica.binary());
    // The binary goes out from under it (so dyld cannot hand back the
    // image it already has, the original goes first).
    worker.drop_instance();
    let away = dir.join("away");
    std::fs::rename(&binary, &away).unwrap();

    let failed = main.run(move |_| replica.load().err().map(|e| e.to_string()));
    assert!(
        failed.is_some(),
        "the load fails while the binary is missing"
    );

    std::fs::rename(&away, &binary).unwrap();
    main.take(kept);
    assert_eq!(
        main.on(|p| p.param_value(param::VOLUME)),
        Some(0.4),
        "the clone carries the state"
    );
    main.drop_instance();
    drop((worker, main));
    let _ = std::fs::remove_dir_all(&dir);
}

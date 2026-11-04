//! An instance made again on another owning thread ([`Replica`]): loaded on
//! one thread, replicated onto a second where its (fake) editor opens, then
//! back, with `thread-check` naming each instance's own thread and nothing
//! of its state lost. Against nexus's test CLAP plugin, by path and in
//! process; with `NEXUS_REAL_PLUGIN_TEST=1`, also installed CLAP and VST3
//! plugins, timed.

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{Blocks, config, param, real_plugins, test_plugin};
use nexus_host::process::Event;
use nexus_host::{AudioProcessor, Host, Notice, PluginInstance, PluginRef, Replica};
use nexus_test_plugin::notify;

type Job = Box<dyn FnOnce(&mut Option<Box<dyn PluginInstance>>) + Send>;

/// A thread that owns at most one instance and runs jobs on it, as a host's
/// plugin thread or main thread would.
struct Owner {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Owner {
    fn spawn(name: &str) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let mut slot = None;
                for job in rx {
                    job(&mut slot);
                }
                drop(slot); // on this thread, as the formats ask
            })
            .unwrap();
        Self { jobs: Some(jobs), thread: Some(thread) }
    }

    /// Run `f` on this thread with its instance slot, and wait for the answer.
    fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Option<Box<dyn PluginInstance>>) -> R + Send + 'static,
    ) -> R {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |slot| tx.send(f(slot)).unwrap());
        self.jobs.as_ref().unwrap().send(job).unwrap();
        rx.recv().expect("the owner thread answered")
    }

    /// Run `f` on the instance this thread owns.
    fn on<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn PluginInstance) -> R + Send + 'static,
    ) -> R {
        self.run(|slot| f(slot.as_deref_mut().expect("this thread owns an instance")))
    }

    /// Make this thread's instance from `replica`.
    fn take(&self, replica: Replica) {
        self.run(move |slot| *slot = Some(replica.load().expect("the replica loads")));
    }

    /// This thread's instance, replicated; the instance stays.
    fn replica(&self) -> Replica {
        self.on(|p| p.replica().expect("the instance replicates"))
    }

    /// Drop this thread's instance (deactivated), on this thread.
    fn drop_instance(&self) {
        self.run(|slot| {
            if let Some(mut p) = slot.take() {
                p.deactivate();
            }
        });
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

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

/// `NEXUS_REAL_PLUGIN_TEST=1`: installed CLAP and VST3 plugins replicate
/// onto another thread with their state, and what it costs.
#[test]
fn installed_plugins_replicate_onto_another_thread() {
    for plugin in real_plugins() {
        let name = plugin.path.file_name().unwrap().to_string_lossy().into_owned();
        let worker = Owner::spawn("worker");
        let main = Owner::spawn("main");

        let cold = load_on(&worker, &plugin);
        let warm = load_on(&main, &plugin); // a second instance: the binary is loaded
        main.drop_instance();

        // A value off its default, on the plugin's own curve.
        worker.on(|p| {
            let param = p
                .params()
                .into_iter()
                .find(|q| q.is_automatable && !q.is_readonly && !q.is_bypass && !q.is_stepped)
                .expect("a continuous parameter");
            p.set_param_normalized(param.id, 0.3).unwrap();
        });
        // The same replica made on the worker itself: the one on another
        // thread must give the same values.
        let same_thread = worker.on(|p| values(p.replica().unwrap().load().unwrap().as_mut()));

        let started = Instant::now();
        let replica = worker.replica();
        let saved = started.elapsed();
        let started = Instant::now();
        main.take(replica);
        let loaded = started.elapsed();

        let there = main.on(values);
        assert_values_eq(&there, &same_thread, &name);
        // And back to the worker (a state round trip need not be byte-equal:
        // Surge XT's is not).
        let back = main.replica();
        worker.drop_instance();
        worker.take(back);
        assert_values_eq(&worker.on(values), &there, &name);

        eprintln!(
            "{name}: cold load {}, second instance {}; replica {} + load with state {}",
            ms(cold),
            ms(warm),
            ms(saved),
            ms(loaded)
        );
    }
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

fn load_on(owner: &Owner, plugin: &PluginRef) -> Duration {
    let plugin = plugin.clone();
    owner.run(move |slot| {
        let started = Instant::now();
        *slot = Some(Host::new().load(&plugin).unwrap());
        started.elapsed()
    })
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

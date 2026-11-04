//! What a host asks before moving an instance to the main thread for its
//! editor (`nexus_host::binary`): how many instances of its binary are
//! alive, and which thread made the first. One test, alone in its binary,
//! because the counts are per process.

mod common;

use common::{Owner, test_plugin};
use nexus_host::Host;
use nexus_host::binary::{first_thread, live_instances};

fn thread_of(owner: &Owner) -> std::thread::ThreadId {
    owner.run(|_| std::thread::current().id())
}

#[test]
fn a_host_can_tell_when_a_move_is_safe_for_a_juce_plugin() {
    let plugin = test_plugin();
    let binary = plugin.path.clone();
    let worker = Owner::spawn("worker");
    let main = Owner::spawn("main");
    assert_eq!((live_instances(&binary), first_thread(&binary)), (0, None));

    // One instance, on the worker: the binary's first thread is the worker.
    let loaded = plugin.clone();
    worker.run(move |slot| *slot = Some(Host::new().load(&loaded).unwrap()));
    assert_eq!(worker.on(|p| p.binary().map(|b| b.to_path_buf())), Some(binary.canonicalize().unwrap()));
    assert_eq!(live_instances(&binary), 1);
    assert_eq!(first_thread(&binary), Some(thread_of(&worker)));

    // Alone: its replica may go to the main thread. Once the worker's copy
    // is gone, the main thread's is the binary's only instance, but the
    // binary's first thread is still the worker, as a JUCE plugin's message
    // thread would be.
    let replica = worker.replica();
    assert_eq!(replica.binary(), binary);
    main.take(replica);
    assert_eq!(live_instances(&binary), 2);
    worker.drop_instance();
    assert_eq!(live_instances(&binary), 1);
    assert_eq!(first_thread(&binary), Some(thread_of(&worker)));

    // Gone entirely, then loaded on the main thread: the first thread is
    // the main thread, so every instance of the binary stays there safely.
    main.drop_instance();
    assert_eq!((live_instances(&binary), first_thread(&binary)), (0, None));
    let loaded = plugin.clone();
    main.run(move |slot| *slot = Some(Host::new().load(&loaded).unwrap()));
    assert_eq!(first_thread(&binary), Some(thread_of(&main)));

    // In process: the binary is this executable.
    let in_process = Host::new().load_clack::<nexus_test_plugin::Entry>(nexus_test_plugin::ID).unwrap();
    let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
    assert_eq!(in_process.binary(), Some(exe.as_path()));
    assert_eq!(live_instances(&exe), 1);
}

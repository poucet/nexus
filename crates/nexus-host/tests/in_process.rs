//! `N11`: a CLAP plugin compiled into the test binary loads with no bundle.

mod common;

use common::{Blocks, config, param};
use nexus_host::Host;

#[test]
fn a_clack_plugin_in_this_process_loads_and_plays() {
    let mut instance = Host::new()
        .load_clack::<nexus_test_plugin::Entry>(nexus_test_plugin::ID)
        .expect("the in-process entry loads");
    assert_eq!(instance.params().len(), 9);
    instance.set_param(param::VOLUME, 0.25).unwrap();

    let mut processor = instance.activate(config(48_000.0)).unwrap();
    processor.start_processing().unwrap();
    assert_eq!(Blocks::new().run(processor.as_mut()), 0.25);
    processor.stop_processing();
    drop(processor);
    instance.deactivate();
}

#[test]
fn an_unknown_id_is_an_error() {
    let err = Host::new().load_clack::<nexus_test_plugin::Entry>("no.such.plugin").err();
    assert!(err.is_some(), "no plugin by that id");
}

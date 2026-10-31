//! Integration tests for nexus-wrap.
//!
//! None of them probes an installed plugin: each state is built over an empty
//! catalog (`I-99`). Probing loads a plugin's binary into this process, where a
//! broken one panics the test binary and a copy-protected one can block on a
//! licence dialog.

use std::sync::Arc;

use nexus_host::PluginCatalog;
use nexus_tools::bridge::{HostBridge, MainThreadHost};
use nexus_tools::{ToolRegistry, ToolsService};
use nexus_wrap::bridge::InnerPluginState;
use nexus_wrap::state::WrapStateEnvelope;

/// A state with no installed plugin in its catalog.
fn inner() -> InnerPluginState {
    InnerPluginState::with_catalog(PluginCatalog::default())
}

#[test]
fn inner_plugin_state_initial_session() {
    let mut state = inner();
    let session = state.session();
    assert_eq!(session.loaded, None);
    assert_eq!(session.sample_rate, Some(48000.0));
}

#[test]
fn inner_plugin_state_list_plugins() {
    let mut state = inner();
    let plugins = state.list_plugins(None).unwrap();
    assert!(plugins.is_empty(), "the test catalog is empty: {plugins:?}");
}

#[test]
fn inner_plugin_state_params_empty_when_no_plugin() {
    let mut state = inner();
    let res = state.list_params(None, None, None);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code, nexus_tools::ErrorCode::NoPlugin);

    let vals = state.get_params(&[0, 1, 2]);
    assert!(vals.is_err());
    assert_eq!(vals.unwrap_err().code, nexus_tools::ErrorCode::NoPlugin);
}


#[test]
fn wrap_state_envelope_round_trip() {
    let raw_payload = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42, 0x00, 0xFF];
    let envelope = WrapStateEnvelope::new(
        Some("com.test.synth".into()),
        Some("clap".into()),
        Some("Test Synth".into()),
        raw_payload.clone(),
    );

    let serialized = serde_json::to_vec(&envelope).unwrap();
    let deserialized: WrapStateEnvelope = serde_json::from_slice(&serialized).unwrap();

    assert_eq!(deserialized.version, WrapStateEnvelope::CURRENT_VERSION);
    assert_eq!(deserialized.plugin_id.as_deref(), Some("com.test.synth"));
    assert_eq!(deserialized.format.as_deref(), Some("clap"));
    assert_eq!(deserialized.name.as_deref(), Some("Test Synth"));
    assert_eq!(deserialized.inner_state, raw_payload);
}

#[tokio::test]
async fn tools_bridge_communication_with_inner_state() {
    let (bridge, rx) = HostBridge::channel(Arc::new(|| {}));
    let service = Arc::new(ToolsService::new(bridge));
    let registry = Arc::new(ToolRegistry::standard(service));

    // Spawn mock main thread drain loop
    std::thread::spawn(move || {
        let mut inner = inner();
        while let Ok(req) = rx.recv() {
            nexus_tools::bridge::handle(req, &mut inner);
        }
    });

    // Test get_session tool
    let tool = registry.get("get_session").unwrap();
    let res = tool.invoke(serde_json::json!({})).await.unwrap();
    assert!(res.get("corpus_directory").is_some());

    // Test list_plugins tool
    let tool = registry.get("list_plugins").unwrap();
    let res = tool.invoke(serde_json::json!({})).await.unwrap();
    assert!(res.get("plugins").is_some());
}

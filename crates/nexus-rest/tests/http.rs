//! End-to-end HTTP test: real server thread, real sockets, stub tool service.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nexus_tools::error::ToolError;
use nexus_tools::registry::ToolRegistry;
use nexus_tools::tools::NexusTools;
use nexus_tools::types::*;

struct Stub;

#[async_trait]
impl NexusTools for Stub {
    async fn get_session(&self, _req: GetSessionRequest) -> Result<SessionInfo, ToolError> {
        Ok(SessionInfo { loaded: None, sample_rate: Some(48000.0), corpus_directory: "/corpus".into() })
    }
    async fn decode_state(&self, _req: DecodeStateRequest) -> Result<DecodedState, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn edit_state(&self, _req: EditStateRequest) -> Result<EditResult, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn get_state(&self, _req: GetStateRequest) -> Result<StateBlobInfo, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn set_state(&self, _req: SetStateRequest) -> Result<StateBlobInfo, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn snapshot_state(&self, _req: SnapshotStateRequest) -> Result<SnapshotInfo, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn restore_snapshot(&self, _req: RestoreSnapshotRequest) -> Result<RestoreResult, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn list_snapshots(&self, _req: ListSnapshotsRequest) -> Result<SnapshotList, ToolError> {
        Ok(SnapshotList { snapshots: vec![] })
    }
    async fn diff_state(&self, _req: DiffStateRequest) -> Result<StateDiff, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn list_codecs(&self, _req: ListCodecsRequest) -> Result<CodecList, ToolError> {
        Ok(CodecList { codecs: vec![] })
    }
    async fn list_plugins(&self, _req: ListPluginsRequest) -> Result<ListPluginsResponse, ToolError> {
        Ok(ListPluginsResponse { plugins: vec![] })
    }
    async fn load_plugin(&self, _req: LoadPluginRequest) -> Result<LoadPluginResponse, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn list_presets(&self, _req: ListPresetsRequest) -> Result<ListPresetsResponse, ToolError> {
        Ok(ListPresetsResponse { presets: vec![] })
    }
    async fn load_preset(&self, _req: LoadPresetRequest) -> Result<LoadPresetResponse, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn save_preset(&self, _req: SavePresetRequest) -> Result<SavePresetResponse, ToolError> {
        Err(ToolError::no_plugin())
    }
    async fn list_params(&self, _req: ListParamsRequest) -> Result<ListParamsResponse, ToolError> {
        Ok(ListParamsResponse { total: 0, offset: 0, params: vec![] })
    }
    async fn get_params(&self, _req: GetParamsRequest) -> Result<GetParamsResponse, ToolError> {
        Ok(GetParamsResponse { params: vec![] })
    }
    async fn set_params(&self, _req: SetParamsRequest) -> Result<SetParamsResponse, ToolError> {
        Ok(SetParamsResponse { updated: vec![] })
    }
    async fn convert_param_value(&self, req: ConvertParamValueRequest) -> Result<ConvertParamValueResponse, ToolError> {
        Ok(ConvertParamValueResponse { id: req.id, value: req.value, display_text: req.text })
    }
    async fn play(&self, _req: PlayRequest) -> Result<PlayResponse, ToolError> {
        Ok(PlayResponse { queued: true, duration_beats: 4.0, total_events: 0 })
    }
    async fn render_audio(&self, _req: RenderAudioRequest) -> Result<RenderAudioResponse, ToolError> {
        Err(ToolError::no_plugin())
    }
}

const PORT: u16 = 19996;

fn request(method: &str, path: &str, body: &str) -> String {
    // The server may take a moment to bind.
    let mut last_err = None;
    for _ in 0..50 {
        match TcpStream::connect(("127.0.0.1", PORT)) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let request = format!(
                    "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(request.as_bytes()).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                return response;
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    panic!("could not connect to nexus-rest test server: {last_err:?}");
}

#[test]
fn rest_serves_tools_debug_and_errors() {
    let registry = Arc::new(ToolRegistry::standard(Arc::new(Stub)));
    nexus_rest::start_server(registry, PORT);

    // GET /tools lists every tool with schemas.
    let tools = request("GET", "/tools", "");
    assert!(tools.starts_with("HTTP/1.1 200"), "{tools}");
    assert!(tools.contains("\"get_session\""));
    assert!(tools.contains("\"edit_state\""));
    assert!(tools.contains("input_schema"));

    // POST with an empty body defaults to {}.
    let session = request("POST", "/tools/get_session", "");
    assert!(session.starts_with("HTTP/1.1 200"), "{session}");
    assert!(session.contains("\"corpus_directory\":\"/corpus\""));

    // Tool errors map to status codes + a JSON error body.
    let no_plugin = request("POST", "/tools/get_state", "{}");
    assert!(no_plugin.starts_with("HTTP/1.1 409"), "{no_plugin}");
    assert!(no_plugin.contains("\"no_plugin\""));

    let unknown = request("POST", "/tools/nope", "{}");
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");

    let bad_json = request("POST", "/tools/get_session", "not json");
    assert!(bad_json.starts_with("HTTP/1.1 400"), "{bad_json}");

    // The debug page is served inline.
    let debug = request("GET", "/debug", "");
    assert!(debug.starts_with("HTTP/1.1 200"), "{debug}");
    assert!(debug.contains("Nexus Tools"));
}

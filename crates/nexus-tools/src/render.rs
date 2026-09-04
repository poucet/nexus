//! The half of `render_audio` that is the same on every host.
//!
//! Three [`MainThreadHost`](crate::bridge::MainThreadHost) implementations — the
//! app, the wrapper, the standalone MCP server — carried byte-identical copies
//! of everything either side of `render_offline`: build the options, read
//! `output_format`, write a file or encode base64, assemble the response.
//! Three copies is three places for a fix to miss, and this lane alone would
//! have had to write the flipped default, the `warnings` passthrough and the
//! error mapping into each of them.
//!
//! **What stays per-host is only what genuinely differs**: how each obtains an
//! instance to render on. `nexus-app` and `nexus-wrap` clone the live plugin
//! (save state → load a fresh headless instance → restore) so the render never
//! touches the audio callback; `nexus-mcp` has no callback to protect and
//! renders on the instance it holds.
//!
//! Lives in `nexus-tools` and not `nexus-host` because [`ToolError`] does:
//! `nexus-tools` depends on `nexus-host`, never the other way round.

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use nexus_host::HostError;
use nexus_host::fugue::OutputFormat;
use nexus_host::render::{RenderOptions, RenderResult};

use crate::error::ToolError;
use crate::types::{RenderAudioRequest, RenderAudioResponse};

/// Sample rate used when the request names none.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
/// Block size used when the request names none.
pub const DEFAULT_BLOCK_SIZE: usize = 512;
/// Tempo used when the request names none — reported in `warnings`, because a
/// fugue's whole timeline is in beats and 120 is a guess, not a fact.
pub const DEFAULT_TEMPO_BPM: f64 = 120.0;

/// The renderer options a request asks for, with nexus's defaults filled in.
pub fn render_options(req: &RenderAudioRequest) -> RenderOptions {
    RenderOptions {
        sample_rate: req.sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE) as f64,
        block_size: req.block_size.unwrap_or(DEFAULT_BLOCK_SIZE),
        tempo_bpm: req.tempo_bpm.unwrap_or(DEFAULT_TEMPO_BPM),
        fugue: req.fugue.compile(),
    }
}

/// Map a render failure onto the right tool error.
///
/// A request the renderer refuses — an endless `loop_mode`, say — is the
/// caller's to fix, so it answers `invalid_input` rather than blaming the
/// plugin for a decision nobody asked it to make.
pub fn render_error(error: HostError) -> ToolError {
    match error {
        HostError::InvalidRequest(message) => ToolError::invalid_input(message),
        other => ToolError::plugin(format!("render_offline: {other}")),
    }
}

/// Where a `"file"` render goes when the request names no path.
fn default_render_path() -> PathBuf {
    let dir = nexus_host::discovery::config::preset_directory().join("rendered");
    let _ = fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    dir.join(format!("sample_{stamp}.wav"))
}

/// Turn a finished render into the tool's response.
///
/// Writes the WAV or encodes it, per `output_format` — which now **defaults to
/// `file`** (FUGUE-API.md §6.6): the loop is `render_audio` → `analyze(wav)` in
/// another process, which reads a file, and a minute of stereo audio inline is
/// ~30 MB in one message. An unrecognised spelling no longer reaches here at
/// all: [`OutputFormat`] is a closed enum, so the tool surface rejects it with
/// `invalid_input` naming both accepted values.
///
/// Carries the renderer's `warnings` out to the caller and adds the one only
/// this layer knows about — that no tempo was given.
pub fn render_response(
    req: &RenderAudioRequest,
    result: RenderResult,
) -> Result<RenderAudioResponse, ToolError> {
    let mut warnings = result.warnings;
    if req.tempo_bpm.is_none() {
        warnings.push(format!(
            "no tempo_bpm given; used {DEFAULT_TEMPO_BPM} — every beat in the fugue is timed \
             against it"
        ));
    }

    let (wav_base64, file_path) = match req.output_format() {
        OutputFormat::File => {
            let path = req.output_path.as_ref().map_or_else(default_render_path, PathBuf::from);
            fs::write(&path, &result.wav_bytes)
                .map_err(|e| ToolError::internal(format!("write rendered wav: {e}")))?;
            (None, Some(path.display().to_string()))
        }
        OutputFormat::WavBase64 => (Some(BASE64.encode(&result.wav_bytes)), None),
    };

    Ok(RenderAudioResponse {
        sample_rate: result.sample_rate,
        channels: result.channels.len(),
        frames: result.frames,
        duration_s: result.duration_s,
        peak_db: result.peak_db,
        rms_db: result.rms_db,
        is_silent: result.is_silent,
        wav_base64,
        file_path,
        warnings,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(json: &str) -> RenderAudioRequest {
        serde_json::from_str(json).unwrap()
    }

    fn silent_result() -> RenderResult {
        RenderResult {
            channels: vec![vec![0.0; 8], vec![0.0; 8]],
            frames: 8,
            sample_rate: 48_000,
            duration_s: 0.0,
            peak_db: -120.0,
            rms_db: -120.0,
            is_silent: true,
            wav_bytes: b"RIFF....".to_vec(),
            warnings: vec!["from the renderer".into()],
        }
    }

    /// **Absent `output_format` writes a file** — the flipped default (§6.6).
    #[test]
    fn the_default_output_format_is_a_file() {
        let dir = std::env::temp_dir().join("nexus-render-glue-default");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("out.wav");
        let req = request(&format!(
            r#"{{"fugue":{{}},"output_path":{}}}"#,
            serde_json::to_string(&path.display().to_string()).unwrap()
        ));

        let res = render_response(&req, silent_result()).unwrap();
        assert_eq!(res.file_path.as_deref(), Some(path.display().to_string().as_str()));
        assert!(res.wav_base64.is_none(), "the default must not return inline audio");
        assert_eq!(fs::read(&path).unwrap(), b"RIFF....");
    }

    /// Asking for base64 still gets base64.
    #[test]
    fn wav_base64_is_still_available_by_name() {
        let req = request(r#"{"fugue":{},"output_format":"wav_base64"}"#);
        let res = render_response(&req, silent_result()).unwrap();
        assert!(res.file_path.is_none());
        assert_eq!(res.wav_base64.as_deref(), Some(BASE64.encode(b"RIFF....").as_str()));
    }

    /// The renderer's warnings reach the caller, and the glue adds the one only
    /// it can see.
    #[test]
    fn warnings_travel_and_an_absent_tempo_is_reported() {
        let req = request(r#"{"fugue":{},"output_format":"wav_base64"}"#);
        let res = render_response(&req, silent_result()).unwrap();
        assert!(res.warnings.iter().any(|w| w == "from the renderer"));
        assert!(res.warnings.iter().any(|w| w.contains("no tempo_bpm given; used 120")));

        let with_tempo = request(r#"{"fugue":{},"tempo_bpm":90,"output_format":"wav_base64"}"#);
        let res = render_response(&with_tempo, silent_result()).unwrap();
        assert!(!res.warnings.iter().any(|w| w.contains("no tempo_bpm")));
    }

    /// A request the renderer refuses is the caller's fault, not the plugin's.
    #[test]
    fn an_invalid_request_maps_to_invalid_input() {
        let err = render_error(HostError::InvalidRequest("endless".into()));
        assert_eq!(err.code, crate::error::ErrorCode::InvalidInput);
        assert_eq!(err.message, "endless");

        let err = render_error(HostError::Load("boom".into()));
        assert_eq!(err.code, crate::error::ErrorCode::Plugin);
    }

    /// Defaults are read off the request in one place, so the three hosts
    /// cannot drift.
    #[test]
    fn render_options_fill_the_documented_defaults() {
        let opts = render_options(&request(r#"{"fugue":{}}"#));
        assert_eq!(opts.sample_rate, 48_000.0);
        assert_eq!(opts.block_size, 512);
        assert_eq!(opts.tempo_bpm, 120.0);

        let opts = render_options(&request(
            r#"{"fugue":{},"sample_rate":44100,"block_size":256,"tempo_bpm":90}"#,
        ));
        assert_eq!((opts.sample_rate, opts.block_size, opts.tempo_bpm), (44_100.0, 256, 90.0));
    }
}

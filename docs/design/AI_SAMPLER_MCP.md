# AI Plugin Sampler & MCP Tool Suite

**Status:** proposal (gated on human review)
**Date:** 2026-08-31
**Author:** Antigravity & Chris

## Problem

We want AI agents (e.g. LLMs running via MCP) to interact with arbitrary CLAP and VST3 audio plugins as sound designers:
1. Discover what plugins are installed on the system (`list_plugins`).
2. Load any plugin into a session (`load_plugin`).
3. Browse and recall presets or saved states (`list_presets`, `load_preset`, `save_preset`).
4. Inspect and tweak parameters (`list_params`, `get_params`, `set_params`).
5. **Sample audio offline** (`render_audio`): play scheduled MIDI notes / chords / sweeps for $X$ seconds and return the rendered audio (as inlined base64 WAV or a file on disk) and audio metrics (peak dB, RMS dB, silence check), without requiring an active real-time sound card or audio driver.

This provides the sensory feedback loop enabling an AI agent to listen to a plugin preset/sound and analyze or sample it.

> **Downstream Sensory Perception & Reconstruction:** For downstream acoustic feature analysis, spectral diffing, and modular synthesis reconstruction driven by Sense and Flux, see `../../sense/docs/design/AI_AUDIO_PERCEPTION_AND_EXPLORATION.md`, `../../sense/docs/design/AI_PATCH_RECONSTRUCTION_CUJ.md`, and `../../sense/docs/design/AI_PATCH_RECONSTRUCTION_TECH.md`.

## Architecture

```
                  ┌── REST + Web Debug UI (axum loopback :9996)
Tool Registry ────┼── MCP Server (rmcp streamable HTTP / stdio)
(NexusTools trait)└── Direct Rust API
       │
       ▼
Main-Thread Bridge / Headless Host Engine
       │
       ├── Catalog & Presets (discovery cache, .nexuspreset)
       ├── Parameters (set_param, ParamChange ring)
       ├── State & Codecs (save_state, load_state, edit_state)
       └── Offline Audio Renderer (render_offline)
             │
             ▼
       AudioProcessor (CLAP / VST3) ──> Block Loop ──> WAV Encoder (hound)
```

## Tool Definitions

### 1. `play` & `render_audio` (Fugue-Powered Playback & Sampling)

Both take a **`CompositeFugue`** from the shared `simply-fugue` crate (`poucet/music-rs`, `crates/simply-fugue/src/composite.rs`) — the same JSON flux's `render_audio` takes, under the same tool name. (Droplets keeps its own fugue model and is not a consumer; the crate docs say why.)

- **Note Shorthand**: Array form `[beat, "C3", duration, velocity, channel]` or object form `{beat, note, duration, velocity, channel}`. Supports note names (`"C3"`, `"F#4"`, `"Bb2"`) or MIDI numbers `0..127`.
- **CC & Parameter Automation**: `ccs` / `params` points `[[beat, value], [beat, value, curve]]` with curves `none` (stepped, **the default**), `linear`, `exp` (`t²`), `log` (`1 − (1 − t)²`). A point's curve shapes the segment that *starts* at it.
- **Per-Note Expression**: `pitch_bends` / `pressures` lanes of `[beat, value, curve?]` aimed at one `(channel, note)` — semitones and `0..1` respectively; a point without a curve takes the lane's `interpolation`.
- **Raw events**: `raw_events` carries any of the ten `FugueEvent` kinds at a `beat_offset`.
- **Length and looping** are fields *of the fugue*: `duration_beats` (auto-sized from the content if omitted, floored at 1 beat) and `loop_mode` (`"once"` default, `{"times": n}`, `"loop"`, `"ping_pong"`).

#### `play` (Live Transport-Synced Playback)
Dispatches the fugue to the live `nexus-app` audio engine for real-time auditioning.
- **Request** (`PlayRequest`): `instance: String` (default `"default"`), `fugue: CompositeFugue`. There is no `quantize` or `loop_mode` argument on the request — looping is the fugue's own field.
- **Response** (`PlayResponse`): `queued`, `duration_beats`, `total_events`.

#### `render_audio` (Offline Sampler)
Renders audio offline at maximum computational speed through the loaded plugin — a headless clone of it in `nexus-app` / `nexus-wrap`, the held instance in `nexus-mcp` — without an audio driver. The request and response are `simply_fugue::{RenderAudioRequest, RenderAudioResponse}`, shared with flux verbatim.

- **Request** (`RenderAudioRequest`):
  - `instance`: `String` (default `"default"`)
  - `fugue`: `CompositeFugue` (required)
  - `tempo_bpm`: `Option<f64>` (default 120.0; an absent tempo is reported in `warnings`)
  - `sample_rate`: `Option<u32>` (default 48000)
  - `block_size`: `Option<usize>` (default 512; floored at 64, reported in `warnings`)
  - `output_format`: `Option<OutputFormat>` — the enum `"file"` | `"wav_base64"`, **default `"file"`**; any other spelling is refused (`invalid_input`) naming both
  - `output_path`: `Option<String>` (file target when `output_format` is `"file"`; otherwise `<presets dir>/rendered/sample_<unix-time>.wav`)

  There is no `duration_beats` / `seconds` argument: the length is the fugue's `duration_beats` at `tempo_bpm`, times the pass count, **plus a fixed 1.0 s release tail** so a note ending on the last beat keeps its decay. `loop_mode` `once` renders one pass and `{"times": n}` renders `n` passes (`0` counts as one, as on flux) with every event re-emitted `k · duration_beats` later and the tail added once after the last; `loop` / `ping_pong` are endless and are refused offline — they apply to `play`.

- **Response** (`RenderAudioResponse`):
  - `sample_rate: u32`, `channels: usize`, `frames: usize`, `duration_s: f64` — `frames` and `duration_s` **include the tail**
  - `peak_db: f64`, `rms_db: f64`, `is_silent: bool` (peak below −96 dBFS)
  - `wav_base64: Option<String>` | `file_path: Option<String>` — exactly one, per `output_format`
  - `warnings: Vec<String>` — everything decided on the caller's behalf: an assumed tempo, a floored block size, the tail, `times: 0`, and any event kind this plugin path cannot carry, with its count (VST3 takes note on/off only; CLAP takes raw MIDI and per-note expression)
  - `rms`, `peak_amplitude`: `Option<f32>` — linear twins of the dB pair, not filled by nexus today

- **Rendering rules**, the same numbers as flux so one fugue measures the same on both hosts: CC bytes `0–127` go to the plugin untouched — they *are* the normalized (travel) space its own knob moves in, so a CC sweep behaves like turning that knob; a CC or per-note lane with a curve is a ramp sampled every 64 frames (values rounded to a byte, held values not re-sent) and a curve-less lane stays one instantaneous byte per point; `params` lanes interpolate in the plugin's plain value on the same grid; a note-off precedes a note-on at the same sample.

- **Asymmetries with flux**: flux renders through one patch and nexus through a hosted plugin, so identical calls give comparable, not equal, audio; both hosts refuse renders past 60 s (I-96); flux reports errors as prose, nexus as `{code, message}`.

### 2. `list_plugins` & `load_plugin`

- `list_plugins`: returns `Vec<PluginInfo>` (`id`, `name`, `vendor`, `format`, `path`).
- `load_plugin`: loads by `id` (or unique fuzzy name).

### 3. `list_presets`, `load_preset`, `save_preset`

- `list_presets`: queries the presets directory (`~/Music/Nexus/Presets/...`).
- `load_preset`: loads a `.nexuspreset` file into the active instance.
- `save_preset`: exports current plugin identity + state to `.nexuspreset`.

### 4. `list_params`, `get_params`, `set_params`

- `list_params`: paged/filtered parameter query (`query`, `limit`, `offset`).
- `get_params`: batch parameter value query by ID.
- `set_params`: batch parameter updates `[{ id, value }]`.

## Consistency & Audio Thread Isolation

When `nexus-app` is running with the UI open and CPAL audio active, calling `render_audio` must not glitch live playback. We achieve this by:
1. Calling `instance.save_state()` to capture the live state.
2. Instantiating a headless clone `PluginInstance` on a background worker thread.
3. Loading the state (`clone.load_state(&state)`), activating the offline processor, running the render block-by-block, and tearing down the clone.
4. When running purely headless (e.g. `nexus-mcp` standalone binary), `Host::load` runs directly without any GUI overhead.

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

Leverages the **Droplets Fugue definition** (`../droplets/src/fugue/types.rs`) for musical performance and parameter gestures:

- **Note Shorthand**: Array form `[beat, "C3", duration, velocity, channel]` or object form `{beat, note, duration, velocity, channel}`. Supports note names (`"C3"`, `"F#4"`, `"Bb2"`) or MIDI numbers `0..127`.
- **CC & Parameter Automation**: Points `[[beat, value], [beat, value, curve]]` with interpolation curves (`linear`, `exp`, `log`, `none`).
- **Per-Note Expression**: Pitch bend and pressure/aftertouch lanes.
- **Composite Fugue**: Bundles notes, CCs, parameter ramps, and expression into one atomic performance.

#### `play` (Live Transport-Synced Playback)
Dispatches the fugue to the live `nexus-app` audio engine for real-time auditioning.
- **Request**:
  - `instance`: `String` (default `"default"`)
  - `fugue`: `FugueDefinition` or `CompositeFugue`
  - `quantize`: `Option<QuantizeMode>` (`"immediate"`, `"beat"`, `"bar"`, `"bars:N"`)
  - `loop_mode`: `Option<LoopMode>` (`"once"`, `"forever"`, `{"times": N}`)

#### `render_audio` (Offline Sampler)
Renders audio offline at maximum computational speed through the loaded plugin (or clone) without an audio driver.
- **Request**:
  - `instance`: `String` (default `"default"`)
  - `tempo_bpm`: `Option<f64>` (default 120.0)
  - `duration_beats`: `Option<f64>` or `seconds: Option<f64>` (0.05..60.0s, auto-sized if omitted)
  - `sample_rate`: `Option<u32>` (default 48000)
  - `block_size`: `Option<usize>` (default 512)
  - `fugue`: `Option<FugueDefinition>` (or simple `notes` list)
  - `output_format`: `Option<AudioFormat>` (`"wav_base64"` | `"file"`, default `"wav_base64"`)
  - `output_path`: `Option<String>` (optional file target)

- **Response**:
  - `duration_s`: `f64`
  - `sample_rate`: `u32`
  - `channels`: `usize`
  - `peak_db`: `f64`
  - `rms_db`: `f64`
  - `is_silent`: `bool`
  - `wav_base64`: `Option<String>`
  - `file_path`: `Option<String>`

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

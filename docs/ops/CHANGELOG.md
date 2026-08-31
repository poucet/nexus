# Nexus — CHANGELOG

> What belongs here, and what does not: @AGENTS.md §2. Newest first.

| Dates | Tag | |
|---|---|---|
| 2026-09-01 → … | — | **[Unreleased](#unreleased)** — 0.3 exploration & wrap |
| 2026-08-26 → 09-01 | `v0.2.0` | [Milestone 0.2: State Tools & AI Sampler](#v020--state-tools--ai-sampler) — MCP server, driver-free offline rendering, Fugue sequencing, codecs |
| 2026-06-01 → 08-26 | `v0.1.0` | [Milestone 0.1: Load & Hear](#v010--load--hear) — CLAP/VST3 hosting engine + standalone egui app |

---

## Unreleased

#### Realtime Parameter Automation Ring (`PARAM-AUTOMATION`) · 2026-09-01 00:30

**Lock-free parameter change ring (`rtrb`), CLAP & VST3 realtime event pipelines, and offline automation curve evaluation** (`nexus-host`).
- **`ParamChange` SPSC Ring (`host.rs`, `PluginInstance::activate`)**: Defined `ParamChange { id, value, offset_frames }`. `PluginInstance::activate` instantiates a lock-free 1024-capacity SPSC ring using `rtrb`, exposing the `rtrb::Producer<ParamChange>` to the host via `param_producer()`, `take_param_producer()`, and `set_param()`.
- **CLAP Event Processing (`clap/process.rs`, `clap/instance.rs`)**: `ClapAudioProcessor` drains parameter changes from the `rtrb::Consumer` per block into `ParamValueEvent`s on the input event list with sample frame offsets, and maps façade `Event::ParamValue/Mod/Gesture*` from `block.events`.
- **VST3 Parameter Changes COM Objects (`vst3/param_changes.rs`, `vst3/process.rs`, `vst3/instance.rs`)**: Implemented preallocated `HostParameterChanges` (`IParameterChanges`) and `HostParamQueue` (`IParamValueQueue`) COM objects. Maps plain values to normalized (0..1) via `IEditController::plainParamToNormalized` and passes to `ProcessData.inputParameterChanges`.
- **Curve Compilation & Block Slicing (`render.rs`)**: Slices high-level parameter automation curves from `simply_fugue::composite::CompositeFugue` (`Linear`, `Exp`, `Log`, `None`) into time-stamped `ParamChange` points evaluated across block boundaries during offline rendering.

---

## v0.2.0 — State Tools & AI Sampler

**Tagged `v0.2.0` · 2026-09-01 00:15 · quurmwqx**

#### Generic VST3/CLAP Presets & AI Offline Sampling Test (`GENERIC-PRESETS-TEST`) · 2026-09-01 00:15 · qplzmxnr

**Native VST3 / CLAP program inspection and driver-free AI sampling** (`nexus-host`, `nexus-tools`, `nexus-mcp`).
- **Generic VST3 & CLAP Preset Management (`PluginInstance::presets` / `load_preset`)**: `PluginInstance` now provides generic methods `presets()` and `load_preset()` implemented natively for VST3 via `IUnitInfo` / `IEditController` program lists and parameter mappings, and for CLAP via preset extensions and program changes. Removed all filesystem and vendor-specific preset scanning.
- **DTO Modernization (`nexus-tools::types`)**: Standardized `PresetItem` (`index`, `name`, `category`, `program_list_id`) and `LoadPresetResponse` (`name`, `index`, `loaded`).
- **End-to-End AI Sampling Verified (`sample_vital.rs` / `sample_vital_clap.rs`)**: Verified complete autonomous workflow via `nexus-mcp`: `list_plugins` -> `load_plugin` -> `list_presets` -> parameter inspection -> 5-second offline audio render to a 32-bit float 48 kHz WAV file (`vital_5s_sample.wav`).

#### AI Sampler, offline rendering, Fugue playback & MCP Server (`AI-SAMPLER-MCP`) · 2026-08-31 23:30 · wvluppyw

**Driver-free offline plugin sampling, Droplets Fugue playback, catalog/preset/param tools, and standalone MCP server** (`nexus-host`, `nexus-tools`, `nexus-app`, `nexus-mcp`, `nexus-rest`).
- **Offline Audio Rendering (`nexus-host::render`)**: Driver-free offline rendering engine (`render_offline`) evaluating arbitrary plugins at maximum CPU speed without needing an audio driver or hardware interface. Renders 32-bit float 48 kHz WAV files via `hound` and computes audio metrics (`peak_db`, `rms_db`, `duration_s`, `is_silent`).
- **Droplets Fugue Musical Model (`nexus-host::fugue`)**: Full implementation of Droplets' Fugue sequencing model supporting compact note arrays `[beat, "C3", duration, velocity, channel]`, DAW pitch notation (`C3` = MIDI 60, `C-2` = 0), CC automation, and parameter curves (`linear`, `exp`, `log`, `none`).
- **Tool Surface Expansion (`nexus-tools`)**: Added 10 new high-level tools: `list_plugins`, `load_plugin`, `list_presets`, `load_preset`, `save_preset`, `list_params`, `get_params`, `set_params`, `play` (live playback), and `render_audio` (offline fast sampling).
- **Standalone MCP Server (`crates/nexus-mcp`)**: Model Context Protocol server exposing the full Nexus tool registry to AI coding agents over both stdio (`nexus-mcp --stdio`) and streamable HTTP (`nexus-mcp --port 9997`). Includes a headless runner that scans the plugin catalog and renders audio autonomously.
- **REST Debug UI Upgrades (`nexus-rest`)**: Added quick "Sample C3" and "Play C3" buttons, prefilled examples for all new tools, and an integrated HTML5 `<audio controls autoplay>` player rendering base64 WAVs directly in the browser.

**In-process REST API server and browser-based debug UI** (`crates/nexus-rest`, Feature 3, T13–T14).
- `crates/nexus-rest`: axum HTTP server running on dedicated background thread with its own tokio runtime. Binds loopback-only (dual 127.0.0.1 and `[::1]`) on port 9996 (or `NEXUS_REST_PORT`).
- Endpoints: `GET /tools` (enumerates registered tool schemas), `POST /tools/{name}` (dispatches JSON requests with per-tool timeout classes and maps errors to HTTP status codes).
- `/debug` page: single inlined HTML/JS debug interface with tool list, JSON request editor, collapsible response viewer, quick actions for session info, decode live, snapshot, and diff against live. Captures `base_sha256` into edit examples; `⌘⏎` dispatches.
- Wired into `nexus-app` top-level status bar with active REST endpoint URL.
- End-to-end integration tests over real loopback sockets (`tests/http.rs`).

#### Tools trait core + state/snapshots/codecs (`TOOLS-CORE`, `STATE-CODECS`) · 2026-08-26 08:36 · luuznszn

**Transport-agnostic tool surface and state codec layer** (`crates/nexus-tools`, Features 1 & 2, T1–T12).
- `crates/nexus-tools` scaffolded: `NexusTools` async trait, DTOs with `schemars` JSON schemas, structured `ToolError` codes.
- Main-thread bridge: crossbeam channel + futures oneshot replies marshaling async tool calls onto the host UI thread via `MainThreadHost` trait. Wired into `nexus-app` event loop.
- Hand-written tool registry with schema extraction and dynamic dispatch.
- State operations: `get_state` (size/sha256/base64 cap), `set_state` (auto-snapshotting prior state), in-memory LRU snapshot store + on-disk corpus export (`.../Nexus/state-corpus/<plugin_id>/<name>.state` + JSON sidecar), `diff_state` doc-level structural diff.
- Codec system: `StateCodec` trait + ordered registry. `generic` fallback codec (recursive gzip/zlib decompression, RIFF chunk walker, VST3 chunk-list framing, JSON/XML sniffers, entropy/binary leaf detection) and `vital-json` codec (lossless JSON decode + encode).
- `edit_state`: JSON pointer edits with `base_sha256` guard, dry-run support, auto-snapshotting prior state, and through-the-plugin verification (load → re-save → re-decode → compare).

---

## v0.1.0 — Load & Hear

**Tagged `v0.1.0` · 2026-08-26 07:15 · smpxzzxx**

The initial release of Nexus: a format-agnostic Rust audio plugin hosting engine and standalone host application for macOS.
- **Engine (`nexus-host`)**: Format-agnostic plugin hosting abstracting CLAP (via `clack`) and VST3 (via `vst3` pure-Rust COM bindings). Discovery and scanning across macOS standard paths (`~/Library/Audio/Plug-Ins/{CLAP,VST3}` and `/Library`), caching bundle scan results to avoid startup lag. Audio and MIDI processing engine with lock-free parameter and audio buffers. Preset save/recall (`.nexuspreset`).
- **Host App (`nexus-app`)**: Standalone host application built with `egui` and `eframe` rendered via Metal (`wgpu`) to prevent OpenGL context collisions with plugin editors. Native child window parenting for plugin GUI editors (AppKit/objc2). CPAL audio output device selection, hardware MIDI input (`midir`) + built-in virtual on-screen keyboard, menu bar (`muda`), searchable plugin browser palette (`⌘P`), and plugin scan failure reporting.
- **Packaging (`xtask`)**: Build automation and bundling for CLAP and VST3 plugins (`cargo bundle`, `cargo install-plugin`).

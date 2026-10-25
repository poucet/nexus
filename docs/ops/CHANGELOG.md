# Nexus — CHANGELOG

> What belongs here, and what does not: @AGENTS.md §2. Newest first.

| Dates | Tag | |
|---|---|---|
| 2026-09-01 → … | — | **[Unreleased](#unreleased)** — 0.4 state exploration & AI synthesis |
| 2026-09-01 | `v0.3.0` | [Milestone 0.3: Plugin Wrapper & Automation](#v030--in-daw-wrapper-parameter-automation--cross-platform-linux) — `nexus-wrap` CLAP plugin, realtime parameter ring, Linux/Docker support, AI patch reconstruction design |
| 2026-08-26 → 09-01 | `v0.2.0` | [Milestone 0.2: State Tools & AI Sampler](#v020--state-tools--ai-sampler) — MCP server, driver-free offline rendering, Fugue sequencing, codecs |
| 2026-06-01 → 08-26 | `v0.1.0` | [Milestone 0.1: Load & Hear](#v010--load--hear) — CLAP/VST3 hosting engine + standalone egui app |

---

## Unreleased

#### A fresh clone builds again; `simply-fugue` pinned by tag (`I-98`) · 2026-10-25 21:50 · xuskpswy

- Removed the tracked `[patch]` that pointed `simply-fugue` at a sibling `../music-rs` checkout; `simply-fugue` is now pinned to music-rs tag `v1.0.1`, so `cargo update` no longer moves it with music-rs main. A local override belongs in an untracked cargo config (`I-98`).

Gates: `cargo test --workspace --locked` 109 passed / 0 failed / 11 ignored; `cargo clippy --workspace --all-targets --locked` 24 warnings, none new.

#### Multi-channel CC support verified against shared crate (`I-53`, `I-91`) · 2026-09-06 18:20 · omkzwslr

- **Multi-channel CC verified (`I-53`)**: Verified `render_offline` with `CompositeFugue`'s new `CcTarget` support; CC lanes specified as `"channel:controller"` (e.g. `"2:74"`) deliver MIDI CC messages on the designated channel.
- **Shared crate doc parity (`I-91`)**: Updated `music-rs` doc comments across `lib.rs`, `types.rs`, and `composite.rs` reflecting full CLAP/VST3 per-note expression support in Nexus and native support in Flux.

Gates: `cargo test --workspace` 108 passed / 0 failed / 11 ignored; `cargo clippy -p nexus-host --all-targets` 0 new warnings against baseline.

#### VST3 MIDI controller mapping and per-note expression (`I-97`) · 2026-09-06 12:00 · tppoyvxx

- **Per-note expressions**: `to_vst_event` translates `NoteExpression::{Pressure, Volume, Pan, Tuning, Vibrato, Expression, Brightness}` into VST3 `PolyPressureEvent` (`kPolyPressureEvent`) and `NoteExpressionValueEvent` (`kNoteExpressionValueEvent`), normalizing semitone pitch tuning onto `[0.0, 1.0]` centered at 0.5.
- **Polyphonic key pressure**: raw MIDI `0xA0` translates to `PolyPressureEvent`.
- **MIDI controller mapping**: `query_midi_mapping` iterates over channels `0..16`, standard CCs `0..=127`, `kPitchBend`, and `kAfterTouch` via VST3 `IMidiMapping::getMidiControllerAssignment`, mapping incoming raw MIDI messages into normalized VST3 parameter changes (`HostParameterChanges`).
- **Event support**: `Vst3PluginInstance::event_support()` reports `note_expression: true` and `raw_midi: !midi_map.is_empty()`, allowing `render_offline` to pass through expression lanes and mapped MIDI without spurious warnings.

Gates: `cargo test --workspace` 107 passed / 0 failed / 11 ignored; `cargo clippy -p nexus-host --all-targets` 0 new warnings against baseline.

#### Offline render fixes, automation deduplication & parity (`I-92`–`I-96`) · 2026-09-06 11:38 · wrturnym

- **Arriving-point curve easing (`I-92`)**: `evaluate_curve` reads `k1.interpolation` instead of `k0.interpolation`, matching `flux_contract::Point` / droplets' rule and the shared crate's `PerNotePoint` doc.
- **No double parameter delivery to CLAP plugins (`I-94`)**: `render_offline` delivers `ParamChange` to `param_producer` when available, and only falls back to `block_events` when the instance has no ring buffer, eliminating duplicated `ParamValueEvent`s in CLAP.
- **Param lane held values not repeated (`I-95`)**: `render_offline` tracks `last_param` and suppresses repeated emissions of unchanged values on the 64-frame grid, matching the rule used by CC and expression lanes.
- **60 s duration ceiling (`I-96`)**: offline rendering refuses fugues exceeding 60.0 s (`MAX_DURATION_S`) with `HostError::InvalidRequest`, matching flux's ceiling and guarding against multi-hundred-megabyte WAV allocations.
- **Stale prose updated (`I-93`)**: `play` tool description updated from "Droplets Fugue format" to "Fugue format", `render_audio` docs note the 60 s maximum length, and `instructions.md` explicitly names `file` as the default output format.

Gates: `cargo test --workspace` 104 passed / 0 failed / 11 ignored; `cargo clippy --workspace --all-targets` 0 new warnings against baseline.

#### `render_audio` renders what the fugue says (`FUGUENEXUS-2`) · 2026-09-04 23:19 · vwwnyskw, wzsyxqru, wvyzxrsm, tswpsmmv

The behaviour half of FUGUE-API.md §6's ruling (the design lives in flux, `docs/design/FUGUE-API.md` §6.1, §6.2, §6.6, §6.7), on nexus's side. **Every item below changes what already-valid fugue JSON sounds like** — that is the ruling — and every policy is the same number as flux's `FUGUEONE`, so one fugue measures the same on both hosts. Evidence: @docs/research/render-audio-fixture-table.md, the 12 shared fixtures rendered before and after.

- **`Log` is `1 − (1 − t)²`**, the formula flux and droplets use; nexus computed `√t` and was the outlier. One `curve_factor` serves every interpolating site (flux `I-49`).
- **A segment eases with the arriving point's curve**, not the departing point's: `evaluate_curve` now reads `k1.interpolation` instead of `k0.interpolation`, matching `flux_contract::Point` / droplets' rule and the shared crate's `PerNotePoint` doc (flux `I-92`).
- **`Cc.interpolation` is honoured**: a CC lane with a curve is a ramp of bytes on the 64-frame grid `params` lanes use, rounded, held values not re-sent; a curve-less lane is still exactly one byte per point, so no existing fugue gains an event (flux `I-50`).
- **The five kinds `_ => {}` swallowed are rendered**: channel pitch bend, pressure and poly pressure as raw MIDI; per-note pitch bend and pressure as CLAP note expressions (`Tuning` in semitones, `Pressure` 0..1), interpolated on the same grid. The match is exhaustive now. `Event::NoteExpression` reached nothing before — the clack conversion had no arm for it and has one (flux `I-48`).
- **A fixed 1.0 s release tail**, once, after the last pass; `frames` and `duration_s` include it (flux `I-51`).
- **`loop_mode` offline**: `once` is one pass, `{"times": n}` is `n` passes (`0` counts as one, as on flux) with every event re-emitted a pass later; `loop` / `ping_pong` are refused with `invalid_input` naming live playback (flux `I-52`).
- **Note-off before note-on at the same sample**, in nexus's own sort, whatever order `compile()` delivered (flux `I-54`).
- **`output_format` is the shared `OutputFormat` enum and defaults to `file`** (`simply-fugue` moved to `bd774ec2`; nothing else in the lock moved); an unknown spelling is refused naming both values instead of falling through to base64. The REST debug UI asks for `wav_base64` by name, so it is unaffected.
- **`warnings` is filled**: an assumed tempo, a floored `block_size`, the tail, `times: 0`, and any event kind the plugin path cannot carry, with its count — VST3 takes note on/off only and says so through the new `PluginInstance::event_support`.
- **Fixed in passing**: a block's event buffer was not in time order — every note went in ahead of every parameter change whatever their frames said, and CLAP specifies that list as time-ordered. One sort.
- **One restructuring**: the three copy-pasted `render_audio` host impls share `nexus_tools::render` (options, file/base64, response, error mapping); each keeps only how it obtains an instance. The tool description states the policies that have no field — the tail, the loop semantics, the `file` default, the curve set, and §6.7's ruling that CC bytes 0–127 *are* the travel space.
- `docs/design/AI_SAMPLER_MCP.md` §1 corrected to the real request and response (flux `I-47`).

Gates: `cargo test --workspace` 101 passed / 0 failed / 11 ignored (was 84 / 0 / 11); `cargo clippy --workspace --all-targets` 24 warnings against the 25 baseline, none new.

#### `simply-fugue` consumed via git dependency (`poucet/music-rs`) · 2026-09-04 18:09 · ulrvzslx

Switched `simply-fugue` from a local path dependency to `https://github.com/poucet/music-rs.git`.

- `Cargo.toml` now references `{ git = "https://github.com/poucet/music-rs.git" }`.
- External clones no longer require `music` checked out as a sibling at `../music`.
- Container builds (`Dockerfile`) build cleanly from the repo root without sibling context mounting.
- Local cross-repo development can be overridden using Cargo's `[patch]` in `.cargo/config.toml`.

#### `simply-fugue` moved to the shared `music` repo (`FUGUENEXUS`) · 2026-09-03 22:25 · qpnkxvoo

The fugue wire format is now shared with Flux, so it lives in a repo neither
owns: `~/projects/simply/music`. Nexus consumes it as a path dependency and no
longer vendors it.

- **`crates/simply-fugue` deleted here**, and `Cargo.toml:19` now reads
  `path = "../music/crates/simply-fugue"` — the only place that repo's name
  appears. `nexus-host` and `nexus-mcp` keep `simply-fugue.workspace = true`,
  and `nexus_host::fugue` (`nexus-host/src/lib.rs:18`) still re-exports it, so
  no use site moved.
- **`RenderAudioRequest` / `RenderAudioResponse` moved into the crate** and
  `nexus-tools/src/types.rs` re-exports them, so nexus and flux share the type
  instead of keeping two copies in step by hand. The response gained
  `warnings`, `rms` and `peak_amplitude`; all three are skipped when unused and
  **nexus fills none of them**, so its serialized response is byte-identical.
  The three constructors (`nexus-app/src/tools_host.rs`,
  `nexus-wrap/src/bridge.rs`, `nexus-mcp/src/standalone.rs`) take them with
  `..Default::default()` and are otherwise unchanged.
- **No behaviour change.** `render.rs` is untouched — the `Log` formula, the
  discarded `Cc.interpolation`, the missing note tie-break and the missing
  release tail are all still exactly as they were, pending a ruling.

`cargo test --workspace`: 84 passed, 0 failed, 11 ignored — 87 before, less the
three `simply-fugue` unit tests that went with the crate (the suite there is now
24). `cargo clippy --workspace --all-targets`: 25 warnings, 27 before, less the
two in `simply-fugue/src/note.rs` that left with it. Both examples still build.

**Building nexus now requires `~/projects/simply/music` checked out beside it**,
and a lane workspace needs the gitignored symlink
`ln -sfn ~/projects/simply/music .workspaces/music` — see `music/AGENTS.md`.
The `Dockerfile`'s build context does not yet reach the sibling; `docker build`
is expected to fail until it is re-rooted.

---

## v0.3.0 — In-DAW Wrapper, Parameter Automation & Cross-Platform Linux

**Tagged `v0.3.0` · 2026-09-01 01:20**

#### `nexus-wrap` Plugin Wrapper & In-DAW Loopback Bridge (`NEXUS-WRAP`) · 2026-09-01 00:37 · koyzxqvo

**CLAP plugin wrapper, inner plugin hosting bridge, and live in-DAW loopback MCP/REST server** (`nexus-wrap`).
- **CLAP Plugin Interface (`crates/nexus-wrap`)**: Implemented `NexusWrapPlugin` with `clack-plugin`, declaring `PluginAudioPorts` (stereo in/out), `PluginNotePorts` (CLAP + MIDI note dialects), `PluginParams` (parameter reflection and bidirectional DAW automation), and `PluginState` (state persistence).
- **Inner Plugin Hosting Bridge (`nexus-wrap::bridge`)**: Wraps and executes any inner CLAP/VST3 plugin using `nexus-host`, routing audio/MIDI processing in a realtime-safe `ProcessBlock` and exposing inner state/presets/parameters to `nexus-tools` via `MainThreadHost`.
- **In-DAW Loopback Servers (`nexus-wrap::server`)**: Spawns background worker thread starting loopback REST server on port 9996 (`http://127.0.0.1:9996/debug`) and MCP server on port 9997 (`http://127.0.0.1:9997/mcp`) so external AI agents can connect and interact with active DAW sessions.
- **DAW State Envelope (`nexus-wrap::state`)**: Implemented `WrapStateEnvelope` preserving inner plugin identity and raw binary state blobs across DAW project saves and reloads.

#### Linux Cross-Platform Support & Docker Setup (`LINUX-DOCKER`) · 2026-09-01 00:35 · wnlyruwx

**Standard Linux plugin search paths, POSIX dynamic module loader, and containerized CI/agent environment** (`nexus-host`, `nexus-mcp`).
- **Linux Discovery Paths (`nexus-host::discovery::paths`)**: Added standard Linux paths for CLAP (`~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`) and VST3 (`~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`).
- **POSIX VST3 Module Loader (`vst3/module/linux.rs`)**: Implemented `dlopen`/`dlsym` dynamic module loader locating Linux `.so` binaries under `Contents/x86_64-linux/` or bundle roots and binding `GetPluginFactory`, `ModuleEntry`, and `ModuleExit`.
- **Target Compilation Gating**: Cleanly gated macOS-specific AppKit window parenting behind `#[cfg(target_os = "macos")]`.
- **Docker Setup & Documentation**: Created multi-stage `Dockerfile` and `docker-compose.yml` for headless container testing and running `nexus-mcp --port 9997`, with comprehensive documentation in [`docs/guides/LINUX.md`](docs/guides/LINUX.md).

#### Realtime Parameter Automation Ring (`PARAM-AUTOMATION`) · 2026-09-01 00:30 · nprppvmq

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

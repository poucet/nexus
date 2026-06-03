# Nexus 0.1 — TASKS

Active task breakdown for the **Load & Hear** MVP. See [ROADMAP.md](ROADMAP.md)
for the feature list and [../../designs/ARCHITECTURE.md](../../designs/ARCHITECTURE.md)
for the design rationale.

All tasks are Phase 01 unless noted. IDs are `T{N}`. Status: ⬜ todo / 🔄 in
progress / ✅ done.

---

## Feature 1: Plugin discovery & catalog

Scan the macOS folders Bitwig/Ableton use and produce a list of installed
plugins with enough metadata (name, vendor, format, path) to display and load.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T1 | Define macOS plugin search paths for CLAP + VST3 (user `~/Library/Audio/Plug-Ins/{CLAP,VST3}` and system `/Library/Audio/Plug-Ins/{CLAP,VST3}`) | `crates/nexus-host/src/discovery/paths.rs` — `search_paths(format)` + pure `search_paths_in(home, format)`; user scope before system. 3 unit tests. |
| ✅ | T2 | Filesystem scan → `Vec<(PluginFormat, bundle_path)>`; CLAP = `*.clap` bundles, VST3 = `*.vst3` bundles | `crates/nexus-host/src/discovery/scan.rs` — `scan(format)` + `scan_all()`; recurses vendor folders (depth-capped) but never descends into a matched bundle; missing dirs non-fatal. Used `std::fs` recursion, not `walkdir`. 5 tests. |
| ✅ | T2.5 | Config file with custom CLAP/VST3 plugin locations that **extend** the default search paths | `crates/nexus-host/src/discovery/config.rs` — TOML at `~/Library/Application Support/Nexus/config.toml` (override via `NEXUS_CONFIG`); `[plugins] clap=[...] vst3=[...]`; `~` expansion; missing/invalid non-fatal. `paths::search_paths` merges defaults + config, deduped; `scan` picks them up automatically. 6 tests. |
| ✅ | T3 | `PluginCatalog` + `PluginRef { format, path, descriptor{ id, name, vendor } }`; assemble by probing each bundle's factory (delegates to CLAP probe T7 / VST3 probe T11) | `crates/nexus-host/src/discovery/catalog.rs` (re-exported from `lib.rs`) — `PluginCatalog::scan()` over `scan_all` + per-format probe; `display_name()` falls back to id; failures logged + skipped. Verified: assembles 24 CLAP plugins (VST3 skipped until T11). |

## Feature 2: Format-agnostic host core

The seam every format and the UI sit behind. Extend the existing `Host`/
`HostError`/`PluginFormat` stub in `lib.rs`.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T4 | `PluginInstance` trait (activate, start/stop processing, process, editor handle) + `Host::load(format, path) -> Box<dyn PluginInstance>` dispatching on format | `crates/nexus-host/src/host.rs` — moved `PluginFormat`/`HostError`/`Host` here (re-exported from `lib.rs`); added `PluginInstance` + `Editor` traits, `AudioConfig`, and `Host::load` dispatch (backends return `NotImplemented` until T8/T12). Used `Host::load` not `PluginRef::load` (PluginRef is T3). 2 tests. **Refactored (2026-06-03):** split into main-thread `PluginInstance` (activate/deactivate/editor, not `Send`) + `activate() -> Box<dyn AudioProcessor>` where `AudioProcessor: Send` owns start/stop/process — matches clack's & VST3's main/audio thread split. |
| ✅ | T5 | Shared audio/event buffer types: interleaved/planar audio buffers, note/MIDI event list passed to `process` | `crates/nexus-host/src/process.rs` — `AudioBuffer` (planar, reuse via `clear`/`set_frames`), `EventBuffer`, `ProcessBlock`. Alloc-free per block. **Revised 2026-06-03:** `Event` is now **CLAP-faithful** (note on/off/choke/end, note-expression, param value/mod/gesture, transport, MIDI/MIDI2/sysex) so wrapping CLAP-in-CLAP is lossless; VST3 maps onto it (known gaps in [PLUGIN_EXTENSIONS.md](../../designs/PLUGIN_EXTENSIONS.md)). 5 tests. |

## Feature 3: CLAP hosting (via `clack`) — full host-side extension parity

CLAP first — it unblocks the engine (F5) and UI (F6) plumbing that VST3 reuses,
and it is the **reference surface** for every plugin aspect. The host implements
the host side of every standard CLAP extension; each aspect adds a
format-agnostic capability to `PluginInstance` (T4) that the egui app and, later,
`nexus-wrap` consume. See [PLUGIN_EXTENSIONS.md](../../designs/PLUGIN_EXTENSIONS.md).

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T6 | Add `clack-host` + `clack-extensions` (workspace deps); host infrastructure extensions: `log`, `thread-check`, `timer-support` (+ stubs for `thread-pool`, `event-registry`) | `crates/nexus-host/src/clap/{mod,host}.rs` — `NexusHost` handlers (shared/main-thread), `host_info()`, main-thread message channel, `log` extension registered + forwarded to the `log` crate. clack tracks `main` with `all-extensions` enabled (lockfile pins the commit). thread-check is provided by clack automatically; `timer`/`gui` registration deferred to T29. |
| ✅ | T7 | CLAP factory probe: dlopen bundle, read `clap_plugin_factory` descriptors → `(id, name, vendor)` for the catalog (T3) | `crates/nexus-host/src/clap/probe.rs` — `probe(path) -> Vec<PluginDescriptor>` (id/name/vendor/version); owned `PluginDescriptor` added to `host.rs`. Verified against real install: read 24 CLAP plugins (FabFilter, Surge XT, TAL, VCV Rack). Ignored test `probe_installed_clap_plugins` for manual runs. |
| ✅ | T8 | Instantiate + activate behind `PluginInstance` (T4); negotiate sample-rate/block-size; `render` mode | `crates/nexus-host/src/clap/instance.rs` — `ClapPluginInstance` (impls `PluginInstance`) loads+instantiates via clack; `activate()` → `ClapAudioProcessor` (impls `Send` `AudioProcessor`, start/stop via clack's `PluginAudioProcessor` enum). `Host::load` now takes `&PluginRef` and dispatches CLAP. **Verified on a real plugin** (Droplets): load→activate→start→stop→deactivate. process() body is T9. |
| ✅ | T9 | Audio + Note I/O: `audio-ports` + `audio-ports-config` layout, `note-ports`, transport; `process()` maps F5 buffers/events to `clap_process` | `clap/ports.rs` (`IoLayout::query` reads audio/note port channel counts) + `clap/process.rs` (`ClapAudioProcessor`: reusable clack `AudioPorts` buffers, façade `Event`→clack note/MIDI events, `process()` runs the plugin and copies the main port back). **Verified: Surge XT renders non-silent audio from a note-on** (headless). Param/expression/transport/sysex event mapping deferred to T26/nexus-wrap (engine only emits Note+Midi). |
| ✅ | T26 | Params: enumerate, get/set, value↔text, flush; handle host rescan/clear/request-flush | `crates/nexus-host/src/clap/params.rs` — read side: `enumerate`/`value`/`value_to_text` via `PluginParams`; façade `ParamInfo` (id/name/module/range/flags) + 3 `PluginInstance` methods. **Verified on Surge XT** (params enumerated, value_to_text works). Host rescan/clear already handled by `HostParamsImpl` (T6). Host-initiated *set* travels as `Event::ParamValue` — wired with the engine / nexus-wrap. |
| ✅ | T27 | State: `state` save/load + `state-context` | `crates/nexus-host/src/clap/state.rs` — `save`/`load` via `PluginState` (opaque chunk through `Vec`/`Cursor`); `save_state`/`load_state` on `PluginInstance`. **Verified: Surge XT state round-trips.** `state-context` deferred to Phase 02. |
| ✅ | T28 | Latency, tail, voice-info, note-name reporting | `crates/nexus-host/src/clap/info.rs` — `latency()` + `voice_info()` on `PluginInstance`, façade `VoiceInfo`. **Verified on Surge XT.** `tail` deferred (CLAP queries it on the audio handle, belongs with the audio half); `note-name` (drum maps) deferred to Phase 02. |
| ⬜ | T29 | Standalone `track-info` provider (host → plugin) + wire the GUI extension host callbacks (resize/show/hide/closed) to the editor window (T20–T22) | `crates/nexus-host/src/clap/host.rs`, `clap/gui.rs`. |

> Long-tail CLAP extensions (surround/ambisonic, posix-fd, full thread-pool,
> param-indication, remote-controls, preset-load, context-menu) are Phase 02
> (ROADMAP #12); they're stubbed here so plugins still load.

## Feature 4: VST3 hosting (via Rust SDK bindings)

> **Sequencing (per direction 2026-06-03): all CLAP before VST3.** Do not start
> Feature 4 (or the VST3 editor T22 / VST3 state T30) until the full CLAP
> vertical slice works end to end: CLAP host parity (T6–T9, T26–T29) → engine
> (T14–T16) → app shell (T17–T19) → CLAP editor embedding (T20–T21) → CLAP
> integration (T23). VST3 then brings the second format up to the same surface.

Riskiest piece. COM-style lifetimes; reuses the F2 abstraction and F5 engine.
Phase 01 brings VST3 to **load / process / params / state / GUI**, mapping onto
the same `PluginInstance` surface as CLAP. The long tail of VST3-host parity
(units, note-expression, param-indication, …) is Phase 02 (ROADMAP #11).

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T10 | Add a VST3 binding crate (`vst3`/`vst3-sys`); load a `.vst3` module, get `IPluginFactory`, implement a minimal `IHostApplication` | new `crates/nexus-host/src/vst3/module.rs`, `vst3/host.rs`. Needs the Steinberg VST3 SDK. |
| ⬜ | T11 | VST3 factory probe: enumerate `PClassInfo` → `(id, name, vendor)` for the catalog (T3) | new `crates/nexus-host/src/vst3/probe.rs`. |
| ⬜ | T12 | Instantiate `IComponent` + `IAudioProcessor` + `IEditController`; connect components, set bus arrangements, `setupProcessing`, activate | new `crates/nexus-host/src/vst3/instance.rs`. Behind `PluginInstance` (T4). |
| ⬜ | T13 | VST3 `process()`: fill `ProcessData` audio buses + event input list from F5; map note/MIDI events; `IEditController` params | new `crates/nexus-host/src/vst3/process.rs`, `vst3/params.rs`. |
| ⬜ | T30 | VST3 state: `IComponent` get/setState (+ `IEditController` state) behind the trait's `save_state`/`load_state` | new `crates/nexus-host/src/vst3/state.rs`. |

## Feature 5: Realtime engine (in `nexus-app`)

Turns "loaded" into "audible." **Lives in `nexus-app`, not `nexus-host`** —
`nexus-host` exposes the `AudioProcessor`; the app owns the audio/MIDI hardware
and the realtime callback that drives it. (Created as part of T17.)

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T14 | CPAL output stream; realtime callback pulls audio from the active `AudioProcessor` and writes the chosen output device | new `crates/nexus-app/src/engine/audio.rs` (`cpal`). |
| ⬜ | T15 | MIDI input via `midir` → lock-free queue (`rtrb`) → `Event`s consumed in the audio callback | new `crates/nexus-app/src/engine/midi.rs`. |
| ⬜ | T16 | Engine lifecycle: start/stop, swap active instance, negotiate sample-rate/block-size against the chosen device, hand them to `AudioProcessor` | new `crates/nexus-app/src/engine/mod.rs`. |

## Feature 6: `nexus-app` egui shell

The visible deliverable: a window that browses the catalog, loads a plugin, and
lets you pick audio/MIDI devices.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T17 | Create `crates/nexus-app` (`eframe`/`egui`), app state, main window; pull in `cpal`/`midir` (Feature 5 lives here); add to workspace members | new crate; update root `Cargo.toml` members + workspace deps. |
| ⬜ | T18 | Embed `nexus-host`'s wrapper UI (selector + presets + editor frame, T33) in the app window; the app supplies the window handle + the catalog | `crates/nexus-app/src/app.rs`. Replaces a bespoke browser — the chrome is shared with `nexus-wrap`. |
| ⬜ | T19 | Wire app → `nexus-host`: load selected plugin, start the engine, show status (loaded / playing) | `crates/nexus-app/src/app.rs`. |
| ⬜ | T31 | **Device-selection UI**: dropdowns for audio output device (and input), sample rate/buffer size, and MIDI input source; apply selection to the engine (restart stream on change) | new `crates/nexus-app/src/devices.rs`. Enumerate via `cpal`/`midir`; persist last choice via the config file (T25). |

## Feature 7: Plugin editor embedding

"Show the actual plugin." Native child window parented into the egui window;
both CLAP and VST3 views attach into it.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T20 | Native child window (macOS NSView) parented to the egui window via `raw-window-handle`; reserve a rect, handle move/resize | new `crates/nexus-app/src/editor_window.rs`. The reusable embedding primitive (extract to its own crate when the web DAW needs it). |
| ⬜ | T21 | CLAP GUI extension: `create` / `set_parent` / `set_scale` / `show`, size negotiation into the child window | new `crates/nexus-host/src/clap/gui.rs`. |
| ⬜ | T22 | VST3 `IPlugView`: `attached(parent, "NSView")`, `getSize`/`onSize`, `IPlugFrame::resizeView` | new `crates/nexus-host/src/vst3/editor.rs`. |

## Feature 8: End-to-end integration

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T23 | Full flow on macOS with real plugins: scan → browse → load (one CLAP **and** one VST3) → play via MIDI keyboard → editor opens and is interactive. Capture findings; note any plugin that misbehaves | manual verification; update `README.md` quickstart. |
| ⬜ | T24 | Error surfacing: scan/probe/load failures show in the UI and are skipped, never crash the app | `crates/nexus-app` + `nexus-host` error types. |

## Feature 15: Wrapper UI + presets (`nexus-host`, `ui` feature)

The shared host chrome around a hosted plugin — plugin selector, presets, and
the inner-editor frame — behind an opt-in egui `ui` feature, so `nexus-app` and
(later) `nexus-wrap` reuse it while the bare library and a web DAW omit egui.
See [ARCHITECTURE.md](../../designs/ARCHITECTURE.md).

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T32 | Preset model: `Preset { format, plugin_id, path, state: Vec<u8> }` saved/loaded to a `.nexuspreset` file (serde, no UI); recall re-loads the plugin via `Host::load` and restores state (T27) | `crates/nexus-host/src/preset.rs` — `Preset` (state base64 in JSON); `capture`/`save_to_file`/`load_from_file`/`recall`. `PluginFormat` is now serde. **Verified: Surge XT captured → file → recalled.** |
| ✅ | T33 | egui wrapper UI behind a `ui` feature: plugin selector (from `PluginCatalog`), preset save/recall controls, inner-editor frame (reserve a rect, parent the plugin editor via the GUI ext into a host-supplied window handle) | `crates/nexus-host/src/ui/` — `WrapperUi::show()` draws selector + preset buttons + reserves the editor rect, returns `WrapperUiActions` for the app to act on. `ui` feature → optional `egui` 0.31 (0.34 needs rustc 1.92; we're on 1.90). Default build stays egui-free. Headless egui test verifies it draws. **Editor-rect *parenting* deferred to T20/T21** (needs native window + GUI ext). |

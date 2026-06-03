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
| ⬜ | T3 | `PluginCatalog` + `PluginRef { format, path, descriptor{ id, name, vendor } }`; assemble by probing each bundle's factory (delegates to CLAP probe T7 / VST3 probe T11) | new `crates/nexus-host/src/discovery/catalog.rs`; re-export from `lib.rs`. Probe failures are logged + skipped, never fatal (folders contain junk). |

## Feature 2: Format-agnostic host core

The seam every format and the UI sit behind. Extend the existing `Host`/
`HostError`/`PluginFormat` stub in `lib.rs`.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T4 | `PluginInstance` trait (activate, start/stop processing, process, editor handle) + `Host::load(format, path) -> Box<dyn PluginInstance>` dispatching on format | `crates/nexus-host/src/host.rs` — moved `PluginFormat`/`HostError`/`Host` here (re-exported from `lib.rs`); added `PluginInstance` + `Editor` traits, `AudioConfig`, and `Host::load` dispatch (backends return `NotImplemented` until T8/T12). Used `Host::load` not `PluginRef::load` (PluginRef is T3). 2 tests. |
| ✅ | T5 | Shared audio/event buffer types: interleaved/planar audio buffers, note/MIDI event list passed to `process` | `crates/nexus-host/src/process.rs` — `AudioBuffer` (planar, reuse via `clear`/`set_frames`), `Event` (NoteOn/Off + raw `Midi`), `EventBuffer`, `ProcessBlock`. Alloc-free per block. 5 tests. |

## Feature 3: CLAP hosting (via `clack`) — full host-side extension parity

CLAP first — it unblocks the engine (F5) and UI (F6) plumbing that VST3 reuses,
and it is the **reference surface** for every plugin aspect. The host implements
the host side of every standard CLAP extension; each aspect adds a
format-agnostic capability to `PluginInstance` (T4) that the egui app and, later,
`nexus-wrap` consume. See [PLUGIN_EXTENSIONS.md](../../designs/PLUGIN_EXTENSIONS.md).

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T6 | Add `clack-host` + `clack-extensions` (workspace deps); host infrastructure extensions: `log`, `thread-check`, `timer-support` (+ stubs for `thread-pool`, `event-registry`) | `crates/nexus-host/src/clap/{mod,host}.rs` — `NexusHost` handlers (shared/main-thread), `host_info()`, main-thread message channel, `log` extension registered + forwarded to the `log` crate. clack tracks `main` with `all-extensions` enabled (lockfile pins the commit). thread-check is provided by clack automatically; `timer`/`gui` registration deferred to T29. |
| 🔄 | T7 | CLAP factory probe: dlopen bundle, read `clap_plugin_factory` descriptors → `(id, name, vendor)` for the catalog (T3) | new `crates/nexus-host/src/clap/probe.rs`. Cheap; no instantiation. |
| ⬜ | T8 | Instantiate + activate behind `PluginInstance` (T4); negotiate sample-rate/block-size; `render` mode | new `crates/nexus-host/src/clap/instance.rs`. |
| ⬜ | T9 | Audio + Note I/O: `audio-ports` + `audio-ports-config` layout, `note-ports`, transport; `process()` maps F5 buffers/events to `clap_process` | new `crates/nexus-host/src/clap/process.rs`, `clap/ports.rs`. Adds `audio_ports()`/`note_ports()` to the trait. |
| ⬜ | T26 | Params: enumerate, get/set, value↔text, flush; handle host rescan/clear/request-flush | new `crates/nexus-host/src/clap/params.rs`. Adds param methods to `PluginInstance`. |
| ⬜ | T27 | State: `state` save/load + `state-context` | new `crates/nexus-host/src/clap/state.rs`. Adds `save_state`/`load_state` to the trait. |
| ⬜ | T28 | Latency, tail, voice-info, note-name reporting | new `crates/nexus-host/src/clap/info.rs`. Adds `latency()`/`tail()`/`voice_info()`/note-name. |
| ⬜ | T29 | Standalone `track-info` provider (host → plugin) + wire the GUI extension host callbacks (resize/show/hide/closed) to the editor window (T20–T22) | `crates/nexus-host/src/clap/host.rs`, `clap/gui.rs`. |

> Long-tail CLAP extensions (surround/ambisonic, posix-fd, full thread-pool,
> param-indication, remote-controls, preset-load, context-menu) are Phase 02
> (ROADMAP #12); they're stubbed here so plugins still load.

## Feature 4: VST3 hosting (via Rust SDK bindings)

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

## Feature 5: Audio + MIDI engine

Turns "loaded" into "audible." Owns the realtime callback; drives the active
instance regardless of format.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T14 | CPAL output stream; realtime callback pulls audio from the active `PluginInstance` | new `crates/nexus-host/src/engine/audio.rs` (`cpal`). |
| ⬜ | T15 | MIDI input via `midir` → lock-free queue (`rtrb`) → events consumed in the audio callback as note on/off | new `crates/nexus-host/src/engine/midi.rs`. |
| ⬜ | T16 | Engine lifecycle: start/stop, swap active instance, expose sample-rate/block-size to instances | new `crates/nexus-host/src/engine/mod.rs`. |

## Feature 6: egui host app shell (`nexus-app`)

The visible deliverable: a window that browses the catalog and loads a plugin.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T17 | Create `crates/nexus-app` (`eframe`/`egui`), app state, main window; add to workspace members | new crate; update root `Cargo.toml` members + workspace deps. |
| ⬜ | T18 | Plugin browser panel: list catalog (name / vendor / format), text filter, "Load" action | new `crates/nexus-app/src/browser.rs`. |
| ⬜ | T19 | Wire app → `nexus-host`: load selected plugin, start the engine, show status (loaded / playing) | `crates/nexus-app/src/app.rs`. |

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

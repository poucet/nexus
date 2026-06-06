# Nexus 0.1 Roadmap

## Summary

Nexus 0.1 is the **Load & Hear** MVP: a simple **egui** desktop app
(`nexus-app`) that scans the macOS plugin folders Bitwig/Ableton use, lists
every **CLAP and VST3** found, and lets you load one, **hear it** (CPAL audio
out, MIDI in), and **see its editor** embedded as a native child window.

Scope decisions (2026-06-01): UI is egui now, DAW UI deferred; "load" includes
audio so the plugin makes sound; **both** CLAP and VST3 instantiate in the MVP;
macOS only. Rationale in [designs/ARCHITECTURE.md](../../designs/ARCHITECTURE.md).

The two hard, schedule-driving items are **VST3 hosting** (Rust SDK bindings,
COM lifetime wiring) and **editor embedding** (native child window). CLAP is
sequenced first because it unblocks the engine and UI plumbing that VST3 reuses.

---

## Feature Overview

### Phase 01: MVP — Load & Hear (macOS, egui, CLAP + VST3 + audio)

| Done | Pri | # | Feature | Complexity | Impact |
|------|-----|---|---------|------------|--------|
| [ ] | P0 | 1 | Plugin discovery & catalog (scan macOS CLAP/VST3 dirs → descriptors) | M | High — nothing loads without a list of what's installed |
| [ ] | P0 | 2 | Format-agnostic host core (traits, audio/event buffer types) | M | High — the seam every format and UI sits behind |
| [ ] | P0 | 3 | CLAP hosting via `clack` — **full host-side extension parity** (audio/note I/O, params, state, latency/tail/voice-info, GUI, host infra) | L | High — fastest path to a plugin that loads, and the reference surface for every aspect ([PLUGIN_EXTENSIONS.md](../../designs/PLUGIN_EXTENSIONS.md)) |
| [ ] | P0 | 4 | VST3 hosting via Rust SDK bindings (module, instantiate, process, state, GUI) | L | High — second format; the riskiest piece. Full VST3-host parity is Phase 02 |
| [ ] | P0 | 5 | Realtime engine in **`nexus-app`** (CPAL out, midir in, callback driving the host's `AudioProcessor`) | M | High — turns "loaded" into "audible"; lives in the app, not the host library |
| [ ] | P0 | 6 | `nexus-app` egui shell: plugin browser + load + **device-selection UI** (audio in/out devices, MIDI sources) | M | High — the visible deliverable |
| [ ] | P0 | 7 | Plugin editor embedding (native child window; CLAP + VST3 views) | L | High — "show the actual plugin" — the other risky piece |
| [ ] | P0 | 8 | End-to-end integration + macOS verification with real plugins | M | High — proves the whole chain on real CLAP/VST3 |
| [ ] | P1 | 9 | Error surfacing in UI (scan/load failures don't crash the app) | S | Medium — real plugin folders contain broken/old binaries |
| [ ] | P0 | 15 | **Wrapper UI + presets** in `nexus-host` (`ui` feature): plugin selector, preset save/recall (identity + state), inner-editor frame — reused by `nexus-app` now and `nexus-wrap` later | L | High — shared host chrome; `nexus-wrap` inherits it for free |

### Phase 02: Beyond the MVP (not scheduled yet)

| Done | Pri | # | Feature | Complexity | Impact |
|------|-----|---|---------|------------|--------|
| [ ] | P2 | 10 | Cross-platform discovery (Windows + Linux search paths) | M | Medium — MVP is macOS-only by choice |
| [ ] | P2 | 11 | Full **VST3-host** extension parity (params, latency, units, note-expression, …) | L | Medium — bring VST3 hosting up to the CLAP surface |
| [ ] | P2 | 12 | CLAP host long-tail extensions (surround/ambisonic, posix-fd, thread-pool, event-registry, param-indication, remote-controls, preset-load, context-menu) | M | Low–Medium — niche; stubbed in Phase 01, completed here |
| [ ] | P1 | 13 | `nexus-wrap`: expose the host as a CLAP/VST3 plugin, **forwarding the full extension surface** to the inner plugin (VST3 via `clap-wrapper`) | XL | High — layer 2; consumes the same `PluginInstance` surface ([PLUGIN_EXTENSIONS.md](../../designs/PLUGIN_EXTENSIONS.md)) |
| [ ] | P3 | 14 | DAW UI spike (web frontend over the engine; editors as floating windows) | L | High — informs the eventual `nexus-daw` UI choice |
| [ ] | P2 | 16 | Native menu bar via **`muda`** (portable: macOS top-bar, Win/Linux): plugin list, presets, audio-output + MIDI-input device selection. Keeps eframe/egui. | M | Medium — Mac-native feel without losing portability |
| [ ] | P2 | 17 | **Plugin-state explorer**: capture multiple state snapshots, diff their binary layout to map bytes ↔ parameters, working toward driving a plugin's state over **MCP** (AI control). The egui main window's purpose. | L | High — core to the AI-first goal; complements param-based control |

---

## Notes

- **Crate additions in 0.1:** `nexus-app` (egui host binary). Heavy work lands in
  `nexus-host`. `nexus-wrap`/`nexus-daw` stay stubs until Phase 02.
- **New dependencies (planned):** `clack-host`/`clack-extensions` (CLAP host),
  a VST3 SDK binding crate (`vst3` / `vst3-sys`), `cpal` (audio), `midir` (MIDI),
  `eframe`/`egui`, `raw-window-handle`, `walkdir`.
- **VST3 SDK:** hosting needs the Steinberg SDK headers/bindings; `clap-wrapper`
  already expects `CLAP_WRAPPER_VST3_SDK` for the export side.

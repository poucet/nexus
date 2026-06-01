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
| [ ] | P0 | 3 | CLAP hosting via `clack` (probe, instantiate, process) | M | High — fastest path to a plugin that actually loads |
| [ ] | P0 | 4 | VST3 hosting via Rust SDK bindings (module, instantiate, process) | L | High — second format; the riskiest piece of the MVP |
| [ ] | P0 | 5 | Audio + MIDI engine (CPAL out, midir in, realtime callback) | M | High — turns "loaded" into "audible" |
| [ ] | P0 | 6 | egui host app shell `nexus-app` (window, plugin browser, load) | M | High — the visible deliverable |
| [ ] | P0 | 7 | Plugin editor embedding (native child window; CLAP + VST3 views) | L | High — "show the actual plugin" — the other risky piece |
| [ ] | P0 | 8 | End-to-end integration + macOS verification with real plugins | M | High — proves the whole chain on real CLAP/VST3 |
| [ ] | P1 | 9 | Error surfacing in UI (scan/load failures don't crash the app) | S | Medium — real plugin folders contain broken/old binaries |

### Phase 02: Beyond the MVP (not scheduled yet)

| Done | Pri | # | Feature | Complexity | Impact |
|------|-----|---|---------|------------|--------|
| [ ] | P2 | 10 | Cross-platform discovery (Windows + Linux search paths) | M | Medium — MVP is macOS-only by choice |
| [ ] | P2 | 11 | Parameter list + host-side automation/state save-load | L | Medium — toward a real host, and reused by the wrapper |
| [ ] | P2 | 12 | `nexus-wrap`: expose the host as a CLAP/VST3 plugin (wrap a plugin) | XL | High — layer 2 of the project |
| [ ] | P3 | 13 | DAW UI spike (web frontend over the engine; editors as floating windows) | L | High — informs the eventual `nexus-daw` UI choice |

---

## Notes

- **Crate additions in 0.1:** `nexus-app` (egui host binary). Heavy work lands in
  `nexus-host`. `nexus-wrap`/`nexus-daw` stay stubs until Phase 02.
- **New dependencies (planned):** `clack-host`/`clack-extensions` (CLAP host),
  a VST3 SDK binding crate (`vst3` / `vst3-sys`), `cpal` (audio), `midir` (MIDI),
  `eframe`/`egui`, `raw-window-handle`, `walkdir`.
- **VST3 SDK:** hosting needs the Steinberg SDK headers/bindings; `clap-wrapper`
  already expects `CLAP_WRAPPER_VST3_SDK` for the export side.

---
project: nexus
version: "0.1"
phase: "01-mvp"
architecture_updated: 2026-06-01
---

# Nexus

**Problem:** There is no Rust-native, AI-first environment for hosting VST3/CLAP
plugins. Nexus is built in layers so each layer is independently useful: a
format-agnostic hosting **engine**, a **plugin that wraps another plugin** (load
any wrapped plugin inside a DAW), and ultimately an **AI-first DAW**.

**Current focus:** Version 0.1, Phase 01 (MVP) — *Load & Hear*. Ship a simple
**egui** desktop app that scans the macOS plugin folders Bitwig/Ableton use,
lists every CLAP and VST3 found, and lets you load one, **hear it** (CPAL audio
out + MIDI in), and **see its editor** embedded as a native child window.

**Key decisions (see [designs/ARCHITECTURE.md](designs/ARCHITECTURE.md)):**
- The host engine (`nexus-host`) is UI-agnostic; plugin editors are native child
  windows; the app UI is swappable. The DAW UI choice (egui vs web) is
  deliberately deferred — nothing in the MVP bets against a future web DAW.
- MVP UI is **egui** (single process, parents plugin editors directly). The
  standalone host app lives in a new `nexus-app` crate so `nexus-daw` stays
  reserved for the eventual (likely web) DAW.
- Plugin stack mirrors the sibling `droplets` project: `clack` for CLAP (host
  side here), `clap-wrapper` for VST3 *export* later. VST3 *hosting* in the MVP
  uses Rust VST3 SDK bindings.
- **Full plugin-aspect parity** is a committed goal: `nexus-host` drives, and
  `nexus-wrap` forwards, *every* plugin extension (audio/MIDI I/O, params,
  state, GUI, latency/tail/voice-info, …) through one `PluginInstance` surface.
  Phase 01 implements full CLAP host-side parity. See
  [designs/PLUGIN_EXTENSIONS.md](designs/PLUGIN_EXTENSIONS.md).

**Notes:**
- Uses `jj` (Jujutsu), colocated with git. Commits: `jj describe` / `jj commit`.
- Build/bundle via `xtask`: `cargo bundle`, `cargo install-plugin`.

---

## Crate map

| Crate | Kind | Role | MVP? |
| --- | --- | --- | --- |
| `nexus-host` | lib | Discovery + format-agnostic hosting of CLAP/VST3 + audio/MIDI engine | yes |
| `nexus-app` | bin | egui standalone host UI (the 0.1 deliverable) | yes (new) |
| `nexus-wrap` | cdylib | A plugin that hosts another plugin | later |
| `nexus-daw` | bin | The AI-first DAW (UI TBD, likely web) | later |
| `xtask` | bin | Build automation / bundling | — |

---
project: nexus
version: "0.1"
phase: "01-mvp"
architecture_updated: 2026-06-03
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
- **Device I/O lives in `nexus-app`, not `nexus-host`.** `nexus-host` is a pure
  hosting library with no CPAL/MIDI dependency; it exposes a plugin's
  `AudioProcessor` for *someone else* to drive. `nexus-app` owns the CPAL audio
  devices, MIDI input (`midir`), the realtime callback, and the UI for picking
  audio in/out devices and MIDI sources. This is essential: `nexus-wrap` gets
  its audio/MIDI from the host DAW, so the host library must not pull in
  hardware I/O.
- Plugin stack mirrors the sibling `droplets` project: `clack` for CLAP (host
  side here), `clap-wrapper` for VST3 *export* later. VST3 *hosting* in the MVP
  uses Rust VST3 SDK bindings.
- **Full plugin-aspect parity** is a committed goal: `nexus-host` drives, and
  `nexus-wrap` forwards, *every* plugin extension (audio/MIDI I/O, params,
  state, GUI, latency/tail/voice-info, …) through one `PluginInstance` surface.
  Phase 01 implements full CLAP host-side parity. See
  [designs/PLUGIN_EXTENSIONS.md](designs/PLUGIN_EXTENSIONS.md).

**Notes:**
- Uses `jj` (Jujutsu), colocated with git. Commit with `jj commit -m`.
- Build/bundle via `xtask`: `cargo bundle`, `cargo install-plugin`.

---

## Crate map

| Crate | Kind | Role | MVP? |
| --- | --- | --- | --- |
| `nexus-host` | lib | Discovery + format-agnostic hosting of CLAP/VST3. **No device I/O.** | yes |
| `nexus-app` | bin | egui standalone: CPAL audio + MIDI (`midir`) setup, the realtime engine, device-selection UI, plugin browser (the 0.1 deliverable) | yes (new) |
| `nexus-wrap` | cdylib | A plugin that hosts another plugin | later |
| `nexus-daw` | bin | The AI-first DAW (UI TBD, likely web) | later |
| `xtask` | bin | Build automation / bundling | — |

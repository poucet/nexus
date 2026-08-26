---
project: nexus
version: "0.1"
phase: "complete"
architecture_updated: 2026-06-03
---

# Nexus

**Problem:** There is no Rust-native, AI-first environment for hosting VST3/CLAP
plugins. Nexus is built in layers so each layer is independently useful: a
format-agnostic hosting **engine**, a **plugin that wraps another plugin** (load
any wrapped plugin inside a DAW), and ultimately an **AI-first DAW**.

**Status: 0.1 — *Load & Hear* — is complete** (2026-08-26, tagged `v0.1.0`).
`nexus-app` scans the macOS plugin folders Bitwig/Ableton use, lists every CLAP
and VST3 found, loads one, plays it (CPAL audio out; hardware MIDI or the
built-in virtual keyboard), opens its editor in a native window, and
saves/recalls presets. Scan failures are surfaced in the UI and cached. See
`README.md` for the quickstart. The next version is not planned yet
(`/simply:next` starts it).

**Key decisions (see [designs/ARCHITECTURE.md](designs/ARCHITECTURE.md)):**
- The host engine (`nexus-host`) is UI-agnostic; plugin editors are native child
  windows; the app UI is swappable. The DAW UI choice (egui vs web) is
  deliberately deferred — nothing in 0.1 bets against a future web DAW.
- The 0.1 UI is **egui** (single process, parents plugin editors directly),
  rendered via **wgpu/Metal** — plugin editors that run their own OpenGL clobber
  a GL-based host renderer. The standalone host app lives in `nexus-app` so
  `nexus-daw` stays reserved for the eventual (likely web) DAW.
- **Device I/O lives in `nexus-app`, not `nexus-host`.** The engine exposes a
  plugin's `AudioProcessor` for *someone else* to drive; the app owns CPAL,
  MIDI (`midir`), and the realtime callback. Essential because `nexus-wrap`
  gets its audio/MIDI from the host DAW. Tearing a plugin down requires the
  audio thread to acknowledge dropping its processor first (clear/ack
  handshake) — destroying a plugin mid-`process` segfaults.
- **The wrapper UI lives in `nexus-host`, behind an opt-in `ui` feature**
  (egui; off by default): plugin selector, preset save/recall, and the
  inner-editor frame, reused by `nexus-app` and later `nexus-wrap`. A
  **preset** captures plugin identity (format + id + path) + the inner
  plugin's opaque state via `save_state`/`load_state`.
- Plugin stack mirrors the sibling `droplets` project: `clack` for CLAP
  hosting; VST3 hosting via the pure-Rust `vst3` COM bindings; `clap-wrapper`
  for VST3 *export* later.
- **Full plugin-aspect parity** is a committed goal: `nexus-host` drives, and
  `nexus-wrap` will forward, *every* plugin extension through one
  `PluginInstance` surface. 0.1 shipped full CLAP host-side parity; VST3 covers
  load/process/params/state/GUI. See
  [designs/PLUGIN_EXTENSIONS.md](designs/PLUGIN_EXTENSIONS.md).

**Backlog (unscheduled, carried from the 0.1 roadmap):**
- `nexus-wrap`: ship the host as a CLAP/VST3 plugin, forwarding the full
  extension surface to the inner plugin (VST3 via `clap-wrapper`).
- Full VST3-host extension parity (param automation via `IParameterChanges`,
  units, note-expression, `IPlugFrame::resizeView`, …).
- CLAP long-tail extensions (surround/ambisonic, posix-fd, thread-pool,
  param-indication, remote-controls, preset-load, context-menu, state-context,
  note-name, tail, track-info).
- Cross-platform support (Windows/Linux discovery paths + editor windows; only
  macOS is implemented, other OSes `compile_error!`).
- Plugin-state explorer: snapshot/diff plugin state to map bytes ↔ parameters,
  toward driving plugins over MCP (the AI-first goal).
- DAW UI spike (likely web) → `nexus-daw`.
- Sandboxed/out-of-process probing so a plugin that crashes in its own
  initializer can't take the app down.

**Notes:**
- Uses `jj` (Jujutsu), colocated with git. Commit with `jj commit -m`.
- Build/bundle via `xtask`: `cargo bundle`, `cargo install-plugin`.

---

## Crate map

| Crate | Kind | Role | Status |
| --- | --- | --- | --- |
| `nexus-host` | lib | Discovery + format-agnostic hosting of CLAP/VST3 + preset model + *optional* egui wrapper UI (`ui` feature). **No device I/O.** | shipped in 0.1 |
| `nexus-app` | bin | egui standalone host: CPAL audio + MIDI, realtime engine, device selection, plugin browser + editor windows | shipped in 0.1 |
| `nexus-wrap` | cdylib | A plugin that hosts another plugin | stub |
| `nexus-daw` | bin | The AI-first DAW (UI TBD, likely web) | stub |
| `xtask` | bin | Build automation / bundling | — |

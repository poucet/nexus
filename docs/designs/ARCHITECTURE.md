# Architecture & UI strategy

**Status:** planned (drives nexus 0.1, Phase 01)
**Date:** 2026-06-01

## Problem

Nexus must host VST3 and CLAP plugins *and* eventually be a DAW. The DAW UI will
likely be web (rich timeline/piano-roll/mixer tooling, designer-friendly), but
plugin editors are fundamentally native and cannot live inside a webview. We
need an architecture that ships a usable host now without betting against a
future web DAW.

## The constraint that forces the design

A VST3/CLAP plugin editor is a **native OS view** (NSView / HWND / X11 window),
not HTML. The host hands the plugin a parent window handle:

- **CLAP:** `clap_plugin_gui` → host passes a `clap_window`, calls `set_parent`.
- **VST3:** `IPlugView::attached(parent, platformType)`.

Implications:
- A **web UI cannot embed** a plugin editor in the DOM — the editor must be a
  separate floating native window the Rust side spawns and positions.
- A **native UI (egui/winit)** owns a real window handle, so it can parent the
  editor as a child view *inside* its own window.
- Either way, the **plugin-window embedding is native and separable** from the
  main UI.

## Decision

Three layers, decoupled:

1. **`nexus-host`** — UI-agnostic engine. Discovery, format-agnostic plugin
   instances (CLAP via `clack`, VST3 via Rust SDK bindings), and an audio/MIDI
   engine (CPAL + midir). Exposes commands/state; knows nothing about the UI.
2. **Native plugin-window embedding** — creates the child native window and
   parents the editor in. Needed by *any* UI. For the MVP it lives in
   `nexus-app`; extract to its own crate when a second UI (web DAW) needs it.
3. **App UI** — swappable. MVP is **egui** (`nexus-app`). The DAW UI is decided
   later and can be web (arrangement/mixer in web, editors as floating native
   windows) while reusing layers 1 and 2.

### Why egui for the MVP, not web

- One process: the host window can both list plugins and parent the editor, with
  no IPC.
- It builds `nexus-host` and the embedding component, both reused regardless of
  the eventual DAW UI — so no work is wasted.
- egui is excellent for host/mixer/parameter surfaces. A full arrangement DAW in
  egui is capable-but-laborious; that is exactly the call we defer, not make now.

### Why `nexus-app` is separate from `nexus-daw`

`nexus-daw` is reserved for the eventual DAW (likely web). The egui standalone
host is its own crate so the two don't collide; the engine + embedding layers are
the shared, durable parts.

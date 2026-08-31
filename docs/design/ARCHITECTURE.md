# Architecture & UI strategy

**Status:** planned (drives nexus 0.1, Phase 01)
**Date:** 2026-06-01

## Problem

Nexus must host VST3 and CLAP plugins behind a UI that can change: the first
deliverable is a native egui app, but richer frontends (possibly web) may come
later — and plugin editors are fundamentally native and cannot live inside a
webview. We need an architecture that ships a usable host now without betting
against a future non-native UI.

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

1. **`nexus-host`** — **device-agnostic** hosting library. Discovery,
   format-agnostic plugin instances (CLAP via `clack`, VST3 via Rust SDK
   bindings), the `PluginInstance` / `AudioProcessor` surface, and a **preset
   model** (plugin identity + opaque state, saved to a file). It has **no
   audio/MIDI hardware dependency** — it exposes a plugin's `AudioProcessor` for
   a *driver* to run, and knows nothing about CPAL or MIDI devices.

   It *also* ships an **optional egui "wrapper UI"** behind a `ui` feature
   (off by default): a plugin selector, preset save/recall controls, and the
   frame that embeds the inner plugin's editor. This is the one UI that lives in
   the library, on purpose (see below).
2. **`nexus-app`** — the standalone driver + UI. Owns the **CPAL** audio devices,
   **MIDI** input (`midir`), and the realtime callback that pulls audio from the
   host's `AudioProcessor` and feeds it MIDI; plus the egui UI, including
   **device selection** (audio in/out devices, MIDI sources). Also hosts the
   native plugin-window embedding (the child window the editor parents into) —
   extract that to its own crate when a second UI needs it.
3. **App UI** — swappable. MVP is **egui** (`nexus-app`). A future frontend can
   be web (main UI in web, editors as floating native windows) while reusing
   layers 1 and 2.

### Why the wrapper UI lives in `nexus-host` (behind a feature)

A "host around a plugin" — plugin selector, preset save/recall, and the inner
editor frame — is needed identically by **`nexus-app`** (standalone) and
**`nexus-wrap`** (the plugin-in-a-DAW, whose *entire* editor is exactly that
chrome). Putting it in `nexus-host` means `nexus-wrap` gets it **for free**.

To keep that from contradicting the device-/UI-agnostic goal, the wrapper UI is
gated behind an opt-in **`ui` feature** (egui), off by default:

- `nexus-app` / `nexus-wrap` build with `features = ["ui"]` → free chrome.
- The bare library, and any future web frontend, omit the feature → no egui.

Device I/O is *not* given the same treatment — it stays out of `nexus-host`
entirely (next section), because no consumer of the library wants hardware I/O
baked in.

### Why device I/O lives in `nexus-app`, not `nexus-host`

`nexus-wrap` (a plugin) receives its audio and MIDI **from the host DAW**, never
from hardware. If CPAL/`midir` lived in `nexus-host`, every `nexus-wrap` build
would drag in audio-device code it must never use. So hardware I/O is strictly an
*application* concern: `nexus-host` stays a pure library that processes whatever
buffers/events it's handed, and `nexus-app` is the one that sources them from
real devices.

### Why egui for the MVP, not web

- One process: the host window can both list plugins and parent the editor, with
  no IPC.
- It builds `nexus-host` and the embedding component, both reused regardless of
  any future frontend — so no work is wasted.
- egui is excellent for host/mixer/parameter surfaces; anything richer is a
  call we defer, not make now. The engine + embedding layers are the shared,
  durable parts either way.

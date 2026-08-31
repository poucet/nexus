# Nexus — CHANGELOG

> What belongs here, and what does not: @AGENTS.md §2. Newest first.

| Dates | Tag | |
|---|---|---|
| 2026-08-26 → … | — | **[Unreleased](#unreleased)** — 0.2 State tools in progress |
| 2026-06-01 → 08-26 | `v0.1.0` | [Milestone 0.1: Load & Hear](#v010--load--hear) — CLAP/VST3 hosting engine + standalone egui app |

---

## Unreleased

#### REST frontend + debug UI over the tool registry (`REST-DEBUG`) · 2026-08-26 08:40 · qqkwzolo

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

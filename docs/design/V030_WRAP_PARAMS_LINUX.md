# Design Proposal: v0.3.0 — Cross-Platform (Linux/Docker), Realtime Parameter Automation Ring, and `nexus-wrap`

> ⚠️ PROPOSAL — Awaiting human review before landing on trunk.

**Status:** Proposed  
**Date:** 2026-09-01  
**Target:** Nexus 0.3.0  

---

## 1. Executive Summary & Goals

Nexus 0.2.0 delivered structured state inspection/codecs, standalone headless MCP server capabilities, and driver-free offline fast audio rendering with the `simply-fugue` sequencing model.

Milestone **0.3.0** expands Nexus across three core vectors:
1. **Cross-Platform Support (Linux & Docker)**: Enable full headless execution, CI verification, and AI sampling workflows in Linux container environments without requiring hardware audio or display servers.
2. **Realtime Parameter Automation Ring**: Implement a lock-free Single-Producer Single-Consumer (SPSC) parameter change ring (`rtrb`) from the host into `AudioProcessor` for both CLAP and VST3, enabling sample-accurate parameter modulation curves during both live playback and offline sampling.
3. **`nexus-wrap` In-DAW Plugin Wrapper**: Package Nexus as an audio plugin (`.clap` and `.vst3`) that hosts inner plugins, forwards audio/MIDI/parameters/GUI seamlessly, and starts an in-process MCP/REST server so AI agents can inspect and automate plugins running live inside any DAW (Bitwig, Reaper, Live).

---

## 2. Cross-Platform Support: Linux & Docker

### 2.1 Problem & Motivation
Development occurs primarily on macOS, but server-side AI execution, continuous integration (CI), and cloud agent pipelines run on Linux. We do not need Windows support immediately, but Linux is achievable and testable via Docker on macOS.

### 2.2 Architecture & Paths
- **Discovery Paths**:
  - **CLAP**:
    - User scope: `~/.clap`
    - System scope: `/usr/lib/clap`, `/usr/local/lib/clap`
  - **VST3**:
    - User scope: `~/.vst3`
    - System scope: `/usr/lib/vst3`, `/usr/local/lib/vst3`
- **Headless & Driver-Free Design**:
  - Offline sampling (`render_offline`) and the standalone MCP server require **zero** audio drivers (`cpal`) or display servers (X11/Wayland).
  - Standalone headless runner runs cleanly in minimal Linux Docker containers.
- **Audio Output Backend (Live App)**:
  - CPAL supports ALSA and JACK/PipeWire on Linux.
  - Optional feature flags `alsa` / `jack` in `nexus-app`.
- **Plugin GUI Embedding (X11 / Wayland)**:
  - On macOS, `editor.rs` parents `NSView` via AppKit.
  - On Linux:
    - CLAP provides `CLAP_WINDOW_API_X11` / `CLAP_WINDOW_API_WAYLAND`.
    - VST3 provides `kPlatformTypeHWND` (X11 window ID on Linux).
    - In headless / Docker mode, GUI is not initialized; AI tools use headless parameter and state APIs.

### 2.3 Docker Environment Definition
- Provide a standard `Dockerfile` and `docker-compose.yml`:
  - Base: Debian/Ubuntu slim or Arch Linux with Rust toolchain and build tools (`clang`, `libasound2-dev`).
  - Volume mount for Linux `.clap` and `.vst3` plugin libraries (e.g. Surge XT, Vital, Odin2).
  - Entrypoint: `nexus-mcp --port 9997` or headless test runners.

---

## 3. Realtime Parameter Automation Ring

### 3.1 Problem
In 0.2.0, the host can query parameters (`list_params`, `get_params`) and set parameters statically on the main thread (`set_params`). However, `AudioProcessor` currently only processes MIDI Note and CC events in its audio render loop; dynamic parameter curves (such as filter sweeps, envelope modulations, and LFO shapes defined in `simply-fugue`) are not fed sample-accurately into the DSP process loop.

### 3.2 Lock-Free SPSC Ring Architecture
```
Main Thread / Host / Fugue Compiler
      │
      │  ParamChange { id: u32, value: f64, offset_frames: u32 }
      ▼
┌──────────────────────────────────────────────────┐
│  rtrb::Producer<ParamChange> (Lock-Free Ring)    │
└──────────────────────────────────────────────────┘
                      │
                      ▼
┌──────────────────────────────────────────────────┐
│  rtrb::Consumer<ParamChange> (Realtime Audio)   │
└──────────────────────────────────────────────────┘
      │
      ├─► CLAP Backend: clap_event_param_value in Input Event List
      └─► VST3 Backend: IParameterChanges / IParamValueQueue
```

### 3.3 Backend Event Dispatch
- **CLAP (`ClapAudioProcessor`)**:
  - Drain parameter ring events per audio block.
  - Emit `clap_event_param_value` structures with `param_id`, `value`, and `time` (sample frame offset in block).
  - Support modulation gestures via `clap_event_param_mod` when applicable.
- **VST3 (`Vst3AudioProcessor`)**:
  - Implement `vst3/param_changes.rs` holding preallocated `IParameterChanges` and `IParamValueQueue` COM objects.
  - Map plain parameter values to normalized values (`plainParamToNormalized`).
  - Assign to `ProcessData.inputParameterChanges` before calling `IAudioProcessor::process`.
- **Fugue Parameter Curve Compilation**:
  - `simply_fugue::composite` automation curves (`InterpolationMode::Linear`, `Exponential`, `Logarithmic`) compile into time-stamped `ParamChange` points sliced per block.

---

## 4. `nexus-wrap` — In-DAW Plugin Wrapper

### 4.1 Concept & Motivation
`nexus-wrap` is a plugin that you load inside any DAW (e.g. Bitwig, Reaper, Ableton Live) as an instrument or audio effect. Inside, it instantiates an inner CLAP or VST3 plugin (e.g. Vital, Serum 2, Diva). 

While the inner plugin renders audio and is played directly by the DAW's transport, `nexus-wrap` exposes a background loopback server (`REST` on 9996, `MCP` on 9997). This allows an external AI coding agent to connect to the active DAW session, inspect state, tweak parameters in realtime, and trigger sampling!

```
┌────────────────────────────────────────────────────────────────────────┐
│ Digital Audio Workstation (DAW)                                        │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ nexus-wrap (Outer CLAP / VST3 Plugin)                             │  │
│  │                                                                  │  │
│  │   ┌──────────────────────────────────────────────────────────┐   │  │
│  │   │ Inner Plugin Instance (e.g. Vital / Serum 2 / Diva)      │   │  │
│  │   │ Audio / MIDI / Parameters / GUI View                     │   │  │
│  │   └──────────────────────────────────────────────────────────┘   │  │
│  │                                 ▲                                │  │
│  │                                 │ Host Bridge                    │  │
│  │   ┌─────────────────────────────┴────────────────────────────┐   │  │
│  │   │ Background Loopback Server (nexus-tools + rmcp / axum)   │   │  │
│  │   │  • GET/POST http://127.0.0.1:9996 (REST / Debug UI)      │   │  │
│  │   │  • MCP http://127.0.0.1:9997 (Model Context Protocol)    │   │  │
│  │   └──────────────────────────────────────────────────────────┘   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Crate & Bundle Architecture
- **Crate**: `crates/nexus-wrap`
- **Formats**:
  - Primary: **CLAP** (`clack-plugin` entry point)
  - Secondary: **VST3** (`vst3-sdk` class factory)
- **Forwarding Pipeline**:
  - **Audio & MIDI**: Outer `process()` directly converts input audio/MIDI buffers and invokes inner `PluginInstance::activate()` processor.
  - **Parameters**: Outer plugin reflects inner plugin parameter descriptors so DAW automation lanes map directly to the inner synth.
  - **GUI Window**: Outer editor attaches the inner plugin's GUI view to the parent window handle provided by the DAW.
  - **State Persistence**: Outer plugin saves the inner plugin's full state blob in its chunk, guaranteeing that saving the DAW project file preserves the inner synth's patch.

---

## 5. Workspaces, Subagents & Lane Plan

Following repository rules (`AGENTS.md`), we organize the execution into isolated work lanes using Jujutsu workspaces (`.workspaces/<lane>`):

```
                       ┌─────────────────────────┐
                       │   Tech Lead (Design)    │
                       │ docs/design/V030_...md  │
                       └────────────┬────────────┘
                                    │
                                    ▼
       ┌────────────────────────────┼────────────────────────────┐
       │                            │                            │
       ▼                            ▼                            ▼
┌──────────────┐             ┌──────────────┐             ┌──────────────┐
│ Lane: linux  │             │ Lane: params │             │ Lane: wrap   │
│ Linux paths, │             │ rtrb SPSC,   │             │ nexus-wrap   │
│ Dockerfile,  │             │ CLAP/VST3    │             │ CLAP/VST3    │
│ CI runner    │             │ process ring │             │ inner host   │
└──────────────┘             └──────────────┘             └──────────────┘
```

1. **Lane 1 (`lane-linux`)**:
   - Linux plugin paths and system directory scans.
   - `Dockerfile` and headless smoke tests in Docker.
2. **Lane 2 (`lane-params`)**:
   - `rtrb` ring creation in `PluginInstance::activate`.
   - Parameter change conversion in `ClapAudioProcessor` and `Vst3AudioProcessor`.
   - Fugue parameter curve sampling integration.
3. **Lane 3 (`lane-wrap`)**:
   - `nexus-wrap` CLAP plugin entry and inner hosting bridge.
   - Background MCP/REST loopback server in wrapped context.
   - Inner editor window forwarding.

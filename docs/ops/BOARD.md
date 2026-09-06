# Nexus — BOARD

> What belongs here, and what does not: @AGENTS.md §2.
> Scratchpad state lives in `docs/scratch/` — gitignored, invisible to lanes.

Legend: 🚧 in flight · ✅ landed · ⏸ deferred.

## In flight

## Milestones

### 0.4 - AI Sampler Harness & State Exploration

Headless VST3/CLAP plugin sampling, offline rendering harness (`nexus-mcp`), state codecs, and DAW wrapper extensions. (Downstream audio analysis & patch reconstruction live in `../sense` and `../flux`.)

#### Feature 3: Verification & External MCP Harness — ⬜ Queued

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ⬜ | T27 | `MCP-SMOKE` | End-to-end MCP verification with Claude Desktop / Cursor / external agent harnesses | `claude mcp add --transport http` on port 9997 |

## Queued

| St | Pri | Size | Label | What | Notes |
|---|---|---|---|---|---|


## Deferred by decision

| St | Label | What | Reason / Decision |
|---|---|---|---|
| ⏸ | `INVESTIGATE` | Manual binary state reverse-engineering pass (Vital, Serum 2, Arturia) | Deferred in favor of audio sampling + Flux modular reconstruction workflow |
| ⏸ | `CODEC-SERUM2` | `serum2` binary state codec | Deferred; focus is sample-and-reconstruct into Flux |
| ⏸ | `CODEC-ARTURIA` | `arturia` binary state codec | Deferred; focus is sample-and-reconstruct into Flux |

## Landed index

- **Offline render fixes, automation deduplication & parity** (✅ · 2026-09-06 · `wrturnym`) — arriving-point curve easing (`I-92`), CLAP parameter duplication fix (`I-94`), `params` held-value repeat suppression (`I-95`), 60 s offline render length ceiling (`I-96`), and tool documentation cleanup (`I-93`).
- **`render_audio` renders what the fugue says** (`FUGUENEXUS-2` ✅ · 2026-09-04 · `vwwnyskw` `wzsyxqru` `wvyzxrsm` `tswpsmmv`) — the behaviour half of FUGUE-API.md §6 on nexus: `Log` = `1 − (1 − t)²`, arriving-point curve easing (`I-92`), CC curves honoured, the five swallowed event kinds rendered, a fixed 1.0 s release tail, `loop_mode` offline, the note-off tie-break, `output_format` as the shared enum defaulting to `file`, `warnings` filled; the three host impls share one post-render glue. Same numbers as flux's `FUGUEONE`; evidence in @docs/research/render-audio-fixture-table.md.
- **`simply-fugue` consumed via git dependency (`poucet/music-rs`)** (✅ · 2026-09-04 · `ulrvzslx`) — switched `simply-fugue` from local path dependency to git dependency `https://github.com/poucet/music-rs.git`. No sibling clone required; Docker container builds cleanly without context re-rooting.
- **`simply-fugue` moved to the shared `music` repo** (`FUGUENEXUS` ✅ · 2026-09-03 · `qpnkxvoo`) — the fugue wire format is now shared with Flux and lives in `~/projects/simply/music` (`poucet/music-rs`); nexus consumes it as a dependency, the vendored copy is gone, and `RenderAudioRequest`/`RenderAudioResponse` moved into the crate so the two hosts share the type. No behaviour change: the four `render.rs` fixes remain held.
- `T21` **Parameter Tools & Unit Conversion** (`PARAM-TOOLS` ✅ · 2026-09-01 · `xvzlvmrn` `16d25e93`) — Bidirectional parameter enumeration, reading, mutation, and plain $\leftrightarrow$ display-text conversion (`convert_param_value` via CLAP `PluginParams` and VST3 `IEditController`) exposed across `nexus-tools`, standalone MCP server, in-DAW wrapper, REST server, and debug web UI.
- **v0.3.0 In-DAW Wrapper, Parameter Automation & Cross-Platform Linux** (✅ · 2026-09-01 · `v0.3.0`) — `nexus-wrap` CLAP plugin wrapper, in-DAW loopback MCP/REST server, realtime parameter automation ring (`rtrb` SPSC), Linux discovery paths/POSIX loader, Docker container setup, and AI patch reconstruction design specs.
- **`nexus-wrap` Plugin Wrapper & In-DAW Loopback Bridge** (`NEXUS-WRAP` ✅ · 2026-09-01 · `koyzxqvo` `3982a4e6`) — `crates/nexus-wrap` CLAP plugin exporting `PluginAudioPorts`, `PluginNotePorts`, `PluginParams`, and `PluginState`. Inner plugin hosting bridge on top of `nexus-host` forwarding audio/MIDI buffers and reflecting parameter changes to DAWs. Background worker thread spinning up in-DAW loopback MCP (`rmcp` on 9997) and REST (`axum` on 9996) servers for live DAW AI automation. `WrapStateEnvelope` preserving inner plugin state blobs across DAW project saves.
- **Linux Cross-Platform Support & Docker Setup** (`LINUX-DOCKER` ✅ · 2026-09-01 · `wnlyruwx` `daad4d12`) — Standard Linux plugin search paths for CLAP (`~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`) and VST3 (`~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3`). Clean target gating for macOS-specific AppKit window parenting, POSIX dynamic module loader for Linux VST3 bundles (`vst3/module/linux.rs`), multi-stage `Dockerfile` + `docker-compose.yml` for headless container testing and CI, and `docs/guides/LINUX.md`.
- `T18`–`T20` **Realtime Parameter Automation Ring** (`PARAM-AUTOMATION` ✅ · 2026-09-01 · `nprppvmq` `1809d508`) — Lock-free `rtrb` SPSC parameter ring (`ParamChange`) instantiated on `PluginInstance::activate`, CLAP `ParamValueEvent` ring drain and event translation in `ClapAudioProcessor`, VST3 `IParameterChanges` / `IParamValueQueue` COM objects in `vst3/param_changes.rs` wired into `ProcessData.inputParameterChanges` with normalized value mapping, and Fugue parameter automation curve evaluation and block slicing in `render_offline`.
- **v0.2.0 State Tools & AI Sampler** (✅ · 2026-09-01 · `quurmwqx` `8cc33975` `v0.2.0`) — Standalone MCP server (`nexus-mcp`), driver-free fast offline rendering engine (`render_offline` + 32-bit float WAV), `simply-fugue` musical sequencing model, 10 expanded tool definitions, REST debug UI audio player, and native VST3/CLAP generic preset and parameter APIs.
- `T13`–`T14` **REST frontend + debug UI** (`REST-DEBUG` ✅ · 2026-08-26 · `qqkwzolo` `4b799c8c`) — `crates/nexus-rest` (axum server on dedicated thread + tokio runtime, loopback-only dual bind on 9996, `GET /tools`, `POST /tools/{name}`, per-tool timeouts, socket tests) + `/debug` page (inlined HTML/JS, collapsible response tree, snapshot/diff-against-live).
- `T6`–`T12` **State, snapshots & codecs** (`STATE-CODECS` ✅ · 2026-08-26 · `luuznszn` `364ffc90`) — `get_state`/`set_state` (sha256/base64 cap, auto-snapshot), `SnapshotStore` (in-memory 32/256MB LRU + on-disk corpus export to `.../Nexus/state-corpus/<plugin_id>/`), `StateCodec` trait + ordered registry, `generic` sniff codec (recursive gzip/zlib, RIFF walk, VST3 chunk-list framing, JSON/XML detection, binary leaves), `vital-json` codec (lossless JSON decode/encode), `edit_state` (JSON-pointer ops, `base_sha256` guard, dry-run, through-the-plugin verify), `diff_state` (doc-level structural diff).
- `T1`–`T5` **Tools core** (`TOOLS-CORE` ✅ · 2026-08-26 · `luuznszn` `364ffc90`) — `crates/nexus-tools` scaffold (`ToolError`, DTOs with `schemars` JSON schemas), `NexusTools` async trait (10 methods), main-thread bridge (`HostRequest` + futures oneshot + crossbeam channel + `MainThreadHost`), hand-written `ToolRegistry`, `nexus-app` wiring (`tools_host.rs` `MainThreadHost` impl + repaint waker).
- **v0.1.0 Load & Hear** (✅ · 2026-08-26 · `smpxzzxx` `v0.1.0` `fd1a476d`) — `nexus-host` discovery & hosting engine (CLAP via `clack`, VST3 via pure-Rust COM bindings, bundle scan cache, `.nexuspreset`), `nexus-app` standalone host (egui on Metal/`wgpu`, native child window parenting, CPAL audio, MIDI in + virtual keyboard, `muda` menus, `⌘P` finder palette), `xtask` bundler.

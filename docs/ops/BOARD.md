# Nexus — BOARD

> What belongs here, and what does not: @AGENTS.md §2.
> Scratchpad state lives in `docs/scratch/` — gitignored, invisible to lanes.

Legend: 🚧 in flight · ✅ landed · ⏸ deferred.

## In flight

## Milestones

### 0.2 State tools: structured plugin state access

Structured, transport-agnostic access to a loaded plugin's stored state (extensible per-plugin codecs, snapshot→tweak→diff investigation loop, REST debug UI first, MCP last). Rationale and design: @docs/design/STATE_TOOLS.md. See @docs/ops/ROADMAP.md.

#### Feature 1: Tools core (`nexus-tools`) — ✅ Landed

The transport-agnostic trait, DTOs, registry, and main-thread bridge.

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T1 | `TOOLS-SCAFFOLD` | `crates/nexus-tools` scaffold: crate, workspace entry, `ToolError { code, message }`, deps (serde, serde_json, schemars, async-trait, crossbeam-channel, futures, base64, sha2, flate2, log, nexus-host) | `crates/nexus-tools/` — error.rs (`ErrorCode` + actionable messages). No tokio/axum. |
| ✅ | T2 | `TOOLS-TRAIT` | `NexusTools` trait core (get_session, decode_state, edit_state, get/set_state, snapshot_state/restore/list, diff_state, list_codecs) + DTOs with JsonSchema; `instance:"default"` on every request | tools.rs + types.rs — 10 methods; requests reject instance ≠ "default". |
| ✅ | T3 | `TOOLS-BRIDGE` | Main-thread bridge: `HostRequest` enum + futures oneshot replies + crossbeam channel + `drain(rx, &mut impl MainThreadHost)` + waker hook | bridge.rs — `MainThreadHost` is the small sync trait apps implement; timeouts are the frontend's job (`TimeoutClass` on each ToolDef). Service tests marshal through a real bridge + MockHost thread. |
| ✅ | T4 | `TOOLS-REGISTRY` | Hand-written tool registry: `ToolDef { name, description, input/output schema, handler }` + dispatch; no macro in v1 | registry.rs — local `tool!` macro_rules builds entries; `descriptors()` feeds GET /tools. 4 tests. |
| ✅ | T5 | `APP-WIRING` | `nexus-app` wiring: `tools_host.rs` implements `MainThreadHost` (state ops on the loaded `PluginInstance`), drain at top of `update()`, repaint waker | `crates/nexus-app/src/tools_host.rs`; app holds `Arc<ToolsService>` for the F3 REST server. Compile-verified. |

#### Feature 2: State, snapshots & codecs — ✅ Landed

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T6 | `STATE-BLOB` | get_state/set_state: sha256, size, base64 opt-in with `max_bytes` cap; set_state auto-snapshots prior state | service.rs — set_state re-saves after load (proves the plugin accepted it; reports normalized state). |
| ✅ | T7 | `STATE-SNAPSHOT` | Snapshot store (in-memory, 32 / 256 MB LRU) + on-disk corpus export (`.../Nexus/state-corpus/<plugin_id>/<name>.state` + JSON sidecar) + list/restore | snapshots.rs — LRU tests; export is best-effort-tested via real use (writes under $HOME). restore rejects plugin-id mismatch (Conflict). |
| ✅ | T8 | `CODEC-REGISTRY` | `StateCodec` trait + ordered registry + confidence selection + list_codecs | codec.rs — highest confidence wins; `CodecRegistry::standard()` is the one registration point. |
| ✅ | T9 | `CODEC-GENERIC` | `generic` sniff codec (decode-only): gzip/zlib recurse, RIFF walk, VST3-preset framing, JSON/XML detection (whole + at-offset), binary leaves (entropy/strings/hexdump); depth + 32MB caps | codecs/generic.rs — 7 fixture tests (gzip-wrapped JSON, XML histogram, RIFF chunks, noise entropy, embedded JSON at offset, string extraction). |
| ✅ | T10 | `CODEC-VITAL` | `vital-json` codec (decode+encode) | codecs/vital.rs — matches Vital-named plugins with JSON state (confidence 220); round-trip test. |
| ✅ | T11 | `STATE-EDIT` | Shared edit application (JSON-pointer ops on the decoded doc) + `edit_state`: `base_sha256` guard, dry-run, auto-snapshot, through-the-plugin verify (load → re-save → re-decode → compare edited paths) | codec.rs (`apply_edits`, RFC 6901 incl. escapes + array append) + service.rs; float-tolerant verification. Full-flow test: dry-run → apply → verified → stale-hash Conflict → auto-snapshot restore. |
| ✅ | T12 | `STATE-DIFF` | `diff_state(a, b=live)`: doc-level structural diff (byte info stays in the generic codec's binary leaves) | diff.rs — changed/added/removed with escaped pointer paths, big values summarized, 500-change cap. 5 tests. |

#### Feature 3: REST + debug UI (`nexus-rest`) — 🔶 In Progress

Investigation starts here.

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T13 | `REST-SERVER` | `crates/nexus-rest`: server bootstrap on own thread + tokio runtime (droplets pattern: OnceLock, loopback-only, dual 127.0.0.1+[::1] bind), `GET /tools`, `POST /tools/{name}`; port 9996 / `NEXUS_REST_PORT` | lib.rs — per-tool timeouts from `TimeoutClass` (5s/30s); error codes → HTTP status. **End-to-end tested over real sockets** (tests/http.rs: tools listing, empty-body default, 409/404/400 mapping, debug page). Wired into `nexus-app` (status line shows the URL). |
| ✅ | T14 | `REST-DEBUG-UI` | `/debug` page (one inlined HTML+JS file): tool list, raw JSON request editor (schema shown for reference), collapsible response tree, one-click snapshot / diff-against-live | debug.html — quick-action bar (Session / Decode live / Snapshot / Diff vs last snap), per-tool example bodies, captures `base_sha256` from decode responses into the edit_state example, ⌘⏎ sends. Visual check = T15. |
| ⬜ | T15 | `INVESTIGATE` | Manual investigation pass: Vital, Serum 2, Arturia — snapshot → GUI knob → diff loop; export corpus; record per-plugin findings here | user-driven; findings feed F4/F6 |

#### Feature 4: Codec expansion (driven by F3 findings) — ⬜ Queued

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ⬜ | T16 | `CODEC-SERUM2` | `serum2` codec (decode-first) | from corpus |
| ⬜ | T17 | `CODEC-ARTURIA` | `arturia` codec (decode-first) | from corpus |

#### Feature 5: Param-set plumbing (`nexus-host`) — ✅ Landed

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T18 | `PARAM-HOST` | `PluginInstance::set_param` + `ParamChange` SPSC ring; `ParamInfo: Serialize`; per-instance `rtrb` ParamChange ring created at `activate()` | host.rs, PluginInstance trait |
| ✅ | T19 | `PARAM-CLAP` | CLAP: ring drain → `ParamValueEvent`s; translate façade ParamValue/Mod/Gesture* from block.events | clap/process.rs, clap/instance.rs |
| ✅ | T20 | `PARAM-VST3` | VST3: `vst3/param_changes.rs` (IParamValueQueue/IParameterChanges, prealloc) → `ProcessData.inputParameterChanges`; normalized values on ring; `setParamNormalized` GUI sync | vst3/param_changes.rs, vst3/process.rs, vst3/instance.rs |
| ⬜ | T21 | `PARAM-TOOLS` | Params tools (list_params paged/filtered, get_params, set_params, convert_param_value) in trait + registry + debug page | trait + registry + debug page |

#### Feature 6: Contract growth (as investigation demands) — ✅ Landed

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T22 | `TOOLS-CATALOG` | Catalog/session tools: list_plugins, load_plugin | id → exact, else unique name match |
| ✅ | T23 | `TOOLS-PRESET` | Preset tools: save/load/list_presets | over existing `Preset` |
| ✅ | T24 | `RENDER-AUDIO` | Offline plugin sampling engine (`render_offline` + `hound` 32-bit float WAV encoder) | Driver-free offline rendering with Droplets fugue note/CC/param scheduling |
| ✅ | T25 | `PLAY-FUGUE` | Live performance playback tool (`play`) in `nexus-tools` and `nexus-app` | Transport-synced playback using Droplets Fugue data model |

#### Feature 7: MCP frontend — ✅ Landed

| St | # | Label | Task | Files / notes |
|---|---|---|---|---|
| ✅ | T26 | `MCP-CORE` | `crates/nexus-mcp`: rmcp 0.9.1 streamable-HTTP and stdio server over `ToolRegistry` + standalone headless runner | Dual transport: stdio (`nexus-mcp --stdio`) and HTTP (`nexus-mcp --port 9997`) |
| ⬜ | T27 | `MCP-SMOKE` | MCP E2E smoke with Claude/Cursor + docs | `claude mcp add --transport http` |

## Queued

| St | Pri | Size | Label | What | Notes |
|---|---|---|---|---|---|
| ⬜ | P1 | S | `INVESTIGATE` | Manual investigation pass: Vital, Serum 2, Arturia (F3, T15) | snapshot → GUI knob → diff loop; export corpus; record findings (feeds F4/F6) |
| ⬜ | P2 | S | `CODEC-SERUM2` | `serum2` codec (decode-first from corpus) (F4, T16) | Driven by F3 findings |
| ⬜ | P2 | S | `CODEC-ARTURIA` | `arturia` codec (decode-first from corpus) (F4, T17) | Driven by F3 findings |
| ⬜ | P2 | S | `MCP-SMOKE` | MCP E2E smoke + docs (F7, T27) | `claude mcp add --transport http`; README + design updates |

## Deferred by decision

## Landed index

The full record lives in @docs/ops/CHANGELOG.md.

- `T18`–`T20` **Realtime Parameter Automation Ring** (`PARAM-AUTOMATION` ✅ · 2026-09-01) — Lock-free `rtrb` SPSC parameter ring (`ParamChange`) instantiated on `PluginInstance::activate`, CLAP `ParamValueEvent` ring drain and event translation in `ClapAudioProcessor`, VST3 `IParameterChanges` / `IParamValueQueue` COM objects in `vst3/param_changes.rs` wired into `ProcessData.inputParameterChanges` with normalized value mapping, and Fugue parameter automation curve evaluation and block slicing in `render_offline`.
- **v0.2.0 State Tools & AI Sampler** (✅ · 2026-09-01 · `uztunkyr` `v0.2.0`) — Standalone MCP server (`nexus-mcp`), driver-free fast offline rendering engine (`render_offline` + 32-bit float WAV), `simply-fugue` musical sequencing model, 10 expanded tool definitions, REST debug UI audio player, and native VST3/CLAP generic preset and parameter APIs.
- `T13`–`T14` **REST frontend + debug UI** (`REST-DEBUG` ✅ · 2026-08-26 · `qqkwzolo` `4b799c8c`) — `crates/nexus-rest` (axum server on dedicated thread + tokio runtime, loopback-only dual bind on 9996, `GET /tools`, `POST /tools/{name}`, per-tool timeouts, socket tests) + `/debug` page (inlined HTML/JS, collapsible response tree, snapshot/diff-against-live).
- `T6`–`T12` **State, snapshots & codecs** (`STATE-CODECS` ✅ · 2026-08-26 · `luuznszn` `364ffc90`) — `get_state`/`set_state` (sha256/base64 cap, auto-snapshot), `SnapshotStore` (in-memory 32/256MB LRU + on-disk corpus export to `.../Nexus/state-corpus/<plugin_id>/`), `StateCodec` trait + ordered registry, `generic` sniff codec (recursive gzip/zlib, RIFF walk, VST3 chunk-list framing, JSON/XML detection, binary leaves), `vital-json` codec (lossless JSON decode/encode), `edit_state` (JSON-pointer ops, `base_sha256` guard, dry-run, through-the-plugin verify), `diff_state` (doc-level structural diff).
- `T1`–`T5` **Tools core** (`TOOLS-CORE` ✅ · 2026-08-26 · `luuznszn` `364ffc90`) — `crates/nexus-tools` scaffold (`ToolError`, DTOs with `schemars` JSON schemas), `NexusTools` async trait (10 methods), main-thread bridge (`HostRequest` + futures oneshot + crossbeam channel + `MainThreadHost`), hand-written `ToolRegistry`, `nexus-app` wiring (`tools_host.rs` `MainThreadHost` impl + repaint waker).
- **v0.1.0 Load & Hear** (✅ · 2026-08-26 · `smpxzzxx` `v0.1.0` `fd1a476d`) — `nexus-host` discovery & hosting engine (CLAP via `clack`, VST3 via pure-Rust COM bindings, bundle scan cache, `.nexuspreset`), `nexus-app` standalone host (egui on Metal/`wgpu`, native child window parenting, CPAL audio, MIDI in + virtual keyboard, `muda` menus, `⌘P` finder palette), `xtask` bundler.

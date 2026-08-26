# Nexus 0.2 — TASKS

Active task breakdown for **state tools** (structured access to plugin state). Design and rationale: [STATE_TOOLS.md](../../designs/STATE_TOOLS.md). See [ROADMAP.md](ROADMAP.md) for the feature list.

Status: ⬜ todo / 🔄 in progress / ✅ done.

## Feature 1: Tools core (`nexus-tools`)

The transport-agnostic trait, DTOs, registry, and main-thread bridge.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T1 | `crates/nexus-tools` scaffold: crate, workspace entry, `ToolError { code, message }`, deps (serde, serde_json, schemars, async-trait, crossbeam-channel, futures, base64, sha2, flate2, log, nexus-host) | `crates/nexus-tools/` — error.rs (`ErrorCode` + actionable messages). No tokio/axum. |
| ✅ | T2 | `NexusTools` trait core (get_session, decode_state, edit_state, get/set_state, snapshot_state/restore/list, diff_state, list_codecs) + DTOs with JsonSchema; `instance:"default"` on every request | tools.rs + types.rs — 10 methods; requests reject instance ≠ "default". |
| ✅ | T3 | Main-thread bridge: `HostRequest` enum + futures oneshot replies + crossbeam channel + `drain(rx, &mut impl MainThreadHost)` + waker hook | bridge.rs — `MainThreadHost` is the small sync trait apps implement; timeouts are the frontend's job (`TimeoutClass` on each ToolDef). Service tests marshal through a real bridge + MockHost thread. |
| ✅ | T4 | Hand-written tool registry: `ToolDef { name, description, input/output schema, handler }` + dispatch; no macro in v1 | registry.rs — local `tool!` macro_rules builds entries; `descriptors()` feeds GET /tools. 4 tests. |
| ✅ | T5 | `nexus-app` wiring: `tools_host.rs` implements `MainThreadHost` (state ops on the loaded `PluginInstance`), drain at top of `update()`, repaint waker | `crates/nexus-app/src/tools_host.rs`; app holds `Arc<ToolsService>` for the F3 REST server. Compile-verified. |

## Feature 2: State, snapshots & codecs

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ✅ | T6 | get_state/set_state: sha256, size, base64 opt-in with `max_bytes` cap; set_state auto-snapshots prior state | service.rs — set_state re-saves after load (proves the plugin accepted it; reports normalized state). |
| ✅ | T7 | Snapshot store (in-memory, 32 / 256 MB LRU) + on-disk corpus export (`.../Nexus/state-corpus/<plugin_id>/<name>.state` + JSON sidecar) + list/restore | snapshots.rs — LRU tests; export is best-effort-tested via real use (writes under $HOME). restore rejects plugin-id mismatch (Conflict). |
| ✅ | T8 | `StateCodec` trait + ordered registry + confidence selection + list_codecs | codec.rs — highest confidence wins; `CodecRegistry::standard()` is the one registration point. |
| ✅ | T9 | `generic` sniff codec (decode-only): gzip/zlib recurse, RIFF walk, VST3-preset framing, JSON/XML detection (whole + at-offset), binary leaves (entropy/strings/hexdump); depth + 32MB caps | codecs/generic.rs — 7 fixture tests (gzip-wrapped JSON, XML histogram, RIFF chunks, noise entropy, embedded JSON at offset, string extraction). |
| ✅ | T10 | `vital-json` codec (decode+encode) | codecs/vital.rs — matches Vital-named plugins with JSON state (confidence 220); round-trip test. |
| ✅ | T11 | Shared edit application (JSON-pointer ops on the decoded doc) + `edit_state`: `base_sha256` guard, dry-run, auto-snapshot, through-the-plugin verify (load → re-save → re-decode → compare edited paths) | codec.rs (`apply_edits`, RFC 6901 incl. escapes + array append) + service.rs; float-tolerant verification. Full-flow test: dry-run → apply → verified → stale-hash Conflict → auto-snapshot restore. |
| ✅ | T12 | `diff_state(a, b=live)`: doc-level structural diff (byte info stays in the generic codec's binary leaves) | diff.rs — changed/added/removed with escaped pointer paths, big values summarized, 500-change cap. 5 tests. |

## Feature 3: REST + debug UI (`nexus-rest`) — investigation starts here

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T13 | `crates/nexus-rest`: server bootstrap on own thread + tokio runtime (droplets pattern: OnceLock, loopback-only, dual 127.0.0.1+[::1] bind), `GET /tools`, `POST /tools/{name}`; port 9996 / `NEXUS_REST_PORT` | axum 0.7 pins from droplets |
| ⬜ | T14 | `/debug` page (one inlined HTML+JS file): tool list, raw JSON request editor (schema shown for reference), collapsible response tree, one-click snapshot / diff-against-live | no generated forms in v1 |
| ⬜ | T15 | Manual investigation pass: Vital, Serum 2, Arturia — snapshot → GUI knob → diff loop; export corpus; record per-plugin findings here | user-driven; findings feed F4/F6 |

## Feature 4: Codec expansion (driven by F3 findings)

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T16 | `serum2` codec (decode-first) | from corpus |
| ⬜ | T17 | `arturia` codec (decode-first) | from corpus |

## Feature 5: Param-set plumbing (`nexus-host`)

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T18 | `PluginInstance::set_param` + `param_text_to_value`; `ParamInfo: Serialize`; per-instance `rtrb` ParamChange ring created at `activate()` | design in STATE_TOOLS.md |
| ⬜ | T19 | CLAP: ring drain → `ParamValueEvent`s; translate façade ParamValue/Mod/Gesture* from block.events (fixes silent drop); inactive flush via `inactive_plugin_handle()` | ignored test vs Surge XT |
| ⬜ | T20 | VST3: `vst3/param_changes.rs` (IParamValueQueue/IParameterChanges, prealloc) → `ProcessData.inputParameterChanges`; normalized values on ring; `setParamNormalized` GUI sync; `getParamValueByString` | extend tests/vst3_render.rs |
| ⬜ | T21 | Params tools (list_params paged/filtered, get_params, set_params, convert_param_value) in trait + registry + debug page | |

## Feature 6: Contract growth (as investigation demands)

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T22 | Catalog/session tools: list_plugins, load_plugin | id → exact, else unique name match |
| ⬜ | T23 | Preset tools: save/load/list_preset | over existing `Preset` |
| ⬜ | T24 | read_state_bytes, delete_snapshot, + anything F3 proved necessary | |

## Feature 7: MCP frontend — last

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T25 | `crates/nexus-mcp`: rmcp 0.9.1 streamable-HTTP over the same registry (droplets bootstrap + schema sanitizer + markdown descriptions); extract registry macro only if boilerplate warrants | contract frozen only after F3–F6 |
| ⬜ | T26 | MCP E2E smoke via `claude mcp add --transport http`; docs (README + designs) | |

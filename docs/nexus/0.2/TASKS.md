# Nexus 0.2 — TASKS

Active task breakdown for **state tools** (structured access to plugin state). Design and rationale: [STATE_TOOLS.md](../../designs/STATE_TOOLS.md). See [ROADMAP.md](ROADMAP.md) for the feature list.

Status: ⬜ todo / 🔄 in progress / ✅ done.

## Feature 1: Tools core (`nexus-tools`)

The transport-agnostic trait, DTOs, registry, and main-thread bridge.

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T1 | `crates/nexus-tools` scaffold: crate, workspace entry, `ToolError { code, message }`, deps (serde, serde_json, schemars, async-trait, crossbeam-channel, futures, base64, sha2, flate2, log, nexus-host) | no tokio/axum in this crate |
| ⬜ | T2 | `NexusTools` trait core (get_session, decode_state, edit_state, get/set_state, snapshot_state/restore/list, diff_state, list_codecs) + DTOs with JsonSchema; `instance:"default"` on every request | v1 rejects instance ≠ "default" |
| ⬜ | T3 | Main-thread bridge: `ToolRequest` enum + futures oneshot replies + crossbeam channel + `drain(rx, &mut impl MainThreadHost)` + waker hook; timeouts (5s / 30s load-class) | MockHost round-trip tests, no plugins needed |
| ⬜ | T4 | Hand-written tool registry: `ToolDef { name, description, input/output schema, handler }` + dispatch; no macro in v1 | schema generation tests |
| ⬜ | T5 | `nexus-app` wiring: `tools_host.rs` implements `MainThreadHost` (state ops on the loaded `PluginInstance`), drain at top of `update()`, repaint waker | reuses existing teardown handshake |

## Feature 2: State, snapshots & codecs

| Done | # | Task | Files / notes |
|------|---|------|---------------|
| ⬜ | T6 | get_state/set_state: sha256, size, base64 opt-in with `max_bytes` cap; set_state auto-snapshots prior state | |
| ⬜ | T7 | Snapshot store (in-memory, 32 / 256 MB LRU) + on-disk corpus export (`.../Nexus/state-corpus/<plugin_id>/<name>.state` + JSON sidecar) + list/restore | corpus survives restarts; feeds codec tests |
| ⬜ | T8 | `StateCodec` trait + ordered registry + confidence selection + list_codecs | in-tree `codecs/` module |
| ⬜ | T9 | `generic` sniff codec (decode-only): gzip/zlib recurse, RIFF walk, VST3 chunk-list framing, JSON/XML detection (whole + at-offset), binary leaves (entropy/strings/hexdump); depth + 32MB caps | synthetic fixture tests |
| ⬜ | T10 | `vital-json` codec (decode+encode) | `decode(encode(doc)) == doc` round-trip tests |
| ⬜ | T11 | Shared edit application (JSON-pointer ops on the decoded doc) + `edit_state`: `base_sha256` guard, dry-run, auto-snapshot, through-the-plugin verify (load → re-save → re-decode → compare edited paths) | the TOCTOU + verification core |
| ⬜ | T12 | `diff_state(a, b=live)`: doc-level structural diff, byte-range fallback for binary leaves | synthetic doc tests |

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

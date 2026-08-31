# State tools: structured access to plugin state

**Status:** draft (drives nexus 0.2)
**Date:** 2026-08-26

## Problem

**We want AI access — ultimately over MCP — to ANY audio plugin.** That is why `nexus-host` exists: plugins (Serum 2, Vital, Arturia, …) are closed native code with no AI-facing interface of their own, but a host sits between every plugin and the world. Because nexus can load any VST3/CLAP plugin, it is the one place that can expose *any* plugin's full surface — its stored state, its parameters, its presets — to an LLM, so an AI can do what a sound designer does: "set filter 1's env routing", "what wavetable is osc A using?", regardless of vendor.

The substance of that surface is the plugin's **stored state**. Today it is an opaque blob (`save_state`/`load_state` round-trip a `Vec<u8>` and nothing more). Parameters help — they're the structured, named surface plugins already expose — but they don't cover everything a plugin stores; the interesting depth is in the blob. So the work is to turn "any plugin's state" into something an AI (and a human) can read and selectively change.

Three constraints force the design:

1. **Every plugin stores state differently.** Vital's blob is pure JSON; Arturia's is XML-ish; Serum 2's is binary chunks; most are unknown until investigated. No single parser exists or ever will — state decoding must be **extensible per plugin**, with a generic best-effort fallback for plugins nobody has studied yet.
2. **LLMs must not round-trip full blobs.** State can be megabytes. The interface has to support *partial* access: read a subtree, edit a value at a path — never "download, modify, re-upload the world".
3. **We don't yet know what the right AI-facing contract is.** The way to find out is for a human to interact with the tools manually first. So the tool surface must be **transport-agnostic**: one clean async trait, servable as REST (with a human debug UI, built first), and later MCP or gRPC — each a thin frontend, none of them the definition. MCP deliberately comes **last**, after manual investigation has shaped the contract. For the same reason, **v1 deliberately specs only a minimal core** — committing to a 20+ tool surface before the investigation would contradict the point.

## What we're building

Three pieces, in order of importance:

### 1. The investigation loop (the actual point)

The way a state format gets understood is: turn one knob in the plugin's own GUI → see what changed in the stored state. The tools exist to make that loop tight, for a human first and an AI later:

- **snapshot → tweak → diff**: take a snapshot, change something (in the plugin GUI, or later via the tools), diff the live state against the snapshot — at the **decoded-document level**, not bytes (one knob turn inside a gzip'd blob changes every compressed byte; a doc-level diff says `/settings/filter_1_cutoff: 0.42 → 0.58`). Byte-range diff remains only as the fallback for undecodable binary leaves.
- **a state corpus on disk**: snapshots can be exported (auto-named per plugin) to an on-disk corpus directory. That corpus is what codec development and codec regression tests feed on — it must survive app restarts.
- **path-scoped read/write**: read a subtree of the decoded state, apply a targeted edit at a path — the primitives the eventual MCP contract will be built from, proven by hand first.

### 2. `NexusTools` — one trait, thin frontends, minimal v1 core

A transport-agnostic async trait. v1 commits **only to the state-centric core**:

```rust
#[async_trait]
pub trait NexusTools: Send + Sync {
    async fn get_session(&self) -> Result<SessionInfo>;              // loaded plugin, sr, codec chosen
    /// Path-scoped READ: opts.path (JSON pointer into the decoded doc) + opts.depth
    /// return just a subtree — never the whole multi-MB document by accident.
    async fn decode_state(&self, source: StateSource, opts: DecodeOpts) -> Result<DecodedState>;
    /// Path-scoped WRITE — the PRIMARY write path: targeted edits at decoded paths.
    /// Guarded by a baseline hash (see Consistency) and verified through the plugin.
    async fn edit_state(&self, base_sha256: String, edits: Vec<StateEdit>, apply: bool) -> Result<EditResult>;
    async fn get_state(&self, opts: GetStateOpts) -> Result<StateBlobInfo>;   // size/sha256; base64 opt-in
    async fn set_state(&self, base64: String) -> Result<StateBlobInfo>;       // escape hatch / restore only
    async fn snapshot_state(&self, name: Option<String>, note: Option<String>, export: bool) -> Result<SnapshotInfo>;
    async fn restore_snapshot(&self, name: String) -> Result<()>;
    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>>;
    /// Doc-level structural diff (byte ranges only for binary leaves).
    /// `b` defaults to the live state — the "what did that knob change?" call.
    async fn diff_state(&self, a: String, b: Option<String>) -> Result<StateDiff>;
    async fn list_codecs(&self) -> Result<Vec<CodecInfo>>;
}
```

Likely additions once investigation demands them — sketched, **not committed**: catalog/session control (`list_plugins`, `load_plugin`), params (`list_params`/`get_params`/`set_params`/`convert_param_value` — see param plumbing below), presets (`save/load/list_preset`), `delete_snapshot`, `read_state_bytes`. Until `load_plugin` exists, the human loads plugins through the app UI as today.

All DTOs derive Serialize/Deserialize/JsonSchema. Every request carries `instance: String` defaulting to `"default"` so a multi-instance future (`nexus-wrap`) doesn't break the contract; v1 rejects other values. Errors are one `ToolError { code, message }` with actionable messages.

**The write-once seam, without premature machinery**: tools are registered in a hand-written registry (`ToolDef { name, description, input/output schema (schemars), async handler }`) that REST consumes now and MCP consumes later. No `declare_tools!` macro in v1 — with a dozen tools and one frontend, hand-written entries are cheaper than a macro; extract one when MCP becomes the second consumer and the boilerplate is real.

### 3. `StateCodec` — extensible per-plugin (de)serialization

The answer to "every plugin stores state differently":

```rust
pub trait StateCodec: Send + Sync {
    fn id(&self) -> &'static str;                        // "vital-json", "generic"
    /// Confidence this codec understands (plugin, blob): 0 = no, 255 = authored for it.
    fn confidence(&self, plugin: &PluginDescriptor, blob: &[u8]) -> u8;
    /// Blob → structured document: a JSON tree where decodable regions are real JSON/XML
    /// values and undecodable regions are {kind:"binary", path, offset, len, sha256,
    /// entropy, strings[], base64?} leaves. Lossless by construction.
    fn decode(&self, plugin: &PluginDescriptor, blob: &[u8]) -> Result<DecodedState>;
    /// Document → blob. None = decode-only codec (edits then error with an explanation).
    /// Edit application (JSON-pointer ops on the document) is SHARED code — codecs only
    /// re-serialize. A codec that can splice binary regions in place may also override
    /// `patch(original, edits)` as an optimization; the default routes through encode.
    fn encode(&self, plugin: &PluginDescriptor, doc: &DecodedState) -> Option<Result<Vec<u8>>>;
}
```

- **Registry**: ordered `Vec<Arc<dyn StateCodec>>`, highest confidence wins; `decode_state` names the codec used + confidence so readers know how much to trust it. In-tree registration (a `codecs/` module; adding a plugin = one file + one registry line); dynamic loading is a later concern.
- **v1 codecs**: (1) `generic` — fallback, decode-only: recursive sniffing (gzip/zlib → decompress+recurse, RIFF chunk walk, VST3 chunk-list framing incl. Comp/Cont entries; JSON whole-blob or embedded-at-offset; XML root + child histogram; binary leaves with entropy/printable-ratio/top-strings/hexdump windows; depth-capped, 32 MB decompressed cap). (2) `vital-json` — decode+encode, Vital's state is pure JSON. (3) More (`serum2`, `arturia`, …) as the investigation loop teaches us each format — decode-only first, encode when understood.
- **Caveat, known**: the generic codec's region paths (`/0/1`) are positional and not stable across plugin versions or even some state changes. Fine for interactive investigation; the later explorer phase needs content-anchored addressing and will extend `DecodedState` rather than replace it.

## Consistency & verification (the hard parts, stated plainly)

- **Baseline guard (TOCTOU)**: the plugin's editor is open while tools run — state can change (the user turns a knob) between a decode and an edit's apply. `edit_state` therefore takes the `base_sha256` of the blob the edits were computed against; at apply time the live state is re-saved and compared, and on mismatch the call fails with "state changed since decode" plus the new hash. No silent clobbering of concurrent changes.
- **Verification is through the plugin, not byte games**: after `load_state`, the state is re-`save_state`d, re-decoded, and the edited paths compared. Plugins normalize state on save, so byte identity is not the criterion — "the plugin accepted it and the value is there" is. Codec self-consistency (`decode(encode(doc)) == doc`) is a unit-test concern per codec, not the runtime check.
- **Undo**: every applying `edit_state`/`set_state`/`restore_snapshot` auto-snapshots the prior state first. `apply:false` is a dry-run returning the would-be result.
- **Serialization**: all plugin-touching operations funnel through the single main-thread drain (below), which serializes them — one tool call's plugin work completes before the next starts. Codec decode/encode runs off the main thread; only `save_state`/`load_state` hop.
- **`load_state` on a live plugin**: allowed by default (DAWs do it); if a plugin glitches or rejects it, fall back to the app's existing deactivate → reload → reactivate path (the preset-recall/device-change handshake already in `nexus-app`). Which plugins need the fallback is itself an investigation finding worth recording.

## Architecture

```
                    ┌── REST + web debug UI (first)   ← humans, curl
tool registry ──────┼── MCP server (last, rmcp)       ← LLMs
(one def per tool)  └── (gRPC later if wanted)
      │  async trait NexusTools
      ▼
main-thread bridge: ToolRequest{args, oneshot reply} → crossbeam → drained in
NexusApp::update() → PluginInstance / StateCodecs
      │  (later) set_param → per-instance rtrb ParamChange ring
      ▼
Clap/Vst3 AudioProcessor drains ring each block
```

Crates (nexus-host stays tokio-free; the architecture rule that it has no device I/O and no UI extends naturally to "no server runtime"):

- **`crates/nexus-tools`** — the trait, DTOs, tool registry, `StateCodec` + registry + generic & vital codecs, snapshot store + on-disk corpus export, main-thread bridge. Deps: serde, serde_json, schemars, async-trait, crossbeam-channel, futures (oneshot), base64, sha2, flate2, log, nexus-host. No tokio/axum.
- **`crates/nexus-rest`** — axum server on its own thread + tokio runtime (bootstrap copied from droplets' proven `mcp/mod.rs`: OnceLock singleton, loopback-only, dual 127.0.0.1+[::1] bind), port 9996 / `NEXUS_REST_PORT`. Routes: `GET /tools` (defs + schemas), `POST /tools/{name}`, `GET /debug`. The debug page is one inlined static HTML+JS file: tool list, a raw JSON request editor per tool (schema shown for reference — no generated forms in v1), a collapsible response tree, and one-click snapshot / diff-against-live buttons around the investigation loop. No auth (loopback), no external assets.
- (later) **`crates/nexus-mcp`** — rmcp 0.9.1 streamable-HTTP frontend over the same registry, reusing droplets' bootstrap + schema sanitizer + markdown tool descriptions.

### Main-thread bridge (in nexus-tools)

`PluginInstance` is deliberately not `Send` — all plugin calls happen on the app's main thread. `ToolRequest` (one variant per operation + `futures` oneshot reply sender) flows over a crossbeam channel, drained at the top of `NexusApp::update()` (which free-runs; a waker closure calling `egui::Context::request_repaint()` makes wake-up robust). `NexusApp` implements a small sync `MainThreadHost` trait (new `crates/nexus-app/src/tools_host.rs`); the async `NexusTools` impl marshals through it. Heavy codec work uses `spawn_blocking`-style offloading in the frontend runtime. Timeouts: 5 s default, 30 s for load-class operations. Snapshots: in-memory `Mutex<HashMap>` (32 / 256 MB, LRU) + explicit export to the corpus directory under app support (`.../Nexus/state-corpus/<plugin_id>/<name>.state` + a small JSON sidecar with sha256/note/date).

### Param-set plumbing (in nexus-host — after the debug UI, feeding the params tools)

Today `Event::ParamValue` is silently dropped by both backends and `set_param` doesn't exist. When this lands (F5): `PluginInstance::set_param(id, value)` + `param_text_to_value`; `#[derive(Serialize)]` on `ParamInfo`; delivery via a per-instance `rtrb::RingBuffer::<ParamChange>::new(1024)` created at `activate()` (producer on the instance, consumer in the `AudioProcessor` — engine and CPAL callback untouched, works for nexus-app, nexus-wrap, and headless tests). CLAP: ring drained into the input event list as `ParamValueEvent`s; also translate façade `Event::ParamValue/Mod/Gesture*` from `block.events` (fixes the silent drop — needed for nexus-wrap regardless); inactive → main-thread `inactive_plugin_handle()` + `PluginParams::flush` (APIs verified in the pinned clack checkout). VST3: new `vst3/param_changes.rs` mirroring `event_list.rs` (`HostParamQueue`/`HostParameterChanges` implementing IParamValueQueue/IParameterChanges, preallocated, wired into `ProcessData.inputParameterChanges`, ComPtr held across `process()`); the ring carries normalized values — `set_param` converts plain→normalized on the main thread and also calls `IEditController::setParamNormalized` so the plugin GUI tracks. Read-back lags ≤ ~1 audio block; `set_params` echoes the requested value with a "verify via get_params" note. Gestures deferred.

## Phasing

Tracked in @docs/ops/BOARD.md and @docs/ops/ROADMAP.md. **State first — the investigation loop starts as early as possible; param plumbing follows it.**

- **F1 Tools core** — nexus-tools: trait core, DTOs, ToolError, hand-written registry, main-thread bridge; MockHost round-trip tests.
- **F2 State, snapshots & codecs** — get/set_state, snapshot store + corpus export, `StateCodec` trait/registry, generic sniff codec (+ fixtures), vital-json codec (+ round-trip tests), doc-level diff, edit_state with baseline guard / dry-run / through-the-plugin verify / auto-snapshot.
- **F3 REST + debug UI** — nexus-rest, /tools, /debug. **Manual investigation starts here**: snapshot → GUI knob → diff-against-live, corpus building on Vital/Serum 2/Arturia. Findings reshape the contract and feed F4.
- **F4 Codec expansion** — serum2 / arturia / … decode-first, driven by F3 findings and the corpus.
- **F5 Param-set plumbing** — nexus-host both backends (details above); params tools added to the trait; headless ignored tests (Surge XT CLAP + VST3).
- **F6 Contract growth** — catalog/load_plugin, presets, read_state_bytes, whatever investigation proved necessary.
- **F7 MCP frontend — last** — nexus-mcp over the same registry (extract the registry macro here if boilerplate warrants); contract frozen only after F3–F6 experience.
- 0.3+: diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing), gRPC, gestures, multi-instance.

## Verification

- Unit: registry dispatch + schema generation; bridge round-trip against a MockHost (no plugins); codec fixtures (gzip-wrapped JSON, XML, RIFF, noise, JSON-at-offset, Vital-like round-trip); edit_state baseline-guard and dry-run logic; doc-diff on synthetic documents.
- Headless ignored tests (existing repo patterns): state round-trips already exist; F5 adds CLAP set-and-read-back vs Surge XT (`cargo test -p nexus-host -- --ignored`) and VST3 set-and-verify via extended `tests/vst3_render.rs` (`NEXUS_VST3_TEST=1`, harness=false, main-thread).
- Manual (no GUI automation; the user does visual checks): run nexus-app → open `http://127.0.0.1:9996/debug` → load Vital from the app menu → decode_state shows the JSON tree → snapshot → turn a knob in Vital's editor → diff-against-live names the changed path → edit_state that path (dry-run, then apply with the baseline hash) → change is audible and visible in the editor → restore_snapshot undoes it → export snapshot to corpus → decode_state on Serum 2 / Arturia shows chunk/XML/binary summaries.

## Risks

- Codec encode acceptance: a re-encoded blob a plugin *rejects* is the real failure mode — mitigated by through-the-plugin verification, auto-snapshot undo, and growing the corpus before attempting encode for a format.
- Plugins that normalize/rewrite state on save make doc-level diffs noisy until codecs learn which fields are volatile (timestamps, UUIDs) — codecs may mark volatile paths to exclude from diffs.
- VST3 `IParameterChanges` COM implementation (F5; mirrors the proven HostEventList/HostStream patterns; some plugins read `getParameterData` lazily — keep queues alive across `process()`).
- Main-thread stalls on load-class operations with big libraries — same as today's menu path; timeouts sized accordingly.
- Version skew: reuse droplets' known-working pins (axum 0.7, tokio 1, schemars 1, rmcp 0.9.1 when F7 arrives).

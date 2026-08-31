# Decisions

ADR-lite: small-but-binding decisions with their rationale, newest first —
things too small for a design doc but too important to live only in commit
messages or conversation. Big designs stay in docs/design/; findings in
docs/research/. One line of context each; link deeper docs where they exist.

One row, one decision, one date — so a settled question stays settled. Carries
both a clock and the jj change id (AGENTS.md §2).

| When · change id | Decision | Rationale |
|---|---|---|
| 2026-08-26 08:20 · zrpnnkzy | **REST + web debug UI built first; MCP frontend comes last** | We don't yet know the right AI contract until humans interact with tools manually. Thin frontends over a transport-agnostic `NexusTools` trait keep the core decoupled. (docs/design/STATE_TOOLS.md) |
| 2026-08-26 08:20 · zrpnnkzy | **State decoding is extensible per plugin with generic fallback** | Every plugin stores state differently (JSON, XML, binary chunks, compressed). A single universal parser cannot exist; `StateCodec` registry allows incremental plugin support. (docs/design/STATE_TOOLS.md) |
| 2026-08-26 08:20 · zrpnnkzy | **Targeted path-scoped edits instead of whole blob round-tripping** | Plugin state blobs can be megabytes; LLMs must not download and modify the entire blob. `edit_state` applies JSON-pointer edits guarded by `base_sha256`. (docs/design/STATE_TOOLS.md) |
| 2026-08-26 07:15 · smpxzzxx | **Device I/O lives in `nexus-app`, not `nexus-host`** | `nexus-host` remains UI- and I/O-agnostic so it can be embedded into `nexus-wrap` (which receives audio/MIDI from DAW host). (docs/design/ARCHITECTURE.md) |
| 2026-08-26 07:15 · smpxzzxx | **Host UI uses `egui` on `wgpu`/Metal** | Plugin editors running OpenGL clobber a GL-based host renderer; Metal avoids context conflicts on macOS. (docs/design/ARCHITECTURE.md) |
| 2026-08-26 07:15 · smpxzzxx | **Plugin editors are native child windows** | Native window embedding provides highest stability across vendor UI frameworks. (docs/design/ARCHITECTURE.md) |

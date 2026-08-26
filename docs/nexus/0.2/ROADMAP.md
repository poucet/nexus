# Nexus 0.2 Roadmap

## Summary

Nexus 0.2 is **state tools**: structured, transport-agnostic access to any loaded plugin's stored state — the substance behind the long-term goal of AI (MCP) access to ANY plugin. Full rationale and design: [STATE_TOOLS.md](../../designs/STATE_TOOLS.md).

The centerpiece is the investigation loop (snapshot → tweak → doc-level diff, with an on-disk state corpus), served by a minimal `NexusTools` trait, an extensible per-plugin `StateCodec` layer, and a REST + web debug UI for manual investigation. MCP arrives **last**, as a thin frontend over the proven tool registry.

## Feature overview

| Done | # | Feature | Notes |
|------|---|---------|-------|
| ⬜ | F1 | Tools core (`nexus-tools`: trait, DTOs, registry, main-thread bridge) | |
| ⬜ | F2 | State, snapshots & codecs (corpus export, generic + vital codecs, guarded edit_state, doc-level diff) | |
| ⬜ | F3 | REST + debug UI (`nexus-rest`) — manual investigation starts | |
| ⬜ | F4 | Codec expansion (serum2, arturia, … decode-first) | driven by F3 findings |
| ⬜ | F5 | Param-set plumbing (`nexus-host` both backends) + params tools | |
| ⬜ | F6 | Contract growth (catalog/load_plugin, presets, read_state_bytes, …) | as investigation demands |
| ⬜ | F7 | MCP frontend (`nexus-mcp`) — last | contract frozen after F3–F6 |

## Later (0.3+)

Diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing), gRPC frontend, param gestures, multi-instance, `nexus-wrap`.

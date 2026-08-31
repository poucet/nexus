# Nexus Roadmap

> What belongs here, and what does not: @AGENTS.md §2. What has already landed:
> @docs/ops/CHANGELOG.md.

Each horizon is one **story line** from @docs/ops/IDEAS.md carried to a usable
state, ordered so every phase makes the next cheaper. Phases are themes, not
gates: a row can be pulled forward whenever it's wanted — say its `LABEL` if it
is on the board, otherwise ask and it gets specced.

> Status: 🔄 in flight · ⏸️ started then paused · 📋 specced & queued on
> @docs/ops/BOARD.md · 🔶 partly shipped (the account is in the changelog; what
> is left is in the row) · 🎯 next up · ⬜ open, not scheduled anywhere.
> Sizes: S ≈ a session · M ≈ a lane · L ≈ a lane plus design.

## `v0.2.0` — State tools

Structured, transport-agnostic access to any loaded plugin's stored state — the
substance behind AI (MCP) access to ANY plugin. Design: @docs/design/STATE_TOOLS.md.

The centerpiece is the investigation loop (snapshot → tweak → doc-level diff, with an on-disk state corpus), served by a minimal `NexusTools` trait, an extensible per-plugin `StateCodec` layer, and a REST + web debug UI for manual investigation. MCP arrives **last**, as a thin frontend over the proven tool registry.

| St | Size | Item | Notes |
|---|---|---|---|
| 🔶 | M | `F3` REST + debug UI (`nexus-rest`) | Server + `/debug` UI landed; manual investigation pass (`INVESTIGATE`) open |
| ⬜ | S | `F4` Codec expansion | `serum2`, `arturia`, … decode-first, driven by F3 findings |
| ⬜ | M | `F5` Param-set plumbing | `nexus-host` both backends + params tools (`list/get/set_params`, `convert_param_value`) |
| ⬜ | S | `F6` Contract growth | `list_plugins`, `load_plugin`, presets, `read_state_bytes` as investigation demands |
| ⬜ | M | `F7` MCP frontend (`nexus-mcp`) | Thin rmcp frontend; contract frozen after F3–F6 |

## Later (`v0.3.0`+)

- Diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing)
- `nexus-wrap`: ship the host as a CLAP/VST3 plugin, forwarding the full extension surface to the inner plugin (load any wrapped plugin inside a DAW)
- Full VST3-host extension parity (`IPlugFrame::resizeView`, units, note-expression)
- CLAP long-tail extensions
- Cross-platform support (Windows/Linux discovery paths + editor windows)
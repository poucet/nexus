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

## `v0.6.0` — Headless Sampling & Extension Parity
 
Fast offline sampling harness (`AI_SAMPLER_MCP.md`), external agent verification, VST3 wrapper, and plugin extension parity. (Downstream audio perception & patch reconstruction specs live in `../sense`.)

| St | Size | Item | Notes |
|---|---|---|---|
| 📋 | S | `MCP-SMOKE` External MCP Verification | End-to-end verification with Claude Desktop / Cursor (`claude mcp add --transport http` on port 9997) |
| 📋 | M | `INVESTIGATE` & Codec Expansion | Manual investigation pass (`INVESTIGATE`), `serum2`, `arturia` binary codecs |
| ⬜ | M | `WRAP-VST3` VST3 Wrapper Target | VST3 wrapper class factory alongside existing CLAP wrapper |
| ⬜ | M | Full VST3/CLAP extension parity | `IPlugFrame::resizeView`, note expression, units, CLAP presets/discovery |

## Later (`v0.7.0`+)

- Diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing)
- Windows cross-platform support
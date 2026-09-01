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

## `v0.4.0` — AI Patch Reconstruction & State Exploration

Acoustic DNA extraction, Bayesian/differential parameter search, Flux modular synthesis bridge, and deep codec exploration.

| St | Size | Item | Notes |
|---|---|---|---|
| 📋 | L | `AI-RECONSTRUCT` AI Patch Reconstruction | Acoustic feature extraction (`simply-signal`) + Flux bridge per `AI_PATCH_RECONSTRUCTION_TECH.md` |
| 📋 | M | `INVESTIGATE` & Codec Expansion | Manual investigation pass (`INVESTIGATE`), `serum2`, `arturia` binary codecs |
| ⬜ | M | `WRAP-VST3` VST3 Wrapper Target | VST3 wrapper class factory alongside existing CLAP wrapper |
| ⬜ | M | Full VST3/CLAP extension parity | `IPlugFrame::resizeView`, note expression, units, CLAP presets/discovery |

## Later (`v0.5.0`+)

- Diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing)
- Windows cross-platform support
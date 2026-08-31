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

## `v0.3.0` — State Exploration & Plugin Wrapper

Deep parameter-to-state diff probing, codec expansions, and hosting wrapper.

| St | Size | Item | Notes |
|---|---|---|---|
| 📋 | M | `INVESTIGATE` & Codec Expansion | Manual investigation pass (`INVESTIGATE`), `serum2`, `arturia` codecs |
| 📋 | M | `PARAM-HOST` Realtime Set | Realtime parameter modulation ring for CLAP/VST3 audio processing |
| ⬜ | L | `nexus-wrap` Plugin Wrapper | Ship host as CLAP/VST3 plugin forwarding extensions to inner plugins |
| ⬜ | M | Full VST3/CLAP extension parity | `IPlugFrame::resizeView`, note expression, units, CLAP presets/discovery |

## Later (`v0.4.0`+)

- Diff-probing explorer (set param → snapshot → diff → bytes↔params mapping, content-anchored region addressing)
- Cross-platform support (Windows/Linux discovery paths + editor windows)
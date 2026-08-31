# Plugin extension parity

**Status:** planned (drives nexus 0.1 Phase 01 CLAP work; goal for `nexus-wrap`)
**Date:** 2026-06-03

## Goal

Support **every aspect of a plugin** — not just audio. Two consumers need the
full surface, and they need the *same* surface:

- **`nexus-host`** must let a host *drive* every aspect of a hosted plugin
  (params, state, audio/MIDI I/O, GUI, latency, …).
- **`nexus-wrap`** must *expose and forward* every aspect of an inner plugin to
  the outer DAW, so a wrapped plugin is indistinguishable from running it
  directly.

## Design principle: one surface, two consumers

The [`PluginInstance`](../../crates/nexus-host/src/host.rs) trait is the single
abstraction for "everything a plugin can do." It grows one capability per aspect
below. Then:

- the **egui host app** consumes `PluginInstance` to show params, save state,
  open the editor, etc.;
- **`nexus-wrap`** implements each CLAP plugin-side extension by **delegating to
  the inner `PluginInstance`** — params forward 1:1, our `state` blob embeds the
  inner state, our editor forwards the inner editor, and so on.

No aspect is implemented twice. VST3 is not implemented separately on the wrap
side: `nexus-wrap` is a CLAP plugin and gets VST3 via **`clap-wrapper`**, which
translates these extensions.

## The full surface (CLAP)

`P01` = implement in Phase 01 (host side). `P02` = full-parity follow-up / wrap
side. Niche extensions may start as minimal/stub implementations.

### Audio I/O
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `audio-ports` | read port layout; route buffers | mirror inner layout outward | P01 |
| `audio-ports-config` / `configurable-audio-ports` | pick/apply a config | forward inner configs | P01 |
| `surround`, `ambisonic` | channel maps | forward | P02 |

### MIDI / Note I/O
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `note-ports` | send/receive notes + MIDI 1.0/2.0 | mirror inner note ports | P01 |
| `note-name` | read named notes (drum maps) | forward | P01 |

### Parameters
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `params` | enumerate; get/set; value↔text; flush; handle rescan/clear/request-flush | forward inner params 1:1 | P01 |
| `param-indication` | render automation/mod indication | forward | P02 |
| `remote-controls` | expose control pages | forward | P02 |

### State
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `state` | save/load chunk | embed inner state inside ours | P01 |
| `state-context` | context-aware save (preset vs project vs duplicate) | forward | P01 |
| `preset-load` | load a preset file/location | forward | P02 |

### GUI
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `gui` | embed editor in a native child window; size negotiation | forward inner editor | P01 (see [ARCHITECTURE.md](ARCHITECTURE.md), tasks T20–T22) |
| `context-menu` | populate/route plugin context menus | forward | P02 |

### Timing, voices, render
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| transport (via `clap_process`) | provide tempo/playhead | pass outer transport in | P01 |
| `latency` | read + report to the graph | report inner latency | P01 |
| `tail` | read release tail length | forward | P01 |
| `render` | set realtime vs offline | forward | P01 |
| `voice-info` | read polyphony info | forward | P01 |

### Host-provided infrastructure
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `log` | receive plugin logs | re-host for inner | P01 |
| `thread-check` | answer is-main/is-audio-thread | re-host | P01 |
| `timer-support` | run plugin timers | re-host | P01 |
| `posix-fd-support` (Linux) | poll plugin fds | re-host | P02 (macOS MVP) |
| `thread-pool` | run plugin tasks | re-host | P02 |
| `event-registry` | resolve custom event spaces | re-host | P02 |

### DAW context
| Extension | Host side | Wrap side | When |
|---|---|---|---|
| `track-info` | provide track name/color/channel | pass outer track info in | P01 (minimal standalone), full on wrap |

## How this maps onto `PluginInstance`

Each aspect adds to the trait, e.g.:

- audio/note I/O → `audio_ports()`, `note_ports()` descriptors + the existing
  `process(ProcessBlock)`;
- params → `params() -> &[ParamInfo]`, `param_value`, `set_param`,
  `param_value_to_text`, `flush_params`;
- state → `save_state(&mut Write)`, `load_state(&mut Read)`;
- latency/tail/voice-info → `latency()`, `tail()`, `voice_info()`;
- gui → `editor()` (already present).

CLAP and VST3 backends implement these against their native extensions; the egui
app and `nexus-wrap` consume them. The trait is the contract — keep new methods
format-agnostic.

## VST3 note

VST3 *hosting* (`nexus-host`, Feature 4) mirrors the same aspects via the VST3
interfaces (`IEditController` for params, `IComponent` get/setState for state,
bus arrangements for audio, `IMidiMapping`/event lists for MIDI, `IPlugView` for
GUI). Phase 01 VST3 covers load/process/state/GUI; the long tail of VST3-host
parity is Phase 02. `nexus-wrap` does **not** touch VST3 directly — `clap-wrapper`
provides the VST3 face of our CLAP implementation.

## Known VST3 → CLAP limitations

The façade is **CLAP-faithful** (CLAP is a near-superset of VST3 on the
functional surface), so a few VST3-only concepts flatten when a VST3 is
expressed through it. None affect a plugin's audible function — its full
internal state round-trips via the opaque state chunk; only host-facing
*enumeration/organization* is reduced:

- **Program lists / units** (`IUnitInfo`, `IProgramListData`) — hierarchical
  unit trees and factory program enumeration have no direct CLAP equivalent
  (CLAP uses flat param modules + file-based presets). *Biggest mismatch.*
- **Custom note-expression types** — VST3 permits arbitrary note-expression
  typeIDs; CLAP has a fixed set (volume/pan/tuning/vibrato/expression/
  brightness/pressure).
- **Some param flags** — `kIsProgramChange`, `kIsList` (discrete string-list
  params) lack exact CLAP equivalents.
- **`IMidiMapping`** (CC→param hints) and a few **bus/speaker arrangement**
  negotiation edge cases.

For the **wrap** path (VST3-in-CLAP via `nexus-wrap`) this loss is *inherent* —
the output format is CLAP, and `clap-wrapper` hits the mirror image going the
other way. For the **direct-host** path (the egui app hosting a VST3), we may
add an *optional, VST3-only* façade extension (e.g. program lists) when VST3
hosting lands (Phase 02); it would sit alongside the CLAP-faithful core, not
replace it. Decision deferred to VST3-hosting time.

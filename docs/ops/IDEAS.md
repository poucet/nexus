# Ideas

An inbox: raw, half-formed, uncommitted. An idea only ever moves down; ids are
stable and never reused.

> What belongs here, and what does not: @AGENTS.md §2.
>
> Status: 💡 open · 🔶 partly answered by something that shipped · 🎯 next up ·
> 📋 specced and queued · 🔄 in flight · 🔬 being researched · ✅ shipped ·
> 🗑️ answered by deciding not to.

## Inbox
| Status | Added | # | Idea | Notes |
|---|---|---|---|---|
| 💡 | 2026-08-31 | 1 | Diff-probing explorer for parameter↔byte mapping | Automate knob tweak → diff loop to map binary offsets to named parameters |
| 💡 | 2026-08-31 | 2 | `nexus-wrap` plugin wrapper (VST3/CLAP hosting inside DAW) | Wrap any plugin to expose MCP state control inside a DAW session |
| 💡 | 2026-08-31 | 3 | Sandboxed / out-of-process plugin probing | Isolate crashing plugin initializers so host app never crashes |
| 💡 | 2026-08-31 | 4 | Windows and Linux host support | Add discovery paths and platform native window handling for Win/Linux |
| 💡 | 2026-08-31 | 5 | gRPC frontend over `NexusTools` | Low-latency binary RPC frontend alongside REST and MCP |
## In-flight
| Status | Added | # | Idea | Notes |
|---|---|---|---|---|
| 🔄 | 2026-08-26 | 0.2 | State tools: structured plugin state access & codecs | In-flight milestone on BOARD.md |

## Done
| Status | Added | # | Idea | Notes |
|---|---|---|---|---|
| ✅ | 2026-08-31 | 6 | AI plugin sampling & offline audio rendering tool suite | Driver-free offline 32-bit float WAV render with Droplets fugue note/CC scheduling, catalog/preset/param tools, and standalone MCP server |
| ✅ | 2026-06-01 | 0.1 | Format-agnostic VST3/CLAP hosting + standalone host app | Shipped in v0.1.0 |

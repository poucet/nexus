# Issues

> What belongs here, and what does not: @AGENTS.md §2.
> 🐞 open · 🔬 triaged (cause known, fix specced) · 🔄 being fixed ·
> ✅ fixed · 🗑️ won't fix / not a bug. Sizes: S ≈ a session · M ≈ a lane.

| Status | Found | What | Verdict |
|---|---|---|---|
| ✅ | 2026-09-04 | **`render_audio`'s `Log` curve was `√t`** where flux and droplets use `1 − (1 − t)²` (flux `I-49`) | Fixed in `FUGUENEXUS-2` · `vwwnyskw` — CHANGELOG |
| ✅ | 2026-09-04 | **`render_offline` discarded `Cc.interpolation`**; every CC lane was a staircase (flux `I-50`) | Fixed in `FUGUENEXUS-2` · `vwwnyskw` — CHANGELOG |
| ✅ | 2026-09-04 | **`render.rs`'s `_ => {}` swallowed pitch bend, pressure, poly pressure and both per-note kinds**, and `warnings` was never filled (flux `I-48`) | Fixed in `FUGUENEXUS-2` · `vwwnyskw`, `wzsyxqru` — CHANGELOG |
| ✅ | 2026-09-04 | **No release tail**: a note ending on the last beat lost its decay and the tail's absence skewed peak/RMS (flux `I-51`) | Fixed in `FUGUENEXUS-2`: a fixed 1.0 s tail · `vwwnyskw` — CHANGELOG |
| ✅ | 2026-09-04 | **`loop_mode` was ignored offline** (flux `I-52`) | Fixed in `FUGUENEXUS-2` · `vwwnyskw`, `wvyzxrsm` — CHANGELOG |
| ✅ | 2026-09-04 | **No note-off-before-note-on tie-break** at the same sample (flux `I-54`, nexus side) | Fixed in `FUGUENEXUS-2` · `vwwnyskw` — CHANGELOG |
| ✅ | 2026-09-04 | **`docs/design/AI_SAMPLER_MCP.md` described a `render_audio` that does not exist** (flux `I-47`) | Corrected to `RenderAudioRequest`/`Response` in `FUGUENEXUS-2` · `tswpsmmv` — CHANGELOG |
| 🐞 | 2026-09-04 | **The VST3 backend delivers note on/off only.** `vst3/process.rs`'s `to_vst_event` drops every other raw-MIDI status (CC, pitch bend, channel and poly pressure) and builds no `kNoteExpressionValueEvent`, though the bindings have the type. `render_offline` now *says* so in `warnings` (`PluginInstance::event_support`), but a fugue's CC sweeps and expression are inaudible through a VST3 plugin. Fix is a VST3 `IMidiMapping` / note-expression path; sized M. Found by `FUGUENEXUS-2`. | |
| 🐞 | 2026-09-04 | **A parameter change reaches a CLAP plugin twice per point.** `render_offline` pushes each `ParamChange` into the rtrb ring *and* as `Event::ParamValue` into the block's event buffer, and `clap/process.rs` turns both into a `ParamValueEvent` at the same frame. Harmless in value, doubles the event count; pre-existing. XS. Found by `FUGUENEXUS-2`. | |
| 🐞 | 2026-09-04 | **A `params` lane is re-sent every 64 frames to the end of the render even while held** — 2250 `ParamValue`s for a one-beat, two-point lane in a 3 s render — where a CC lane suppresses repeats (`last_cc`). Pre-existing; the same held-value rule would fit. XS. Found by `FUGUENEXUS-2`. | |
| 🐞 | 2026-09-04 | **No ceiling on a render's musical length** where flux refuses past 60 s: a 10-minute fugue renders ~230 MB of float WAV, inline if `wav_base64` was asked for. Stated as an asymmetry in `AI_SAMPLER_MCP.md`; a shared ceiling is a policy question. XS. Found by `FUGUENEXUS-2`. | |
| 🐞 | 2026-09-04 | **Stale tool prose**: the `play` tool description still says "Droplets Fugue format" and `nexus-mcp/src/instructions.md:24` says "base64 or file path" without naming the `file` default. XS. Found by `FUGUENEXUS-2`. | |

# `render_audio` before and after `FUGUENEXUS-2` — the 12 shared fixtures

> Measurement for the behaviour half of FUGUE-API.md §6 (the design lives in
> flux). Filed 2026-09-04 23:19 · `tswpsmmv`. The account is in
> @docs/ops/CHANGELOG.md under `FUGUENEXUS-2`.

**Method.** Each of the 12 `simply-fugue` wire-compat fixtures
(`crates/simply-fugue/tests/fixtures/*.json` at `bd774ec2` — the set that
pins "every fugue JSON nexus accepts still parses") was compiled and rendered
by `render_offline` at 48 kHz, 120 BPM, block 256, through the two dummy
instances in `crates/nexus-host/src/render.rs`'s tests — the sine
`DummyOscillatorInstance` (ignores every event; 0.5 amplitude) and
`DummyParamInstance` (its gain is parameter `1`, read from the `ParamChange`
ring) — plus a counter of what reached the plugin's event buffer, as
`Note / Midi / ParamValue`. Once on trunk (`qmywlvrl`), once on the lane's tip.
The 60-line harness was appended to the test module for the run and not
committed.

| fixture | frames before → after | osc peak / rms (dB) | param peak / rms (dB) before → after | events note / midi / param before → after |
|---|---|---|---|---|
| `array_shorthand_notes` | 48000 → 96000 | −6.02 / −9.03 | silent | 5/0/0 → 6/0/0 |
| `cc_lane` | 96000 → 144000 | −6.02 / −9.03 | silent | 0/1/0 → 0/91/0 |
| `empty` | 24000 → 72000 | −6.02 / −9.03 | silent | 0/0/0 → 0/0/0 |
| `instructions_chord` | 48000 → 96000 | −6.02 / −9.03 | silent | 3/0/0 → 6/0/0 |
| `points_with_curves` | 48000 → 96000 | −6.02 / −9.03 | silent | 1/2/0 → 2/91/0 |
| `raw_events_all_variants` | 96000 → 144000 | −6.02 / −9.03 | silent | 4/2/1406 → 4/5/2906 |
| `render_param_curve` | 24000 → 72000 | −6.02 / −9.03 | −1.96 / −6.68 → −1.94 / −3.02 | 0/0/375 → 0/0/1125 |
| `render_sine` | 48000 → 96000 | −6.02 / −9.03 | silent | 0/0/0 → 0/0/0 |
| `sample_vital` | 240000 → 288000 | −6.02 / −9.03 | silent | 12/0/0 → 12/0/0 |
| `sample_vital_clap` | 240000 → 288000 | −6.02 / −9.03 | silent | 6/0/0 → 6/0/0 |
| `sampler_doc_cc_points` | 96000 → 144000 | −6.02 / −9.03 | silent | 1/3/1500 → 2/156/2250 |
| `service_smoke` | 24000 → 72000 | −6.02 / −9.03 | silent | 1/0/0 → 2/0/0 |

"silent" is −120.00 / −120.00: the param dummy hears only parameter `1`, which
only `render_param_curve` drives. The oscillator's pair is the same on both
revisions for every fixture because it hears no event, and a sine's peak and
RMS do not depend on its length.

**What moved each number.**

- **`frames`, +48000 on every fixture** — the 1.0 s tail. `empty`'s 24000 is
  the crate's one-beat floor on `duration_beats`.
- **`render_param_curve` RMS −6.68 → −3.02, peak −1.96 → −1.94** — the tail.
  The lane holds its final value (0.8) through the extra second, and its last
  keyframe (beat 1 = frame 24000) now lands *inside* the render; before, the
  last delivered point was the grid sample at frame 23936.
- **Note events +1** on `array_shorthand_notes`, `points_with_curves`,
  `sampler_doc_cc_points`, `service_smoke`, **+3** on `instructions_chord` —
  the tail. A note-off at exactly `duration_beats` sat at `total_frames` and
  was never delivered: `I-51`'s truncation, counted. `sample_vital` and
  `sample_vital_clap` end their last note before `duration_beats` and are
  unchanged.
- **MIDI events**: `cc_lane` 1 → 91, `points_with_curves` 2 → 91,
  `sampler_doc_cc_points` 3 → 156 — `Cc.interpolation` honoured (`I-50`): a
  30 → 120 lane is 91 distinct bytes, a curve becomes exactly that ramp. (`cc_lane`'s
  second point sits at beat 4 = `total_frames` and was also lost before the
  tail, hence 1.) `raw_events_all_variants` 2 → 5 — pitch bend, pressure and
  poly pressure delivered (`I-48`). Its per-note lanes are none, so the
  expression column is 0 → 0 and is omitted.
- **`ParamValue` counts** grow with the tail because a `params` lane is
  re-sampled every 64 frames to the end of the render whether or not it is
  held — pre-existing, filed in ISSUES.
- **`Log` moves none of the 12.** No fixture starts a segment with `log` on a
  lane a dummy hears: `cc_lane` and `sampler_doc_cc_points` put `log` on their
  *last* point, which starts no segment. `Log(0.5) = 0.75` (not `√0.5 = 0.707`)
  is pinned by `log_is_the_majority_curve_and_the_others_are_unchanged`.
- **`loop_mode` and the tie-break move none** — every fixture is `once` and
  none retriggers a pitch at a boundary. Both are pinned by tests
  (`times_renders_every_pass`, `a_same_pitch_retrigger_delivers_the_off_before_the_on`).

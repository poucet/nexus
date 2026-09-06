# Nexus MCP Server

Nexus allows AI agents to discover, load, configure, perform with, and sample arbitrary VST3 and CLAP audio plugins.

## Core Capabilities

1. **Discovery & Loading**:
   - `list_plugins`: Find installed instruments and effects on the host.
   - `load_plugin`: Load a plugin into the session by ID or name (e.g. `"Vital"`, `"audio.vital.synth"`).
   - `get_session`: Inspect current session state and loaded plugin.

2. **Presets & Saved States**:
   - `list_presets`: List available `.nexuspreset` files.
   - `load_preset`: Load a preset file into the active plugin.
   - `save_preset`: Save the current sound as a `.nexuspreset`.

3. **Parameters**:
   - `list_params`: Query parameter IDs, names, and ranges.
   - `get_params`: Inspect current parameter values.
   - `set_params`: Set parameter values directly.

4. **Fugue Musical Performance & Sampling**:
   - `play`: Send a transport-synchronized performance (notes, CCs, automation ramps) to the live host.
   - `render_audio`: Render audio offline at max CPU speed without an audio driver. Returns audio metrics (`peak_db`, `rms_db`, `duration_s`) and 32-bit float WAV data (defaults to file, or wav_base64) for listening and patch analysis.

5. **State Decoding & Structural Edits**:
   - `decode_state`: Decode the stored binary state into a structured JSON document.
   - `edit_state`: Apply targeted JSON-pointer edits to the decoded state and reload it into the plugin.
   - `snapshot_state` & `diff_state`: Capture snapshots and inspect what parameters or binary offsets change when controls move.

## Fugue Note Shorthand

When scheduling notes in `play` or `render_audio`, use the compact array format:
`[beat, note, duration, velocity, channel]`
Examples:
- `[0, "C3", 2.0, 100]` -> C3 (MIDI 60) at beat 0 for 2 beats at velocity 100.
- `[0, "E3", 2.0, 90]` -> E3 (MIDI 64) forming a chord with C3.
- `[0, "G3", 2.0, 95]` -> G3 (MIDI 67).

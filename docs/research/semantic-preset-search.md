# Semantic preset search for hosted plugins

**Summary:** search a plugin's presets by text ("dark evolving pad") or by reference audio, using SynthScribe-style CLAP embeddings and cosine similarity. It works for any VST3 or CLAP plugin Nexus can load, not one synth.

**Source:** audio.dev talk. Reference: SynthScribe, arXiv:2312.04690 (Brade et al., IUI 2024).

**Why Nexus fits:** it already has headless `load_plugin` and `load_preset` for any VST3 or CLAP plugin, offline `render_audio` at full CPU speed with no audio driver, and state snapshots (`snapshot_state`, `diff_state`) for preset identity.

## Proposed design
1. **Probe.** For each preset, render the same short MIDI clip: a few notes at different pitches and velocities, plus a held note for envelope and modulation (about 4 to 6 s at 48 kHz).
2. **Embed.** Run the render through LAION-CLAP (resampled to 48 kHz), via a Python sidecar first, then ONNX Runtime (`ort`) in Rust.
3. **Index.** One vector per (plugin id, preset, state hash) in a flat file or SQLite with brute-force cosine search. Re-index only when the state hash changes.
4. **Query.** Text goes through the CLAP text encoder; audio or another preset through the audio encoder. Rank by cosine similarity, optionally filtered by plugin.
5. **MCP tools.** `index_presets(plugin)`, `search_presets(text | audio | preset, k)`, `similar_presets(preset)`.

## Later phases
- **Mixing (SynthScribe).** `mix_presets([a, b, c], n)`: crossover and mutation on normalized parameter vectors via `set_params`, then render, embed and index. Drop near-duplicates by cosine similarity.
- **Edit hints (SynthScribe).** `suggest_edit(preset, "brighter")`: perturb each parameter group, re-render, and rank groups by how much they move the similarity toward the target.
- **Text to preset (CTAG, arXiv:2406.00294).** Gradient-free search (e.g. CMA-ES) over parameters with a CLAP score, starting from the top-k retrieved presets.
- **Learned joint embedding.** Parameters and audio in one space, for retrieval without rendering and for audio-to-preset matching (Braun & Finkelstein, arXiv:2608.18226).
- **Timbre-tuned CLAP.** Fine-tune on synth renders with captions built from preset names, categories and tags.

## First experiment
Render the roughly 2,000 categorized Surge XT factory presets with `nexus-mcp`, embed them with `laion_clap`, and measure category recall@k for text queries such as "bass", "pad" and "lead", plus preset-to-preset nearest neighbours.

## Risks
- **Granularity.** CLAP is good at category and vibe, and weaker on subtle timbre (ELSA, arXiv:2606.17404).
- **Probe bias.** One probe misrepresents arpeggios, drones and FX presets. Consider per-category probes.
- **Effects.** They need input audio. Embed the processed output of a fixed dry stimulus, or the difference from the dry embedding.
- **Non-determinism.** Random LFO phases and analog drift. Fix seeds where possible, or average two renders.
- **Licensing.** Storing embeddings is likely fine. Don't redistribute renders of commercial presets.

## Related work
- SynthScribe, arXiv:2312.04690
- FM Synthesizer Audio-Parameter Shared Embeddings, arXiv:2608.18226 (DAFx 2026)
- AutoSynth, arXiv:2610.10774 (2026): text or audio to editable synth programs
- CTAG, arXiv:2406.00294 (ICML 2024)
- Synthesizer Sound Matching Using Audio Spectrogram Transformers, arXiv:2407.16643 (DAFx 2024)
- DiffMoog, arXiv:2401.12570: differentiable synth, so not black-box
- ELSA, arXiv:2606.17404 (Interspeech 2026): caveat on CLAP granularity

Full brief in the knowledge folder: `research/nexus-semantic-preset-search.md`.

**Labels:** enhancement

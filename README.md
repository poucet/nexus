# Nexus

A Rust-based **VST3 / CLAP plugin host**, growing into an AI-first DAW.

Nexus is built in layers. The foundation is a format-agnostic hosting engine
that loads and runs VST3 and CLAP plugins behind a single interface. That engine
is then reused to build a plugin-that-hosts-plugins, and ultimately a full
digital audio workstation.

## Goals

- **One interface, two formats.** Callers should never branch on VST3 vs. CLAP —
  the host abstracts the difference away.
- **Reusable core.** The hosting engine is a library that every higher layer
  depends on, so there is exactly one implementation of plugin loading and
  processing.
- **AI-first.** The eventual DAW treats AI assistance as a primary part of the
  workflow rather than a bolted-on feature.

## Workspace layout

This is a Cargo workspace. Each crate is a layer on top of the one before it:

| Crate | Kind | Purpose |
| --- | --- | --- |
| [`nexus-host`](crates/nexus-host) | library | The engine that discovers, loads, and runs VST3/CLAP plugins. No device I/O. |
| [`nexus-app`](crates/nexus-app) | binary (`nexus-app`) | Standalone host app: egui UI, CPAL audio out, MIDI in, plugin browser + editor windows. |
| [`nexus-wrap`](crates/nexus-wrap) | cdylib + lib | A VST3/CLAP plugin that itself hosts another plugin — load any wrapped plugin inside a DAW. |
| [`nexus-daw`](crates/nexus-daw) | binary (`nexus`) | The AI-first DAW application shell (stub for now). |

```
nexus/
├── Cargo.toml            # workspace manifest (shared deps + metadata)
├── .cargo/config.toml    # cargo aliases for the bundler (cargo bundle, …)
├── xtask/                # build helper: compiles + bundles the plugin
└── crates/
    ├── nexus-host/       # hosting engine (the foundation)
    ├── nexus-app/        # standalone host app (the 0.1 deliverable)
    ├── nexus-wrap/       # plugin that wraps/hosts another plugin
    └── nexus-daw/        # the DAW application
```

> **Status:** 0.1 "Load & Hear" works on macOS — `nexus-app` scans the standard
> plugin folders, loads CLAP and VST3 plugins, plays them through the default
> audio output (hardware MIDI or the built-in virtual keyboard), opens their
> editors, and saves/recalls presets. `nexus-wrap` and `nexus-daw` are stubs.

## Quickstart

Requires a recent stable Rust toolchain (edition 2024, `rustc` ≥ 1.90).
macOS only for now.

```sh
cargo run --bin nexus-app
```

- Plugins are discovered in the standard folders
  (`~/Library/Audio/Plug-Ins/{CLAP,VST3}` and the `/Library` equivalents);
  add extra search paths in `~/Library/Application Support/Nexus/config.toml`
  (`[plugins] clap = [...] vst3 = [...]`).
- Load a plugin from the **Plugin** menu (grouped by vendor) or the searchable
  finder palette (**⌘P**). Its editor opens in its own window.
- Play with a connected MIDI device or the on-screen keyboard
  (keys `A W S E D F T G Y H U J K`, octave shift).
- **State ▸ Save/Load** captures a `.nexuspreset` (plugin identity + its full
  state); **Settings…** picks the audio output device.
- The first launch probes every plugin bundle; results are cached, so later
  launches only re-probe bundles that changed.

```sh
cargo build   # build everything
cargo test    # run tests
```

### Bundling the plugin

Plugin packaging is handled by the `xtask` helper, driven through cargo
aliases (this mirrors the sibling `droplets` project):

```sh
cargo bundle            # build + bundle CLAP and VST3 for this platform
cargo bundle-clap       # CLAP only
cargo bundle-vst3       # VST3 only
cargo bundle-debug      # debug profile instead of release
cargo install-plugin    # build, bundle, and install into the user plugin dirs
```

Bundles are written to `target/bundle/` and installed to the standard
per-user locations (`~/Library/Audio/Plug-Ins/{CLAP,VST3}` on macOS, `~/.clap`
and `~/.vst3` on Linux). The bundle name and identifiers come from a single
`[package.metadata.bundle]` table in `crates/nexus-wrap/Cargo.toml`, read at
bundle time via `cargo metadata` — there is no separate bundler config.

Building VST3 requires the Steinberg VST3 SDK; point `CLAP_WRAPPER_VST3_SDK`
at your SDK checkout. The CLAP build needs no extra setup.

## Planned approach

Nexus leans on the Rust audio-plugin ecosystem rather than reinventing it, and
follows the same stack proven in the sibling `droplets` project:

- **CLAP, host + plugin** — [`clack`](https://github.com/prokopyl/clack) for
  safe CLAP bindings. `clack` covers both sides, so `nexus-host` uses its host
  API and `nexus-wrap` will use its plugin API.
- **VST3 hosting** — the pure-Rust [`vst3`](https://crates.io/crates/vst3-sdk)
  COM bindings (no Steinberg SDK checkout needed to *host*).
- **VST3 export** — [`clap-wrapper`](https://github.com/blepfx/clap-wrapper-rs)
  will wrap the CLAP `nexus-wrap` plugin as a VST3, so a single implementation
  ships in both formats.
- **Bundling** — a custom `xtask` builds the cdylib and lays it out into the
  platform-specific `.clap`/`.vst3` bundle structures.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

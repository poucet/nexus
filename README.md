# Nexus

A Rust-native **VST3 / CLAP plugin host**: a hosting-engine library and a
standalone app that scans, loads, plays, and displays audio plugins.

Nexus is built in layers. The foundation is a format-agnostic hosting engine
that loads and runs VST3 and CLAP plugins behind a single interface. A
standalone app drives that engine today; the next layer reuses it as a
plugin-that-hosts-plugins.

## Goals

- **One interface, two formats.** Callers should never branch on VST3 vs. CLAP —
  the host abstracts the difference away.
- **Reusable core.** The hosting engine is a library that every layer depends
  on, so there is exactly one implementation of plugin loading and processing.
- **Wrap anything.** The engine will ship as `nexus-wrap`, a plugin that hosts
  another plugin, so anything Nexus can load can run — wrapped — inside any DAW.
- **Programmable state (eventually).** A longer-term direction is an MCP
  interface for inspecting and controlling a wrapped plugin's saved state.

## Workspace layout

This is a Cargo workspace. Each crate is a layer on top of the one before it:

| Crate | Kind | Purpose |
| --- | --- | --- |
| [`nexus-host`](crates/nexus-host) | library | The engine that discovers, loads, and runs VST3/CLAP plugins. Fast offline rendering without audio drivers. |
| [`simply-fugue`](crates/simply-fugue) | library | Shared Fugue musical definition, compact note shorthand, and pitch parsing (`C3` = 60). |
| [`nexus-tools`](crates/nexus-tools) | library | Transport-agnostic tool traits, registry, state codecs, snapshot store, and main-thread bridge. |
| [`nexus-mcp`](crates/nexus-mcp) | binary (`nexus-mcp`) | Standalone MCP server exposing all tools to AI coding agents via stdio or streamable HTTP. |
| [`nexus-rest`](crates/nexus-rest) | library | In-process REST API server and browser-based debug UI (`http://127.0.0.1:9996/debug`). |
| [`nexus-app`](crates/nexus-app) | binary (`nexus-app`) | Standalone GUI host app: egui UI, CPAL audio out, MIDI in, plugin browser + editor windows. |
| [`nexus-wrap`](crates/nexus-wrap) | cdylib + lib | A VST3/CLAP plugin that itself hosts another plugin — load any wrapped plugin inside a DAW. |

```
nexus/
├── Cargo.toml            # workspace manifest (shared deps + metadata)
├── .cargo/config.toml    # cargo aliases for the bundler (cargo bundle, …)
├── xtask/                # build helper: compiles + bundles the plugin
└── crates/
    ├── simply-fugue/     # canonical fugue musical performance schema
    ├── nexus-host/       # hosting & offline rendering engine (the foundation)
    ├── nexus-tools/      # tool definitions, state codecs & snapshots
    ├── nexus-mcp/        # standalone MCP server for AI agents (stdio / HTTP)
    ├── nexus-rest/       # in-process REST server & debug UI
    ├── nexus-app/        # standalone GUI host app
    └── nexus-wrap/       # plugin that wraps/hosts another plugin
```

---

## Running for AI & Headless Standalone (MCP Server)

Nexus provides a dedicated **standalone binary (`nexus-mcp`)** built specifically for AI agents (Claude Code, Claude Desktop, Cursor, Antigravity).

It runs **completely headless without opening GUI windows or requiring an audio driver/interface**, allowing an AI agent to autonomously discover, load, configure, perform, and sample arbitrary VST3 and CLAP audio plugins.

### 1. Stdio Mode (Claude Desktop / Claude Code / Cursor / Antigravity)

Build the release binary:
```sh
cargo build --release --bin nexus-mcp
```

Add Nexus to your client configuration (e.g. `claude_desktop_config.json` or `.mcp.json`):
```json
{
  "mcpServers": {
    "nexus": {
      "command": "/path/to/nexus/target/release/nexus-mcp",
      "args": ["--stdio"]
    }
  }
}
```

Or run directly with cargo:
```sh
cargo run --bin nexus-mcp -- --stdio
```

### 2. Streamable HTTP Mode

To serve the MCP endpoint over local HTTP:
```sh
cargo run --bin nexus-mcp -- --port 9997
```
The server binds to `127.0.0.1:9997` and `[::1]:9997`, serving the MCP endpoint at `http://127.0.0.1:9997/mcp`.

### 3. What the AI Can Do Headlessly

- **`list_plugins` / `load_plugin`**: Discover installed CLAP and VST3 plugins from standard paths and load any plugin into memory.
- **`list_presets` / `load_preset` / `save_preset`**: Browse, load, or save `.nexuspreset` files.
- **`list_params` / `get_params` / `set_params`**: Inspect and tweak plugin parameters.
- **`render_audio`**: Render an offline musical performance (notes, CC automation, parameter ramps) at maximum CPU speed and receive 32-bit float 48 kHz WAV audio (as base64 or saved to disk) along with audio metrics (`peak_db`, `rms_db`, `duration_s`, `is_silent`).
- **`decode_state` / `edit_state` / `diff_state` / `snapshot_state`**: Inspect structured plugin state and apply JSON-pointer edits.

---

## Interactive GUI Mode (with REST Debug UI)

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

### Bundling plugin crates

Plugin packaging is handled by the `xtask` helper, driven through cargo
aliases. It applies to plugin crates — today that's `nexus-wrap`, which is
still a stub:

```sh
cargo bundle            # build + bundle CLAP and VST3 for this platform
cargo bundle-clap       # CLAP only
cargo bundle-vst3       # VST3 only
cargo bundle-debug      # debug profile instead of release
cargo install-plugin    # build, bundle, and install into the user plugin dirs
```

Bundles are written to `target/bundle/` and installed to the standard
per-user locations (`~/Library/Audio/Plug-Ins/{CLAP,VST3}` on macOS, `~/.clap`
and `~/.vst3` on Linux). The bundler is name-agnostic: any workspace crate
with a `[package.metadata.bundle]` table and a `cdylib` target is bundled,
discovered via `cargo metadata` — there is no separate bundler config.

Bundling as VST3 requires the Steinberg VST3 SDK; point
`CLAP_WRAPPER_VST3_SDK` at your SDK checkout. (Only the *export* side needs
it — VST3 *hosting* is pure Rust.) The CLAP build needs no extra setup.

## Under the hood

Nexus leans on the Rust audio-plugin ecosystem rather than reinventing it:

- **CLAP hosting** — [`clack`](https://github.com/prokopyl/clack) for safe
  CLAP bindings. `clack` covers both sides, so `nexus-host` uses its host API
  and `nexus-wrap` will use its plugin API.
- **VST3 hosting** — the pure-Rust [`vst3`](https://crates.io/crates/vst3-sdk)
  COM bindings (no Steinberg SDK checkout needed to host).
- **Audio/MIDI I/O** — [`cpal`](https://github.com/rustaudio/cpal) output and
  [`midir`](https://github.com/Boddlnagg/midir) input live in `nexus-app`;
  the `nexus-host` library has no device I/O at all.
- **UI** — [`egui`](https://github.com/emilk/egui)/`eframe` on the wgpu
  (Metal) renderer; plugin editors are native child windows.
- **VST3 export (planned)** —
  [`clap-wrapper`](https://github.com/blepfx/clap-wrapper-rs) will wrap the
  CLAP `nexus-wrap` plugin as a VST3, so a single implementation ships in
  both formats.

## License

Licensed under the [MIT license](LICENSE).

# Nexus — agent guide

A Rust VST3/CLAP plugin host, built in layers (see `README.md`):
`nexus-host` (engine) → `nexus-wrap` (a plugin that hosts a plugin) →
`nexus-daw` (an AI-first DAW). Build automation lives in `xtask`.

## Version control: use jj (Jujutsu) by default

This repo is managed with **[Jujutsu](https://jj-vcs.github.io/jj/)**, colocated
with Git (`.git` and `.jj` coexist). **Prefer `jj` over `git` for all version
control.** Git is kept colocated only so Git-based tooling can still read the
repo — do not create commits or move branches with raw `git`.

Common operations:

| Task | Command |
| --- | --- |
| See status / working copy | `jj status` |
| View history | `jj log` |
| Commit the current change | `jj commit -m "message"` (NOT `jj describe` + `jj new`) |
| Edit an earlier change | `jj edit <change-id>` |
| Move a bookmark (branch) to current change | `jj bookmark set main -r @` |
| Push to a Git remote | `jj git push` |
| Pull/import from Git remote | `jj git fetch` |

Notes:
- jj has no staging area — the working copy *is* a commit and is snapshotted
  automatically. **Always finalize with `jj commit -m "..."`** — it describes the
  current change and starts a fresh one in a single step. Do not use
  `jj describe` + `jj new` to commit.
- "Branches" are **bookmarks** in jj and do not move automatically; advance
  `main` explicitly with `jj bookmark set main -r @-` (or `-r @`) when a change
  is ready.
- Only create commits when the user asks, same as the normal Git workflow.

## Build, test, bundle

```sh
cargo build                 # build the whole workspace
cargo test                  # run tests
cargo run --bin nexus       # run the (stub) DAW

cargo bundle                # build + bundle every plugin crate as CLAP + VST3
cargo bundle-clap           # CLAP only   (also: bundle-vst3, bundle-debug)
cargo install-plugin        # build, bundle, and install into user plugin dirs
```

The `xtask` bundler is **plugin-name-agnostic**: it discovers what to bundle
from `cargo metadata` — any crate with a `[package.metadata.bundle]` table and a
`cdylib` target is bundled. Don't hardcode crate names there. Building VST3
requires `CLAP_WRAPPER_VST3_SDK` pointing at the Steinberg VST3 SDK.

## Conventions

- Keep code DRY; match the style of surrounding code.
- Plugin stack mirrors the sibling `droplets` project: `clack` for CLAP (host
  and plugin), `clap-wrapper` for VST3 export.

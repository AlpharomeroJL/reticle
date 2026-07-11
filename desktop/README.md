# Reticle desktop (Tauri)

The desktop build: a native window on the system webview (WebView2 on Windows)
that loads the same UI as the browser build, bundled for fully offline use, and
the native-only home for the rhai PCell producer (and, on the roadmap, the real
agent). See `docs/decisions/0119-tauri-desktop.md` for the design rationale and
`docs/src/desktop.md` for the user-facing chapter.

This crate is workspace-**excluded** (root `Cargo.toml`'s `[workspace].exclude`,
matching the `crates/reticle-py` precedent): Tauri/wry/webview2 are a heavy,
Windows-webview-coupled dependency chain that must never enter `just ci`,
`just wasm-build`, or `cargo nextest run --workspace`. It has its own
`Cargo.lock`, committed for reproducibility.

## Build

```sh
# 1. Build the offline web bundle this shell embeds (from the repo root).
just web-build   # == cd crates/web; trunk build index.html --release

# 2. Build the desktop shell (from this directory; its own Cargo.lock/target).
cd desktop
cargo build
```

Step 1 must run first: `tauri.conf.json`'s `build.frontendDist` points at
`../crates/web/dist`, and Tauri embeds whatever is in that directory into the
compiled binary *at compile time*. If `crates/web/dist` is missing or stale,
rebuild it before building this crate.

## Run

```sh
cargo run    # from desktop/, or: ./target/debug/reticle-desktop.exe
```

A single window opens, titled "Reticle", showing the same editor the browser
build shows. Nothing is fetched over the network: the HTML/JS/wasm were
embedded into the binary in step 1 above. A "Reticle" menu (native menu bar)
has one action, **"Regenerate demo PCell (native, offline)"**, which runs the
real sandboxed rhai producer (`reticle_script::produce`) against a small
built-in fixture and shows the result in an alert inside the window: the
concrete, native-only feature the browser build honestly cannot run (ADR 0115).

## What is NOT here (ledgered, follow-on)

- Code signing, installers (`.msi`/`.dmg`), auto-update: follow-on work.
- Exposing the plugin host through this shell: after the plugin-host lane
  (wave B).
- Wiring the real agent (`reticle-agent`, already a declared dependency of
  this crate) to a menu action: its live run mode needs a reachable model
  backend, out of scope for the network-disabled offline proof this lane
  targets.
- No PWA `file_handlers` (premise P14: out of scope for a Tauri shell, which
  has its own native file-open story).

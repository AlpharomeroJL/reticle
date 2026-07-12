# Desktop (Tauri)

`desktop/` is a native desktop build: a window on the system webview (WebView2
on Windows) that loads the same UI as the browser build, bundled for fully
offline use, and the native-only home for features the browser build honestly
defers (ADR 0115): the rhai PCell producer today, and the real agent on the
roadmap. See `docs/decisions/0119-tauri-desktop.md` for the full design
rationale.

It is a separate, workspace-**excluded** crate (like `crates/reticle-py`, ADR
0087): Tauri's dependency chain (`wry`, `webview2-com`, `tao`) never enters
`just ci`, `just wasm-build`, or `cargo nextest run --workspace`.

## Why a desktop app, when the editor already runs in a browser

`reticle-script` (the rhai PCell producer) and `reticle-agent` (the real
propose-verify-correct agent) are native-only dependencies of `reticle-app`
(ADR 0115): shipping either in the wasm bundle would blow the measured
browser-bundle gz budget by an order of magnitude. The browser's PCell Inspector
shows the *predicted* provenance and an honest disclaimer that live produce
runs in the desktop app. This chapter's crate is that desktop app.

## Build

Two steps, in order, from the repo root:

```sh
# 1. Build the offline web bundle the shell embeds.
just web-build

# 2. Build the desktop shell itself (its own Cargo.lock and, effectively, its
#    own target dir, since it is workspace-excluded).
cd desktop
cargo build
```

There is no `cargo-tauri` CLI step: `desktop/build.rs` calls
`tauri_build::build()` directly and `src/main.rs` calls
`tauri::Builder::default()....run(tauri::generate_context!())`, so a plain
`cargo build` is the whole build. Step 1 must run first and must be re-run
whenever the web UI changes: `desktop/tauri.conf.json`'s `build.frontendDist`
points at `../crates/web/dist`, and Tauri embeds whatever is in that directory
into the compiled binary at compile time. Building the shell without a fresh
`crates/web/dist` ships whatever was there last.

## How the offline bundling works

Tauri's asset-embedding step (driven by `tauri::generate_context!()`) reads
`crates/web/dist` at compile time and embeds every file's bytes directly into
the binary. At run time, the webview loads `tauri://localhost/` and Tauri's own
asset protocol answers from those embedded bytes. No local HTTP server is
started, and after compilation nothing depends on `crates/web/dist` still
existing on disk or on any network reachability. This is a stronger guarantee
than the browser PWA's offline story (a service worker cache populated over
the network on first load, see [Install and offline](pwa.md)): the desktop
shell needs no service worker at all, because there is no network fetch for
one to intercept.

The web bundle's HTML/CSS/JS already use only relative paths (the work that
made the gh-pages subpath deploy correct also makes the bundle safe to serve
from Tauri's asset root), so no changes were needed to `crates/web` to make it
embeddable.

## The native-only proof: Regenerate demo PCell

The window's "Reticle" menu has one action: **"Regenerate demo PCell (native,
offline)."** Choosing it runs the real sandboxed rhai producer
(`reticle_script::produce`) against a small built-in fixture (a parametric
pixel array; the same fixture shape used by
`reticle_script::pcell::tests::sensor_def` and the browser PCell panel's own
demo) and shows the result, geometry counts and the stamped provenance, in an
alert inside the window. It calls the production sandbox directly; nothing
about the result is scripted or replayed.

This runs as a native menu action rather than a button inside the (unmodified)
web UI so that no change was needed to `reticle-app`'s source: a native
menu-event handler is fully-trusted Rust code with no ACL/capability surface
to configure, unlike a command invoked from the webview's JS.

## What is deferred

- **Installers, code signing, auto-update.** `desktop/tauri.conf.json` leaves
  `bundle.active` at its default `false`; this crate builds a plain
  executable, not a signed package. Follow-on work.
- **The real agent.** `reticle-agent` is already a dependency of `desktop/`
  (see `cargo tree` below), but no menu action calls it yet: its live run mode
  needs a reachable model backend, which does not fit an offline,
  network-disabled proof. Wiring an agent action is follow-on work.
- **Plugin exposure.** After the plugin-host lane (wave B).
- **Live collaboration ("Go live") from the desktop shell.** The bundled CSP's
  `connect-src` does not yet include the Share relay's `wss://` origin. The
  feature already and correctly requires network; this is a small follow-on
  CSP amendment when prioritized, not a defect.

## Proof

```sh
# From desktop/: the shell builds clean, on its own Cargo.lock, and both
# native-only crates are reachable in its dependency graph.
cd desktop
cargo build
cargo tree | grep -E "reticle-script|reticle-agent"
cd ..

# From the repo root: excluding desktop/ from the workspace does not disturb
# the existing gate.
just wasm-build
cargo nextest run --workspace --no-run
```

The GUI half (opening the window with the network disabled, confirming the
bundled UI loads with no request ever leaving the process, and reading a real
produce result from the native menu action) is a headed, interactive check;
see `scratch/lanes/tauri/RESULT.md` for the exact launch command and the
expected observation.

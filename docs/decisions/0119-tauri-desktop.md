# 0119, the desktop shell is Tauri wrapping the offline web bundle, and the native-only home

## Context

ADR 0115 made `reticle-script` (the rhai PCell producer) and, earlier,
`reticle-agent` (the real propose-verify-correct agent) native-only
dependencies of `reticle-app`, to keep the browser wasm bundle inside its
measured `+450 KiB` gz budget. Both features are real and fully implemented;
they simply have no browser-honest place to run. ADR 0115 named the
consequence directly: "Live produce runs in the desktop app... it never claims
to run a live sandboxed produce it cannot," but until this ADR no desktop app
existed. `scratch/campaign/v82-backlog.md` tracked this as a stub: the in-app
disclaimer's "desktop app" was aspirational.

Phase 4 (v8.2.0) closes that gap: a real, native-only-capable desktop binary.

Three shapes were considered:

1. **A second native GUI**, wrapping `reticle-app`'s existing native `eframe`
   binary (which already runs natively, wgpu-rendered, with rhai and the agent
   in its dependency graph) in a nicer installer/window chrome.
2. **A Tauri shell hosting the same web assets** the browser gets (the
   Trunk-built `crates/web/dist`), in a system webview (WebView2 on Windows),
   with the native-only crates as direct dependencies of the shell binary
   itself rather than of the wrapped UI.
3. **A Tauri shell wrapping `reticle-app` as a linked native library**, so the
   webview and the native eframe renderer coexist in one process.

Shape 3 was rejected outright: Tauri's window IS the system webview; running a
second, separate wgpu-rendered native window inside the same process buys
nothing and doubles the GPU surface for no feature. Shape 1 was rejected
because it does not touch the actual carry-item: `reticle-app`'s native binary
*already* runs rhai produce and the agent today (`cargo run -p reticle-app`
does this now, and always has, since ADR 0115); wrapping it in a nicer window
is a packaging exercise, not new capability, and does not honor the brief's
"wraps the trunk-built web assets" framing, which is specifically about
distributing the SAME browser UI offline. Shape 2 was chosen.

## Decision

`desktop/` is a new, workspace-**excluded** crate (root `Cargo.toml`'s
`[workspace].exclude`, matching the `crates/reticle-py` precedent, ADR 0087):
Tauri/wry/webview2 are a heavy, Windows-system-webview-coupled dependency chain
that must never enter `just ci`, `just wasm-build`, or
`cargo nextest run --workspace`. It is its own standalone package with its own
committed `Cargo.lock` (`cd desktop && cargo build`), built CLI-free: no
`cargo-tauri` is required (none is installed on the build host; the brief
explicitly preferred this to reduce toolchain risk). `build.rs` calls
`tauri_build::build()`, and `src/main.rs` calls
`tauri::Builder::default()....run(tauri::generate_context!())` directly.

**Offline asset bundling.** `tauri.conf.json`'s `build.frontendDist` points at
`../crates/web/dist`, the same directory `just web-build`
(`trunk build index.html --release`) produces for the gh-pages deploy.
`tauri::generate_context!()` reads that directory at COMPILE time and embeds
every file's bytes into the compiled binary (Tauri's asset-embedding
codegen, `tauri-codegen`'s `EmbeddedAssets`). At run time the webview requests
`tauri://localhost/...` (or `https://tauri.localhost/...`), and Tauri's own
asset protocol handler answers from those embedded bytes; no local HTTP server
is started and no network request is made. This is the same mechanism that
distinguishes an installed desktop app from "a browser pointed at
localhost": once compiled, the binary has no dependency on `crates/web/dist`
still existing on disk, and no dependency on any network reachability. It is
the strongest offline guarantee this codebase has (the PWA's offline story, by
contrast, depends on a service worker cache that is populated over the network
on first load; the desktop shell needs no service worker at all, since there
is no network fetch for it to intercept in the first place).

The web bundle's HTML/CSS/JS were already fully relative-path-safe (the
gh-pages subpath work, ADR-adjacent to 0098/0099): `index.html` links
`manifest.json`, `icon-192.png`, `./sw.js`, etc. with no leading `/`. That
same property, built for a subpath deploy, is exactly what a custom-protocol
asset root needs; no HTML/JS changes were required to make the existing bundle
Tauri-safe.

**CSP.** `app.security.csp` is set to Tauri's documented baseline,
`default-src 'self'; connect-src ipc: http://ipc.localhost`. Tauri's
asset-embedding codegen computes a SHA-256 hash of every inline `<script>` tag
in each embedded HTML file and appends it to `script-src` automatically
(`tauri_utils::html2::normalize_script_for_csp` plus the codegen's
`CspHashes`), and injects a per-load nonce into every inline `<style>` tag; the
policy above is the base string that mechanism amends. `index.html` has
several inline `<script>` blocks (the boot-failure watchdog, the boot-tip
rotator, the backend-status notice, the service-worker registration, the
in-browser convert driver) and one inline `<style>` block; none needed a
manual nonce or a rewrite into an external file, because this hashing is
automatic and happens at the SAME compile step that embeds the assets.
`connect-src ipc: http://ipc.localhost` is Tauri's own IPC channel allowance
(needed for any future `invoke()`-based command; unused today, see below, but
harmless to allow now). Not yet in `connect-src`: the Share/"Go live" relay's
`wss://` origin, so live collaboration from the desktop shell does not work
today; that feature already and correctly requires network (off for this
lane's offline proof), so this is a ledgered follow-on CSP amendment, not a
regression.

**The native-only home.** `desktop/Cargo.toml` depends directly on
`reticle-script` (produce) and `reticle-agent` (the real agent), NOT on
`reticle-app`. `reticle-app` is a GUI crate coupled to `eframe`/`egui-wgpu`/
`wgpu` for its native window; the desktop shell's UI is the webview, so linking
that whole native rendering stack into this binary would buy nothing and
triple its build time. "Wraps the trunk-built web assets" means wrapping the
build ARTIFACT (the compiled HTML/JS/wasm), not the `reticle-app` Rust crate as
a linked library; the two crates that matter for the native-only-home contract
(`reticle-script`, `reticle-agent`) are depended on directly instead, which is
both lighter and a more literal reading of "the FULL-AUTHORING home for
everything made native-only." `cargo tree -p reticle-desktop` shows both
reachable (see Proof).

**The concrete proof, without touching `reticle-app`.** The brief's owned
paths forbid editing `reticle-app` source except as a last resort, and the
Gate-4 ask is a GUI-observable proof that live produce runs. Rather than wire
a `#[tauri::command]` invoked from the (unmodified) web UI's own PCell panel,
which would need that panel to detect it is running inside Tauri and call
`invoke()`, an edit to `reticle-app` this ADR avoids entirely, the shell adds
its own native "Reticle" menu with one action: "Regenerate demo PCell (native,
offline)". The handler runs `reticle_script::produce` against a small bundled
fixture (the `reticle-script/examples/param_cell.rhai` body, parameterized;
the same fixture shape already proven by
`reticle_script::pcell::tests::sensor_def` and the browser panel's own demo)
and shows the result via `WebviewWindow::eval` (a JS `alert(...)` inside the
same window). This sidesteps Tauri v2's capability/ACL system entirely: a
native menu-event handler runs fully-trusted Rust code directly, with no
`invoke_handler`, no exposed command, and therefore no `capabilities/*.json` to
author, since only JS-invoked commands are ACL-gated. `desktop/` ships with
zero capability files as a result, which is deliberate, not an oversight.

**Icons.** Tauri's Windows build step (`tauri-build`, via `tauri-winres`)
requires a real `.ico` file to exist at `icons/icon.ico` to embed as the
`.exe`'s resource; this is a different, stricter requirement than the
in-app window icon (which falls back to a PNG). `desktop/icons/icon.ico` and
`icon.png` are generated from the already-committed, already-shipped
`crates/web/icon-192.png` (the PWA icon), not a new piece of art: the `.ico`
wraps that same 192x192 RGBA image as a modern PNG-format icon directory entry
(verified by round-tripping it through the same `ico` crate version
`tauri-codegen` uses, decoding back to the expected 147,456 raw RGBA bytes),
so the desktop and web builds present the same mark.

## Consequences

- The desktop shell is honest about what it is: the same UI as the browser,
  offline, plus the native-only features the browser build cannot honestly
  claim. It does not duplicate `reticle-app`'s native GUI or introduce a
  second rendering stack.
- `reticle-agent` is a reachable dependency (satisfying the native-only-home
  contract and the roadmap framing) but is NOT wired to a menu action yet: its
  live run mode needs a reachable model backend, which is out of scope for a
  network-disabled offline proof. This is ledgered, not silently dropped;
  wiring an agent action is follow-on work for whichever lane owns it next.
- No installers (`.msi`), code signing, or auto-update: `bundle.active` is
  left at its default `false` (the crate produces a plain `.exe`, not a signed
  package). Follow-on, per the brief's park list.
- No plugin exposure through this shell yet: that is the plugin-host lane's
  scope (wave B).
- The CSP's `connect-src` does not yet allow the Share/relay's `wss://`
  origin, so live collaboration is not available from the desktop shell today;
  ledgered above, a small follow-on amendment when that is prioritized.
- Building requires two steps in order (`just web-build`, then
  `cd desktop && cargo build`), not one: there is no `cargo-tauri` CLI on the
  build host to automate the first step, and none was added (the brief
  preferred a CLI-free setup to reduce toolchain risk). `desktop/README.md`
  and `docs/src/desktop.md` document the two-step sequence.

## Proof

- `cd desktop && cargo build` exits 0 (see `scratch/lanes/tauri/RESULT.md` for
  the exact recorded run).
- `cargo tree -p reticle-desktop` (run from `desktop/`) lists both
  `reticle-script` and `reticle-agent`.
- From the repo root, with `desktop` present but excluded: `just wasm-build`
  and `cargo nextest run --workspace --no-run` both still succeed (see
  `scratch/lanes/tauri/RESULT.md` for the recorded exit codes).
- The GUI proof (open the window with the network disabled, confirm the
  bundled UI loads with no request ever leaving the process, click "Regenerate
  demo PCell (native, offline)" and read the produce result in the alert) is
  orchestrator-driven at Gate 4, per the brief's GUI-interactive split; the
  exact launch command and expected observation are in
  `scratch/lanes/tauri/RESULT.md`.

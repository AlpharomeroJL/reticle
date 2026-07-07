# Reticle end-to-end tests

Playwright drives the browser demo (the Trunk build of `crates/web`) in headless
Chromium and asserts it boots and renders. This is wired as `just e2e` and is its
own gate, separate from `just ci`.

## Run

From the repository root:

```
just e2e
```

That builds the demo bundle (`trunk build`), installs the Playwright browser if
needed, and runs the `webgl2` and `webgpu` projects. The `ghpages-subpath` project
needs a bundle built for the deploy subpath, so it has its own recipe:

```
just e2e-subpath
```

To run pieces directly from this directory:

```
npm install
npx playwright install chromium
npx playwright test                 # all projects
npx playwright test --project=webgl2
npx playwright test --project=webgpu
npx playwright test --project=ghpages-subpath   # needs a /reticle/ build (see below)
node probe-capability.mjs           # report this host's WebGPU/WebGL2 capability
node subpath-check.mjs              # standalone subpath boot check (see below)
```

## What the projects mean (ADR 0027)

- `webgl2` is the hard gate. A page init script removes `navigator.gpu` so `wgpu`
  takes its WebGL2 fallback on any host, and the app must boot and render. The boot
  signal is real: `web/src/main.rs` hides the `#overlay` element only after
  `eframe::WebRunner::start().await` resolves, i.e. after the renderer initialized.
- `webgpu` launches Chromium with the WebGPU-enabling flags. Where a real adapter
  exists it asserts the WebGPU path (the page reports "WebGPU detected" and an
  adapter is returned). Where none exists, the WebGPU-only assertions skip with an
  annotation instead of failing, while the shared boot check still runs.
- `ghpages-subpath` mounts the SAME bundle under the `/reticle/` subpath (via
  `serve-subpath.mjs`), exactly as GitHub Pages serves the public site, and asserts
  the app boots there with no 404 on the js/wasm. It catches the front-door
  base-path regression (assets emitted at absolute root, `/web-<hash>.js`, that 404
  under the subpath and hang the page) BEFORE deploy. It requires a bundle built
  with `--public-url /reticle/`; a root-path build 404s here, which is the point.
  `just e2e-subpath` builds that bundle and runs this project.
- `share-live` is the two-context proof of the share-link LIVE transport (ADR 0058).
  It needs a real `reticle-server` relay running alongside the bundle, so it is opt-in:
  `just e2e-share` builds the bundle AND the relay binary and runs it with
  `SHARE_LIVE=1`, which adds `serve-relay.mjs` (launches the prebuilt relay on
  `127.0.0.1:3030`, health on `3031`) to the `webServer` list. Context A boots the
  editor and goes live for a room (`?share=1&room=..&relay=..`, publishing its document
  and presence); context B opens the read-only viewer link
  (`?view=viewer&room=..&relay=..`) and its wasm `web_sys::WebSocket` transport streams
  A's live frames. The spec asserts (headlessly, WebGL2 path): both bundles boot, B's
  `?mode=view` socket OPENS, and B RECEIVES and DECODES a real `SyncMessage` frame from
  A (`reticle-live: socket open` / `reticle-live: first frame ...` in B's console). It
  does NOT assert pixel-level rendering (headless Chromium here is WebGL2-only and the
  egui UI is canvas-painted, so there are no DOM nodes for the mirrored geometry); the
  authoritative proof that a viewer materializes the sharer's geometry and presence AND
  that a viewer's frame is dropped server-side is the headless Rust relay test
  `crates/reticle-server/tests/share_live.rs`. A's "Go live" is triggered by the
  `?share=1` boot flag rather than a DOM click because that button is canvas-painted;
  the publish path exercised is identical. Beyond boot and transport, the spec adds two
  behavioral proofs over the wasm `window.__reticle_stats` seam (ADR 0058/0068): an edit
  made in A paints in B (with `?e2e-edit=1` A places one scripted rect; a no-edit control
  room isolates it to exactly `+1` applied shape), and a view-mode socket cannot write
  (the same captured relay frame is dropped when sent from `?mode=view` but applied when
  sent edit-mode, a positive control so the drop is not vacuous). Pixels stay out of
  scope; the counter seam is the browser-observable proof.
- `phone` (ADR 0068) runs the touch spec on a Pixel 7 device descriptor (mobile viewport,
  `hasTouch`). It opens an example document via an intercepted `?gds=` fetch, synthesizes
  a two-finger pinch and drag with CDP `Input.dispatchTouchEvent`, and asserts the camera
  moved via `window.__reticle_stats.camera` (the wasm camera-readout seam). It needs no
  relay; run it with `npx playwright test --project=phone`.

Playwright's bundled headless Chromium ships without WebGPU, so on this host the
`webgpu` project's backend assertion skips and the app is verified on the WebGL2
path. Run the suite against a browser with a real WebGPU adapter (for example system
Chrome or Edge) to exercise the WebGPU path for real; the same spec activates its
assertions with no change. `node probe-capability.mjs` prints exactly what the local
Chromium can do.

`node subpath-check.mjs` is a standalone, runner-free version of the subpath boot
check: it serves the built `../crates/web/dist` under `/reticle/`, opens it in
headless Chromium, and fails (exit 1) on any js/wasm 404 or if the app does not
boot. It needs the same `--public-url /reticle/` bundle.

## Layout

- `playwright.config.ts` five projects (`webgl2`, `webgpu`, `ghpages-subpath`,
  `share-live`, `phone`) plus the static `webServer`s (root and `/reticle/`, and the relay
  when `SHARE_LIVE=1`).
- `serve-dist.mjs` dependency-free static server for `../crates/web/dist` at root
  (serves `application/wasm` correctly).
- `serve-subpath.mjs` the same, but mounted under `/reticle/` to mirror GitHub Pages;
  anything outside `/reticle/` 404s so a stray absolute-root asset ref is a hard error.
- `serve-relay.mjs` launches the prebuilt `reticle-server` relay (honoring
  `CARGO_TARGET_DIR`) and a health endpoint Playwright waits on; used only by the
  `share-live` run (`SHARE_LIVE=1`).
- `tests/reticle-boot.spec.ts` the boot-and-render gate and the backend-path check
  (root projects).
- `tests/subpath-boot.spec.ts` the subpath boot gate (`ghpages-subpath` project only).
- `tests/share-live.spec.ts` the share-link LIVE transport two-context proof plus the
  edit-paints-in-B and view-mode-cannot-write behavioral proofs (`share-live` project
  only; needs the relay).
- `tests/phone-touch.spec.ts` the phone-viewport touch pan-zoom proof (`phone` project).
- `capture-share.mjs` a standalone harness (NOT part of the test run) that drives two
  headed browser contexts over the relay to capture the real share GIF; run via
  `just capture-share`.
- `subpath-check.mjs` standalone subpath boot check (not part of the default gate).
- `probe-capability.mjs` standalone capability probe (not part of the gate).

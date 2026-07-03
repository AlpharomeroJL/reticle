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

- `playwright.config.ts` three projects (`webgl2`, `webgpu`, `ghpages-subpath`) plus
  two static `webServer`s (root and `/reticle/`).
- `serve-dist.mjs` dependency-free static server for `../crates/web/dist` at root
  (serves `application/wasm` correctly).
- `serve-subpath.mjs` the same, but mounted under `/reticle/` to mirror GitHub Pages;
  anything outside `/reticle/` 404s so a stray absolute-root asset ref is a hard error.
- `tests/reticle-boot.spec.ts` the boot-and-render gate and the backend-path check
  (root projects).
- `tests/subpath-boot.spec.ts` the subpath boot gate (`ghpages-subpath` project only).
- `subpath-check.mjs` standalone subpath boot check (not part of the default gate).
- `probe-capability.mjs` standalone capability probe (not part of the gate).

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
needed, and runs both projects. To run pieces directly from this directory:

```
npm install
npx playwright install chromium
npx playwright test                 # both projects
npx playwright test --project=webgl2
npx playwright test --project=webgpu
node probe-capability.mjs           # report this host's WebGPU/WebGL2 capability
```

## What the two projects mean (ADR 0027)

- `webgl2` is the hard gate. A page init script removes `navigator.gpu` so `wgpu`
  takes its WebGL2 fallback on any host, and the app must boot and render. The boot
  signal is real: `web/src/main.rs` hides the `#overlay` element only after
  `eframe::WebRunner::start().await` resolves, i.e. after the renderer initialized.
- `webgpu` launches Chromium with the WebGPU-enabling flags. Where a real adapter
  exists it asserts the WebGPU path (the page reports "WebGPU detected" and an
  adapter is returned). Where none exists, the WebGPU-only assertions skip with an
  annotation instead of failing, while the shared boot check still runs.

Playwright's bundled headless Chromium ships without WebGPU, so on this host the
`webgpu` project's backend assertion skips and the app is verified on the WebGL2
path. Run the suite against a browser with a real WebGPU adapter (for example system
Chrome or Edge) to exercise the WebGPU path for real; the same spec activates its
assertions with no change. `node probe-capability.mjs` prints exactly what the local
Chromium can do.

## Layout

- `playwright.config.ts` two projects (`webgl2`, `webgpu`) plus the static-server
  `webServer`.
- `serve-dist.mjs` dependency-free static server for `../crates/web/dist` (serves
  `application/wasm` correctly).
- `tests/reticle-boot.spec.ts` the boot-and-render gate and the backend-path check.
- `probe-capability.mjs` standalone capability probe (not part of the gate).

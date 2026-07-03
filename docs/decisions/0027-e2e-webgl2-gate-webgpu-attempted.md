# 0027, Playwright e2e: WebGL2 is the hard gate, WebGPU is attempted and skipped honestly headless

## Context

The run needs a Playwright end-to-end suite that drives the browser demo, with a
WebGPU-flagged run and a WebGL2-fallback run, wired as its own `just e2e` gate. A
gate must be deterministic and green. The empirical reality on this host decides the
shape: Playwright's bundled headless Chromium exposes no WebGPU at all. A capability
probe found `navigator.gpu` absent under every flag combination tried, including
`--enable-unsafe-webgpu --enable-features=Vulkan,WebGPU --use-angle=swiftshader` and
Chromium's new headless mode, because that Chromium ships without Dawn. WebGL2, by
contrast, is reliably available through ANGLE on SwiftShader (software). So a run that
hard-asserted a real WebGPU device would either fail here or, worse, be tempted to
fake a pass.

## Decision

Two Playwright projects run the same spec against the Trunk-built bundle, served by a
dependency-free Node static server (`serve-dist.mjs`, which sets `application/wasm` so
the browser will stream-compile). The `webgl2` project is the hard gate: a page init
script deletes `Navigator.prototype.gpu` so `wgpu` takes its WebGL2 fallback on any
host, GPU or not, and the app must boot and render. The boot signal is genuine: the
wasm entry point (`web/src/main.rs`) hides `#overlay` only after
`eframe::WebRunner::start().await` resolves, which happens only once the renderer
initialized on the canvas. The `webgpu` project launches with the WebGPU-enabling
flags and asserts the WebGPU path (status reads "WebGPU detected", a real adapter is
returned) only when an adapter actually exists; where none does, those WebGPU-only
assertions `test.skip` with an annotation rather than fail, while the shared boot
check still runs.

## Consequences

`just e2e` is deterministic and green on this GPU-less host and stays correct on a
host with a real adapter, where the same `webgpu` spec activates its assertions with
no code change. The WebGL2 render path is always exercised end to end; the WebGPU path
is exercised for real wherever it exists and is honestly recorded as skipped where it
does not, so the suite never publishes a WebGPU pass it did not observe. The cost is
that continuous WebGPU coverage is not available in this headless CI; running the suite
against system Chrome or a future WebGPU-capable Playwright Chromium closes that gap.

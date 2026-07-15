import { defineConfig, devices } from "@playwright/test";

// End-to-end tests for the Reticle browser demo.
//
// Three projects:
//   * webgl2  - the hard gate. navigator.gpu is hidden by an init script so wgpu
//               takes its WebGL2 fallback on ANY host, GPU or not. The app must
//               boot and render. Served at root by serve-dist.mjs (port 8080).
//   * webgpu  - launched with the WebGPU-enabling Chromium flags. Where a real
//               adapter exists it asserts the WebGPU path; where it does not
//               (for example Playwright's headless Chromium, which ships without
//               Dawn) the WebGPU-only assertions skip honestly while the boot
//               check still runs. Served at root by serve-dist.mjs (port 8080).
//   * ghpages-subpath - serves the SAME bundle under the `/reticle/` subpath
//               (serve-subpath.mjs, port 8081), exactly as GitHub Pages does, and
//               asserts the app boots with no 404 on the js/wasm. A bundle built
//               without `--public-url /reticle/` emits absolute-root asset refs
//               that 404 here, so this project catches the front-door base-path
//               regression BEFORE deploy. It reuses the same boot signal as
//               webgl2 (main.rs hides #overlay only after the renderer starts).
//
// The bundle under test is the Trunk build in ../crates/web/dist. For the subpath
// project it MUST be built with `--public-url /reticle/` (see `just e2e-subpath`).

const PORT = Number(process.env.PORT || 8080);
const SUBPATH_PORT = Number(process.env.SUBPATH_PORT || 8081);
const BASE_URL = `http://127.0.0.1:${PORT}`;
const SUBPATH_BASE_URL = `http://127.0.0.1:${SUBPATH_PORT}/reticle/`;

// The served-archive e2e (lane v8-2e) opens `?archive=<url>` pointing at a committed
// `.rtla` fixture served with HTTP Range support by serve-archive.mjs on ARCHIVE_PORT
// (8082). It runs against this LOCAL ranged server regardless of any cloud hosting (a
// Wave 2 gate requirement), and on a different port than the bundle so the fetch is
// cross-origin (which the local server answers with permissive CORS).
const ARCHIVE_PORT = Number(process.env.ARCHIVE_PORT || 8082);
const ARCHIVE_HEALTH_URL = `http://127.0.0.1:${ARCHIVE_PORT}/fixture.rtla`;

// The share-live e2e (ADR 0058) needs a real reticle-server relay running alongside
// the bundle. It is opt-in (SHARE_LIVE=1, set by `just e2e-share`) so the ordinary
// e2e run does not require the relay binary to be built. serve-relay.mjs launches the
// prebuilt relay on RELAY_PORT (3030) and a health endpoint on RELAY_HEALTH_PORT (3031)
// Playwright waits on.
const SHARE_LIVE = !!process.env.SHARE_LIVE;
const RELAY_HEALTH_PORT = Number(process.env.RELAY_HEALTH_PORT || 3031);
const RELAY_HEALTH_URL = `http://127.0.0.1:${RELAY_HEALTH_PORT}`;

// Flags that ask Chromium to expose WebGPU with a software (SwiftShader/Dawn)
// adapter. They are a no-op where the Chromium build has no WebGPU support.
const WEBGPU_ARGS = [
  "--enable-unsafe-webgpu",
  "--enable-features=Vulkan,WebGPU",
  "--use-angle=swiftshader",
  "--use-gl=angle",
];

// Software GL so headless has a WebGL2 implementation to fall back onto.
const WEBGL2_ARGS = ["--use-angle=swiftshader", "--use-gl=angle"];

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 45_000 },
  reporter: [["list"]],
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "webgl2",
      // The subpath, share-live, served-archive, pwa, convert, phone, and doc-switch specs
      // each run in their own project below.
      testIgnore: /subpath-boot\.spec\.ts|share-live\.spec\.ts|served-archive\.spec\.ts|pwa\.spec\.ts|convert-opfs\.spec\.ts|phone-touch\.spec\.ts|doc-switch\.spec\.ts|demo-.*\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      name: "webgpu",
      testIgnore: /subpath-boot\.spec\.ts|share-live\.spec\.ts|served-archive\.spec\.ts|pwa\.spec\.ts|convert-opfs\.spec\.ts|phone-touch\.spec\.ts|doc-switch\.spec\.ts|demo-.*\.spec\.ts/,
      use: { launchOptions: { args: WEBGPU_ARGS } },
    },
    {
      // Only the subpath spec runs here, pointed at the /reticle/-mounted server.
      name: "ghpages-subpath",
      testMatch: /subpath-boot\.spec\.ts/,
      use: {
        baseURL: SUBPATH_BASE_URL,
        launchOptions: { args: WEBGL2_ARGS },
      },
    },
    {
      // The share-live two-context transport e2e (ADR 0058). WebGL2 fallback, since
      // headless Chromium here has no WebGPU adapter; the relay must be running
      // (SHARE_LIVE=1 adds the relay webServer below).
      name: "share-live",
      testMatch: /share-live\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      // The served-archive streaming e2e (lane v8-2e). The bundle is served at root by
      // serve-dist.mjs (8080); the `.rtla` fixture it streams comes from the ranged
      // serve-archive.mjs (8082). WebGL2 fallback, since headless Chromium has no
      // WebGPU adapter here.
      name: "served-archive",
      testMatch: /served-archive\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      // The document-switch RC1 acceptance (lane e2e-revive). Streams a served `.rtla`
      // over `?archive=` (document A), then opens a regular GDS (document B) IN-SESSION by
      // dragging it onto the canvas, and asserts the rendered/interactive document is B and
      // that Run DRC re-enabled -- the browser-level bar for RC1 (exiting archive browse on
      // open). Headless WebGL2 fallback, like served-archive: the die paints and the editor
      // stats populate without a WebGPU adapter, and no foreground/headed browser is needed
      // because a single Playwright page is the visible tab (its rAF loop runs). Uses the
      // same root serve-dist (8080) bundle and the ranged serve-archive (8082) fixture, so
      // it needs the debug e2e Trunk build (the `__reticle_e2e_dispatch` bridge it drives is
      // debug-only). Served at root by serve-dist.mjs.
      name: "doc-switch",
      testMatch: /doc-switch\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      // The in-browser convert e2e (lane v8-6c). Served at root by serve-dist.mjs
      // (8080); converts a committed GDS to a `.rtla` in OPFS via the convert Web
      // Worker, then reopens it through the `?archive=` streaming path (the SW OPFS
      // bridge serves the ranges). WebGL2 fallback, since headless Chromium has no
      // WebGPU adapter here. Skips honestly where OPFS is unavailable.
      name: "browser-convert",
      testMatch: /convert-opfs\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      // The PWA install + offline e2e (lane v8-4d-pwa). Served at root by
      // serve-dist.mjs (8080); asserts a valid linked manifest and a service
      // worker that registers, controls the page, and (best-effort) serves the
      // app shell offline from its cache. WebGL2 fallback, since headless
      // Chromium here has no WebGPU adapter (the shell checks do not need it).
      name: "pwa",
      testMatch: /pwa\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      // Phone viewport (lane v8-1e): a Pixel 7 device descriptor (mobile viewport,
      // deviceScaleFactor, and hasTouch) drives the touchscreen so a pinch/pan proves
      // the app navigates a design by touch. WebGL2 fallback, like webgl2 above, since
      // the headless host has no WebGPU adapter. Served at root by serve-dist.mjs.
      name: "phone",
      testMatch: /phone-touch\.spec\.ts/,
      use: {
        ...devices["Pixel 7"],
        launchOptions: { args: WEBGL2_ARGS },
      },
    },
    {
      // Headed demo-quality guards (packet v8.1.0-R). These run in a FOREGROUND,
      // HEADED browser on purpose: eframe/egui pauses its requestAnimationFrame loop
      // in a backgrounded or occluded tab, so the canvas goes black and
      // window.__reticle_stats reads null. A black canvas from a background context is
      // NORMAL browser behavior, not a defect, and must never be reported as one. The
      // demo-*.spec.ts guards each call page.bringToFront() and assert
      // document.visibilityState === "visible" before reading stats, so they measure
      // the real, visible runtime. WebGL2 forced (navigator.gpu deleted in the spec).
      name: "headed-webgl2",
      testMatch: /demo-.*\.spec\.ts/,
      use: { launchOptions: { headless: false, args: WEBGL2_ARGS } },
    },
    {
      // The same headed demo guards on the WebGPU path. On this host's real GPU a
      // headed Chromium with the WebGPU flags gets a hardware adapter (unlike the
      // headless CI Chromium, which ships without one), so the example-render guard
      // exercises BOTH backends. Specs that assert a WebGPU-only fact skip honestly
      // where no adapter is present.
      name: "headed-webgpu",
      testMatch: /demo-.*\.spec\.ts/,
      use: { launchOptions: { headless: false, args: WEBGPU_ARGS } },
    },
  ],
  webServer: [
    {
      command: "node serve-dist.mjs",
      url: BASE_URL,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: "node serve-subpath.mjs",
      // Wait on the subpath URL so Playwright confirms the mount is live.
      url: SUBPATH_BASE_URL,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      // The Range-capable server for the served-archive spec's `.rtla` fixture. Cheap
      // and dependency-free, so it runs alongside every project (only served-archive
      // fetches from it). Wait on the fixture URL to confirm it is serving.
      command: "node serve-archive.mjs",
      url: ARCHIVE_HEALTH_URL,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    // The relay, only when the share-live run opts in (keeps the ordinary e2e free of
    // any relay-binary requirement).
    ...(SHARE_LIVE
      ? [
          {
            command: "node serve-relay.mjs",
            url: RELAY_HEALTH_URL,
            reuseExistingServer: !process.env.CI,
            timeout: 60_000,
            stdout: "pipe" as const,
            stderr: "pipe" as const,
          },
        ]
      : []),
  ],
});

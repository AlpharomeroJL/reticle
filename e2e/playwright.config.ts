import { defineConfig } from "@playwright/test";

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
      // The subpath and share-live specs run in their own projects.
      testIgnore: /subpath-boot\.spec\.ts|share-live\.spec\.ts/,
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      name: "webgpu",
      testIgnore: /subpath-boot\.spec\.ts|share-live\.spec\.ts/,
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

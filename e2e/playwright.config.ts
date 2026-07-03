import { defineConfig } from "@playwright/test";

// End-to-end tests for the Reticle browser demo.
//
// Two projects exercise the two render paths the app supports (ADR 0009):
//   * webgl2  - the hard gate. navigator.gpu is hidden by an init script so wgpu
//               takes its WebGL2 fallback on ANY host, GPU or not. The app must
//               boot and render.
//   * webgpu  - launched with the WebGPU-enabling Chromium flags. Where a real
//               adapter exists it asserts the WebGPU path; where it does not
//               (for example Playwright's headless Chromium, which ships without
//               Dawn) the WebGPU-only assertions skip honestly while the boot
//               check still runs.
//
// The bundle under test is the Trunk build in ../crates/web/dist, served by the
// dependency-free serve-dist.mjs.

const PORT = Number(process.env.PORT || 8080);
const BASE_URL = `http://127.0.0.1:${PORT}`;

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
      use: { launchOptions: { args: WEBGL2_ARGS } },
    },
    {
      name: "webgpu",
      use: { launchOptions: { args: WEBGPU_ARGS } },
    },
  ],
  webServer: {
    command: "node serve-dist.mjs",
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});

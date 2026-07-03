import { test, expect } from "@playwright/test";

// gh-pages subpath boot gate (ADR 0027 companion).
//
// This spec runs ONLY in the `ghpages-subpath` project, whose baseURL mounts the
// bundle under `http://127.0.0.1:<port>/reticle/` exactly as GitHub Pages serves
// https://alpharomerojl.github.io/reticle/. It asserts the app boots there with no
// 404 on the js/wasm assets, so the base-path regression that broke the front door
// (absolute-root asset refs under a subpath) fails BEFORE deploy rather than after.
//
// The bundle MUST be built with `--public-url /reticle/` for this to pass; a
// root-path build 404s here, which is the point.

const SUBPATH = "/reticle/";

// Console/page messages that signal a real failure rather than benign noise.
function isFatal(text: string): boolean {
  const m = text.toLowerCase();
  return (
    m.includes("panic") ||
    m.includes("unreachable executed") ||
    m.includes("failed to start the reticle web app") ||
    m.includes("is missing a #reticle-canvas")
  );
}

test("boots under the /reticle/ subpath with no asset 404", async ({ page }, testInfo) => {
  const errors: string[] = [];
  const badAssets: string[] = [];
  const offPrefix: string[] = [];

  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  // Track js/wasm requests: any 4xx/5xx or any path outside /reticle/ is the bug.
  page.on("requestfailed", (req) => {
    const u = req.url();
    if (/\.(js|wasm)(\?|$)/.test(u)) {
      badAssets.push(`${u} (${req.failure()?.errorText ?? "failed"})`);
    }
  });
  page.on("response", (resp) => {
    const u = resp.url();
    if (/\.(js|wasm)(\?|$)/.test(u)) {
      if (resp.status() >= 400) badAssets.push(`${u} -> HTTP ${resp.status()}`);
      const path = new URL(u).pathname;
      if (!path.startsWith(SUBPATH)) offPrefix.push(u);
    }
  });

  // Navigate to the subpath itself (baseURL includes /reticle/). "" resolves to
  // the baseURL as-is; goto("/") would drop the subpath, so use a relative "".
  await page.goto("");

  // The genuine boot signal: web/src/main.rs hides #overlay (display:none) only
  // AFTER eframe::WebRunner::start().await resolves Ok, i.e. the wgpu renderer
  // (WebGPU or its WebGL2 fallback) initialized on the canvas.
  await expect(page.locator("#overlay")).toBeHidden();

  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box, "canvas has a bounding box").not.toBeNull();
  expect(box!.width).toBeGreaterThan(0);
  expect(box!.height).toBeGreaterThan(0);

  const status = ((await page.locator("#status").textContent()) ?? "").trim();
  testInfo.annotations.push({
    type: "subpath",
    description: `base=${testInfo.project.use.baseURL} status=${JSON.stringify(status)}`,
  });

  expect(badAssets, `js/wasm assets that 404'd or failed:\n${badAssets.join("\n")}`).toHaveLength(0);
  expect(offPrefix, `js/wasm assets not under ${SUBPATH}:\n${offPrefix.join("\n")}`).toHaveLength(0);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

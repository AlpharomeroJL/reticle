import { test, expect } from "@playwright/test";

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

test.beforeEach(async ({ page }, testInfo) => {
  // The webgl2 project forces the fallback path by hiding WebGPU from the page,
  // so the same spec gates WebGL2 on a GPU host as well as a headless one.
  if (testInfo.project.name === "webgl2") {
    await page.addInitScript(() => {
      // Genuinely remove WebGPU so both `"gpu" in navigator` and any adapter
      // request see it as absent, forcing wgpu's WebGL2 fallback. A getter that
      // returns undefined would not work: `"gpu" in navigator` tests existence,
      // so the property must be deleted, not shadowed. On a host without WebGPU
      // (this one) the delete is a harmless no-op.
      try {
        delete (Navigator.prototype as { gpu?: unknown }).gpu;
      } catch {
        /* ignore: property already absent or non-configurable */
      }
    });
  }
});

test("boots and the renderer initializes", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/");

  // web/src/main.rs hides #overlay (display:none) only AFTER
  // eframe::WebRunner::start().await resolves, i.e. the wgpu renderer (WebGPU or
  // its WebGL2 fallback) initialized on the canvas. This is the genuine
  // "it rendered" signal, not a mere DOM-ready check.
  await expect(page.locator("#overlay")).toBeHidden();

  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box, "canvas has a bounding box").not.toBeNull();
  expect(box!.width).toBeGreaterThan(0);
  expect(box!.height).toBeGreaterThan(0);

  const backend = await page.evaluate(() => "gpu" in navigator && navigator.gpu != null);
  const status = ((await page.locator("#status").textContent()) ?? "").trim();
  testInfo.annotations.push({
    type: "backend",
    description: `project=${testInfo.project.name} navigator.gpu=${backend} status=${JSON.stringify(status)}`,
  });

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

test("backend path matches the launch mode", async ({ page }, testInfo) => {
  await page.goto("/");
  await expect(page.locator("#overlay")).toBeHidden();

  const caps = await page.evaluate(async () => {
    const hasGpu = "gpu" in navigator && navigator.gpu != null;
    let adapter = false;
    if (hasGpu) {
      try {
        adapter = (await navigator.gpu.requestAdapter()) != null;
      } catch {
        adapter = false;
      }
    }
    let webgl2 = false;
    try {
      webgl2 = document.createElement("canvas").getContext("webgl2") != null;
    } catch {
      webgl2 = false;
    }
    return { hasGpu, adapter, webgl2 };
  });
  const status = ((await page.locator("#status").textContent()) ?? "").trim();
  testInfo.annotations.push({
    type: "capability",
    description: `hasGpu=${caps.hasGpu} adapter=${caps.adapter} webgl2=${caps.webgl2} status=${JSON.stringify(status)}`,
  });

  if (testInfo.project.name === "webgpu") {
    if (caps.hasGpu && caps.adapter) {
      // A real WebGPU adapter exists: the app must report the WebGPU path.
      expect(status).toContain("WebGPU detected");
    } else {
      test.skip(
        true,
        `WebGPU adapter unavailable here (navigator.gpu=${caps.hasGpu}, adapter=${caps.adapter}); ` +
          `the app fell back to WebGL2, which the webgl2 project gates. Run on a GPU browser to exercise WebGPU.`,
      );
    }
  } else {
    // webgl2 project: WebGPU is hidden, so the fallback must be the active path.
    expect(caps.hasGpu, "WebGPU is hidden in the webgl2 project").toBe(false);
    expect(caps.webgl2, "WebGL2 context is available").toBe(true);
    expect(status.toLowerCase()).toContain("webgl2");
  }
});

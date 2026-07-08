import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";

// Phone-viewport proof (lane v8-1e): a Pixel 7 device descriptor (mobile viewport +
// hasTouch, wired in playwright.config.ts) drives real touch gestures and asserts the
// app navigates a design BY TOUCH: a two-finger pinch changes the zoom and a two-finger
// drag changes the pan.
//
// How the camera is observed: the egui canvas is GPU-painted, so there is no DOM node for
// the camera and pixel readback under the headless WebGL2 fallback is unreliable. Instead
// the wasm build publishes the live view camera into `window.__reticle_stats.camera`
// (`{ center_x, center_y, pixels_per_dbu }`), written every editor frame (crates/
// reticle-app/src/app.rs, the camera-readout seam this lane adds alongside v8-1c's
// applied-frame counters). The seam is present ONLY once the editor canvas is showing, so
// its appearance is also the signal that the example document loaded and the Start screen
// was dismissed. The gesture is synthesized with CDP `Input.dispatchTouchEvent` (two touch
// points), which egui aggregates into the same multi-touch gesture a phone produces.
//
// What is NOT asserted, and why (honest scope): the pixels of the reframed canvas. The
// camera-readout seam is the browser-observable proof that the touch gesture reached the
// pan/zoom math; the pinch/pan transform itself is unit-tested window-free in
// crates/reticle-app/src/camera.rs.

// The URL the app fetches for its `?gds=` open; Playwright fulfills it with the
// compiled-in gallery inverter so the editor opens a real document headlessly (dismissing
// the Start screen, which is egui-painted and so unreachable by a DOM click).
const FIXTURE_URL = "/reticle-e2e-fixture.gds";

/** The compiled-in gallery inverter GDS, resolved from this spec file's own path (via
 * `testInfo.file`) so it is found regardless of the process working directory: the spec
 * lives in `e2e/tests`, so the repo root is two levels up. */
function fixtureBytes(specFile: string): Buffer {
  return readFileSync(
    join(dirname(specFile), "..", "..", "crates", "reticle-app", "assets", "sky130_fd_sc_hd__inv_1.gds"),
  );
}

interface CameraReadout {
  center_x: number;
  center_y: number;
  pixels_per_dbu: number;
}

/** The live camera the wasm seam publishes, or null before the editor canvas is up. */
function readCamera(page: Page): Promise<CameraReadout | null> {
  return page.evaluate(
    () =>
      (window as unknown as { __reticle_stats?: { camera?: CameraReadout } }).__reticle_stats
        ?.camera ?? null,
  );
}

test("a phone navigates a design by touch: pinch zooms and drag pans the camera", async ({
  page,
}, testInfo) => {
  expect(testInfo.project.name, "this spec is the phone project").toBe("phone");

  // Force the WebGL2 fallback on the headless host (no WebGPU adapter here anyway).
  await page.addInitScript(() => {
    try {
      delete (Navigator.prototype as { gpu?: unknown }).gpu;
    } catch {
      /* already absent */
    }
  });

  // Serve the compiled-in inverter GDS for the app's `?gds=` fetch. Same origin as the
  // bundle, so no CORS; the `.gds` suffix is what the open path classifies on.
  const bytes = fixtureBytes(testInfo.file);
  await page.route(`**${FIXTURE_URL}`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/octet-stream",
      body: bytes,
    }),
  );

  // Open the editor on the fixture. The renderer starting hides #overlay; the camera seam
  // then appears once the document loads and the Start screen is dismissed.
  await page.goto(`/?view=editor&gds=${encodeURIComponent(FIXTURE_URL)}`);
  await expect(page.locator("#overlay"), "renderer starts").toBeHidden();
  await expect
    .poll(() => readCamera(page), {
      message: "the editor canvas never came up (the ?gds= fixture did not load)",
      timeout: 45_000,
    })
    .not.toBeNull();

  // Let the initial fit-to-design settle so the baseline is the resting camera.
  await page.waitForTimeout(500);
  const canvas = page.locator("#reticle-canvas");
  const box = await canvas.boundingBox();
  if (!box) throw new Error("no #reticle-canvas bounding box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;

  const client = await page.context().newCDPSession(page);
  const touch = (type: string, points: { x: number; y: number; id: number }[]) =>
    client.send("Input.dispatchTouchEvent", { type, touchPoints: points });

  // ---- Pinch: two fingers spread apart -> zoom changes. ----
  const zoomBefore = (await readCamera(page))!.pixels_per_dbu;
  {
    const start = 30;
    await touch("touchStart", [
      { x: cx - start, y: cy, id: 1 },
      { x: cx + start, y: cy, id: 2 },
    ]);
    for (let i = 1; i <= 8; i++) {
      const s = start + i * 18;
      await touch("touchMove", [
        { x: cx - s, y: cy, id: 1 },
        { x: cx + s, y: cy, id: 2 },
      ]);
      await page.waitForTimeout(30);
    }
    await touch("touchEnd", []);
  }
  await expect
    .poll(() => readCamera(page).then((c) => c?.pixels_per_dbu), {
      message: `a two-finger pinch did not change the zoom (still ${zoomBefore} px/DBU)`,
      timeout: 10_000,
    })
    .not.toBe(zoomBefore);

  // ---- Pan: two fingers drag together -> center changes. ----
  const centerBefore = (await readCamera(page))!;
  {
    const off = 30;
    await touch("touchStart", [
      { x: cx - off, y: cy, id: 1 },
      { x: cx + off, y: cy, id: 2 },
    ]);
    for (let i = 1; i <= 8; i++) {
      const dx = i * 14;
      const dy = i * 10;
      await touch("touchMove", [
        { x: cx - off + dx, y: cy + dy, id: 1 },
        { x: cx + off + dx, y: cy + dy, id: 2 },
      ]);
      await page.waitForTimeout(30);
    }
    await touch("touchEnd", []);
  }
  await expect
    .poll(
      () =>
        readCamera(page).then(
          (c) => c && (c.center_x !== centerBefore.center_x || c.center_y !== centerBefore.center_y),
        ),
      {
        message: `a two-finger drag did not pan the camera (still ${centerBefore.center_x},${centerBefore.center_y})`,
        timeout: 10_000,
      },
    )
    .toBe(true);
});

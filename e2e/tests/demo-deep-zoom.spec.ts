import { test, expect } from "@playwright/test";

// Headed deep-zoom coherence guard (packet v8.1.0-R, zoom-render lane).
//
// Root-cause finding (ADR 0100): across a full zoom ladder (fit -> 1e6 px/DBU) on both
// backends, the shipped start-screen examples do NOT show a "starry"/scattered canvas.
// Deep zoom shows a solid colored fill (inside a shape), the multi-layer structure when
// framed, or an empty field (over-zoomed past the integer-DBU geometry into a sub-DBU
// gap), all expected. The f32-precision regime that could scatter FAR-from-origin
// geometry is not exercised by these small, near-origin designs. So this guard proves
// the meaningful invariant: a zoom-in-to-max-and-back cycle keeps the app healthy and
// the design intact, on both backends, in a foreground tab.
//
// FOREGROUND DISCIPLINE (packet reproduction caveat): headed + visible before reading,
// so eframe's rAF loop is running.

type Stats = {
  applied_scene_shapes?: number;
  render_nonblank?: boolean;
  camera?: { pixels_per_dbu?: number };
};

function isFatal(text: string): boolean {
  const m = text.toLowerCase();
  return (
    m.includes("panic") ||
    m.includes("unreachable executed") ||
    m.includes("failed to start the reticle web app") ||
    m.includes("is missing a #reticle-canvas")
  );
}

function readStats(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

function ppd(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      (window as unknown as { __reticle_stats?: Stats }).__reticle_stats?.camera
        ?.pixels_per_dbu ?? 0,
  );
}

test("zoom-in-to-max-and-back keeps the design coherent", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/?e2e-example=sky130");
  await page.bringToFront();
  expect(
    await page.evaluate(() => document.visibilityState),
    "the page must be foreground/visible for egui to render (packet reproduction caveat)",
  ).toBe("visible");
  await expect(page.locator("#overlay")).toBeHidden();

  // The example is loaded and painting geometry at fit.
  await expect
    .poll(async () => (await readStats(page)).applied_scene_shapes ?? 0, {
      timeout: 30_000,
    })
    .toBeGreaterThan(0);
  const fit = (await readStats(page)) as Stats;
  const sceneFit = fit.applied_scene_shapes ?? 0;
  const ppdFit = fit.camera?.pixels_per_dbu ?? 0;
  expect(fit.render_nonblank, "geometry paints at fit").toBe(true);

  // Zoom all the way in with real wheel input (the actual zoom_about path). Probe the
  // wheel sign once, then drive well past the zoom cap.
  const box = await page.locator("#reticle-canvas").boundingBox();
  expect(box).not.toBeNull();
  const cx = box!.x + box!.width / 2;
  const cy = box!.y + box!.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.wheel(0, -240);
  await page.waitForTimeout(60);
  const dir = (await ppd(page)) >= ppdFit ? -240 : 240;
  for (let i = 0; i < 140; i++) {
    await page.mouse.move(cx, cy);
    await page.mouse.wheel(0, dir);
  }
  await page.waitForTimeout(150);
  const ppdDeep = await ppd(page);
  expect(ppdDeep, "wheel zoomed the camera deep in").toBeGreaterThan(ppdFit * 100);
  // The app is still alive and painting at deep zoom (not frozen/crashed).
  expect(
    (await readStats(page)).render_nonblank,
    "the renderer is still live at deep zoom",
  ).toBe(true);

  // Fit again (view.zoom_fit is bound to F): the whole design must come back intact.
  await page.locator("#reticle-canvas").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("f");
  await expect
    .poll(async () => (await readStats(page)).camera?.pixels_per_dbu ?? 0, {
      timeout: 15_000,
    })
    .toBeLessThan(ppdDeep / 10);

  const back = (await readStats(page)) as Stats;
  expect(back.render_nonblank, "geometry paints again after refit").toBe(true);
  // The zoom cycle did not corrupt or drop the document.
  expect(
    back.applied_scene_shapes,
    "the design is intact through the zoom cycle",
  ).toBe(sceneFit);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

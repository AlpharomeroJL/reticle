import { test, expect } from "@playwright/test";

// Headed first-run tour smoke (packet v8.1.0-R, tour lane, P2).
//
// Boots straight into the guided tour (?tour=1) and confirms the app renders it in a
// foreground tab. The tour overlay and its highlight boxes are egui-canvas-painted (the
// step state machine is unit-tested in tour.rs), so this guard proves the boot-into-tour
// path renders coherently and captures a screenshot for the polish review.

const SCRATCH =
  "C:/Users/jo312/AppData/Local/Temp/claude/D--dev-reticle/ec20cae6-97a4-4cfe-a967-6454c37bc1bb/scratchpad";

test("boots into the first-run tour and renders", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/?tour=1");
  await page.bringToFront();
  expect(await page.evaluate(() => document.visibilityState)).toBe("visible");
  // The app booted into the tour: the renderer started (overlay hidden) and the canvas
  // is present and sized. The tour overlay and its highlight boxes are egui-painted, so
  // the screenshot below is the polish-review artifact; the step state machine itself is
  // unit-tested in tour.rs.
  await expect(page.locator("#overlay")).toBeHidden();
  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box!.width).toBeGreaterThan(0);
  expect(box!.height).toBeGreaterThan(0);
  // Let a few frames paint the tour overlay before capture.
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${SCRATCH}/tour-${testInfo.project.name}.png` });

  const fatal = errors.filter((t) => {
    const m = t.toLowerCase();
    return (
      m.includes("panic") ||
      m.includes("unreachable executed") ||
      m.includes("failed to start the reticle web app")
    );
  });
  expect(fatal, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

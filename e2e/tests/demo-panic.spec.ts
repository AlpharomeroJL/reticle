import { test, expect } from "@playwright/test";

// Headed panic-overlay guard (packet v8.1.0-R, panic-safety-net lane).
//
// console_error_panic_hook alone routes a wasm panic only to console.error, so a runtime
// panic AFTER boot would leave a frozen or blank canvas with nothing on screen. The web
// entry now chains a hook that also writes the panic into the #overlay (cause + bundle
// hash + a reload link). This proves it end-to-end: boot the app so the overlay is hidden
// (the renderer started), deliberately trigger a real runtime panic via the ?e2e-panic=1
// hook (window.__reticleTestPanic), and assert the overlay reappears with a readable
// message, not a silent blank canvas.
//
// Runs headed because the app must actually boot and render (the overlay hides only after
// the renderer starts) before the post-boot panic is triggered.

const SCRATCH =
  "C:/Users/jo312/AppData/Local/Temp/claude/D--dev-reticle/ec20cae6-97a4-4cfe-a967-6454c37bc1bb/scratchpad";

test("a post-boot runtime panic surfaces a readable overlay", async ({
  page,
}, testInfo) => {
  await page.goto("/?view=editor&e2e-panic=1");
  await page.bringToFront();

  // The app booted and rendered: main.rs hides #overlay only after the renderer started.
  await expect(page.locator("#overlay")).toBeHidden();

  // The e2e trigger is installed under ?e2e-panic=1.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          typeof (window as unknown as { __reticleTestPanic?: unknown })
            .__reticleTestPanic,
      ),
    )
    .toBe("function");

  // Deliberately panic at runtime. The wasm trap propagates as a JS exception through the
  // closure call, so swallow it; the panic hook has already written the overlay by then.
  await page.evaluate(() => {
    try {
      (window as unknown as { __reticleTestPanic: () => void }).__reticleTestPanic();
    } catch {
      /* the wasm trap surfaces here after the hook ran; expected */
    }
  });

  // The overlay is visible again with a readable error, the bundle hash, and a reload link
  // (not a silent blank canvas).
  await expect(page.locator("#overlay")).toBeVisible();
  const status = page.locator("#status");
  await expect(status).toHaveClass(/error/);
  await expect(status).toContainText(/internal error/i);
  await expect(status).toContainText(/bundle/i);
  const reload = page.locator("#panic-reload");
  await expect(reload).toBeVisible();
  await expect(reload).toHaveAttribute("href", /.+/);

  await page.screenshot({ path: `${SCRATCH}/panic-overlay-${testInfo.project.name}.png` });
});

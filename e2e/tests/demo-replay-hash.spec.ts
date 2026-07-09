import { test, expect } from "@playwright/test";

// Headed replay-hash guard (packet v8.1.0-R, observability-seam lane).
//
// Plays the bundled demo transcript to the end on the REAL wasm runtime and asserts
// window.__reticle_stats.hash_check === "Match": the replayed document hashes to the
// transcript's recorded final hash. This converts the a8e45b1 fix's Rust store-test
// proof (document_hash made platform-independent via a FixedWidth hasher) into a
// browser-level assertion on the actual wasm build, which the fix session could not
// automate.
//
// The replay transport is GPU-canvas-painted, so there is no DOM button to click; the
// spec drives playback with the e2e-only ?e2e-autoplay=1 flag (the public ?view=replay
// landing still waits at Play). hash_check hashes the document model, not pixels, so it
// is backend-agnostic and this spec runs unchanged on both the headed-webgl2 and
// headed-webgpu projects.
//
// FOREGROUND DISCIPLINE (packet reproduction caveat): eframe pauses its
// requestAnimationFrame loop in a backgrounded or occluded tab, which leaves the canvas
// black and window.__reticle_stats null. That is NORMAL browser behavior, not a defect.
// This guard runs headed, brings the page to front, and asserts
// document.visibilityState === "visible" before reading, so a background black canvas
// can never be mistaken for a hash failure.

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

type DemoStats = {
  hash_check?: string;
};

function readStats(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: DemoStats }).__reticle_stats ?? {},
  );
}

test("the bundled demo replays to a matching hash on the real wasm runtime", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/?view=replay&e2e-autoplay=1");

  // Foreground discipline: make the tab active and confirm it is visible, so the rAF
  // loop is actually running before we read anything (a hidden tab pauses it and the
  // stats never advance).
  await page.bringToFront();
  expect(
    await page.evaluate(() => document.visibilityState),
    "the page must be foreground/visible for egui to render (packet reproduction caveat)",
  ).toBe("visible");

  // The renderer started: main.rs hides #overlay only after eframe's wgpu backend
  // (WebGPU or its WebGL2 fallback) initialized on the canvas.
  await expect(page.locator("#overlay")).toBeHidden();

  // Autoplay advances the transcript to the end. Poll until the verdict is terminal
  // (Match / Mismatch / Unverifiable); "Pending" (or an absent object before the first
  // frame) keeps waiting.
  await expect
    .poll(async () => (await readStats(page)).hash_check ?? "Pending", {
      message: "replay never reached a terminal hash verdict",
      timeout: 90_000,
    })
    .toMatch(/^(Match|Mismatch|Unverifiable)$/);

  const stats = (await readStats(page)) as DemoStats;
  expect(
    stats.hash_check,
    `replay hash verdict was "${stats.hash_check}" on project ${testInfo.project.name}; ` +
      `expected "Match" (a8e45b1 made document_hash platform-independent for wasm)`,
  ).toBe("Match");

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

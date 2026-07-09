import { test, expect } from "@playwright/test";

// Headed per-example render guard (packet v8.1.0-R, examples lane).
//
// Opens each of the three Start-screen examples and asserts it actually renders, on
// BOTH backends (the headed-webgl2 and headed-webgpu projects), in a FOREGROUND browser.
// The Start-screen gallery cards are egui-canvas-painted and not DOM-clickable, so the
// two embedded GDS designs open via the ?e2e-example= boot hook and the streamed die via
// the ?archive= path (the same streaming code path the "Streamed die" card uses),
// pointed at the local ranged fixture.
//
//   - Tiny Tapeout sample (tt03) and SKY130 inverter (sky130) are editor documents, so
//     assert __reticle_stats.applied_scene_shapes > 0 (the flattened renderable count,
//     which counts a hierarchical design's instance-expanded geometry; applied_shapes is
//     the top-cell DIRECT count, 0 for the hierarchical Tiny Tapeout sample) and
//     render_nonblank == true.
//   - The streamed die is a read-only archive with no editor document, so assert
//     archive_records_painted > 0 and render_nonblank == true.
//
// FOREGROUND DISCIPLINE (packet reproduction caveat): eframe pauses its rAF loop in a
// backgrounded or occluded tab, blanking the canvas and nulling __reticle_stats. Each
// case brings the page to front and asserts document.visibilityState === "visible"
// before reading, so a background black canvas is never mistaken for a white/empty pane.

const ARCHIVE_PORT = Number(process.env.ARCHIVE_PORT || 8082);
const ARCHIVE_URL = `http://127.0.0.1:${ARCHIVE_PORT}/fixture.rtla`;

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
  applied_shapes?: number;
  applied_scene_shapes?: number;
  render_nonblank?: boolean;
  archive_records_painted?: number;
};

function readStats(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: DemoStats }).__reticle_stats ?? {},
  );
}

// On the WebGL2 project, hide navigator.gpu so wgpu deterministically takes the WebGL2
// fallback. The WebGPU project leaves it in place to exercise the real adapter.
test.beforeEach(async ({ page }, testInfo) => {
  if (testInfo.project.name === "headed-webgl2") {
    await page.addInitScript(() => {
      try {
        delete (Navigator.prototype as { gpu?: unknown }).gpu;
      } catch {
        /* already absent */
      }
    });
  }
});

async function bootVisible(page: import("@playwright/test").Page, url: string) {
  await page.goto(url);
  // Foreground discipline: active + visible before reading, so the rAF loop is running.
  await page.bringToFront();
  expect(
    await page.evaluate(() => document.visibilityState),
    "the page must be foreground/visible for egui to render (packet reproduction caveat)",
  ).toBe("visible");
  // main.rs hides #overlay only after eframe's wgpu backend initialized on the canvas.
  await expect(page.locator("#overlay")).toBeHidden();
}

for (const { id, label } of [
  { id: "tt03", label: "Tiny Tapeout sample" },
  { id: "sky130", label: "SKY130 inverter cell" },
]) {
  test(`embedded example "${label}" renders geometry`, async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    page.on("pageerror", (e) => errors.push(String(e)));

    await bootVisible(page, `/?e2e-example=${id}`);

    await expect
      .poll(async () => (await readStats(page)).applied_scene_shapes ?? 0, {
        message: `${label} applied no scene shapes on ${testInfo.project.name}`,
        timeout: 30_000,
      })
      .toBeGreaterThan(0);

    const stats = (await readStats(page)) as DemoStats;
    expect(
      stats.render_nonblank,
      `${label} did not paint geometry (render_nonblank) on ${testInfo.project.name}`,
    ).toBe(true);

    const fatals = errors.filter(isFatal);
    expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
  });
}

test("streamed die archive renders resident records", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await bootVisible(page, `/?archive=${encodeURIComponent(ARCHIVE_URL)}`);

  await expect
    .poll(async () => (await readStats(page)).archive_records_painted ?? 0, {
      message: `streamed die painted no records on ${testInfo.project.name}`,
      timeout: 30_000,
    })
    .toBeGreaterThan(0);

  const stats = (await readStats(page)) as DemoStats;
  expect(
    stats.render_nonblank,
    `streamed die did not paint geometry (render_nonblank) on ${testInfo.project.name}`,
  ).toBe(true);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

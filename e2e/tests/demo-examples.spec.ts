import { test, expect } from "@playwright/test";

// Headed per-example render guard (packet v8.1.0-R, examples lane).
//
// Opens each of the three Start-screen examples in a FOREGROUND browser on BOTH backends
// (headed-webgl2 and headed-webgpu) and asserts it renders CORRECTLY, not merely
// "not blank". The gallery cards are egui-canvas-painted and not DOM-clickable, so the
// two embedded GDS designs open via the ?e2e-example= boot hook and the streamed die via
// the ?archive= path (the streaming code path the "Streamed die" card uses), pointed at
// the local ranged fixture.
//
// STRUCTURE, not just presence (the reported bug was a solid WHITE blob: the examples
// opened WITHOUT their SKY130 technology, so every layer drew an opaque default fill and
// they overpainted to one color). Each embedded example must:
//   (a) have its technology grafted, so its layers are NAMED, __reticle_stats.named_layers
//       is well above zero (a bare import has 0 named, only "L#D#" placeholders);
//   (b) paint MULTIPLE distinct colors, a color histogram of the canvas has more than one
//       significant non-background bucket, so a flat single-color fill FAILS;
//   (c) apply geometry: applied_scene_shapes > 0 and render_nonblank == true.
// The streamed die is a read-only archive (no editor document / layer table), so it uses
// archive_records_painted > 0 plus the same multi-color histogram check.
//
// FOREGROUND DISCIPLINE (packet reproduction caveat): eframe pauses its rAF loop in a
// backgrounded/occluded tab, blanking the canvas and nulling __reticle_stats. Each case
// brings the page to front and asserts document.visibilityState === "visible".

const ARCHIVE_PORT = Number(process.env.ARCHIVE_PORT || 8082);
const ARCHIVE_URL = `http://127.0.0.1:${ARCHIVE_PORT}/fixture.rtla`;
const SCRATCH =
  "C:/Users/jo312/AppData/Local/Temp/claude/D--dev-reticle/ec20cae6-97a4-4cfe-a967-6454c37bc1bb/scratchpad";

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
  applied_scene_shapes?: number;
  named_layers?: number;
  render_nonblank?: boolean;
  archive_records_painted?: number;
};

function readStats(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: DemoStats }).__reticle_stats ?? {},
  );
}

// Count the "significant" distinct color buckets on the canvas: screenshot the
// #reticle-canvas (Playwright captures the real composited pixels, bypassing the wgpu
// no-readback limitation), then decode that PNG in-page via an Image element (a decoded
// image draws to a 2D canvas cleanly, unlike the wgpu canvas) and histogram colors into
// 5-bit-per-channel buckets. Returns the number of buckets covering >= 0.5% of pixels.
// A correct multi-layer render has several (background + layer colors + alpha blends); a
// flat single-color fill has one, and a two-tone blob (background + one blob) has two.
async function significantColorBuckets(
  page: import("@playwright/test").Page,
): Promise<number> {
  const png = await page.locator("#reticle-canvas").screenshot();
  const b64 = png.toString("base64");
  return page.evaluate(async (data) => {
    const img = new Image();
    await new Promise<void>((res, rej) => {
      img.onload = () => res();
      img.onerror = () => rej(new Error("decode"));
      img.src = "data:image/png;base64," + data;
    });
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const ctx = c.getContext("2d")!;
    ctx.drawImage(img, 0, 0);
    const { data: px } = ctx.getImageData(0, 0, c.width, c.height);
    const total = c.width * c.height;
    const counts = new Map<number, number>();
    for (let i = 0; i < px.length; i += 4) {
      const key =
        ((px[i] >> 3) << 10) | ((px[i + 1] >> 3) << 5) | (px[i + 2] >> 3);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    let significant = 0;
    for (const n of counts.values()) if (n >= total * 0.005) significant++;
    return significant;
  }, b64);
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
  await page.bringToFront();
  expect(
    await page.evaluate(() => document.visibilityState),
    "the page must be foreground/visible for egui to render (packet reproduction caveat)",
  ).toBe("visible");
  await expect(page.locator("#overlay")).toBeHidden();
}

for (const { id, label } of [
  { id: "tt03", label: "Tiny Tapeout sample" },
  { id: "sky130", label: "SKY130 inverter cell" },
]) {
  test(`embedded example "${label}" renders named, colored layers`, async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    page.on("pageerror", (e) => errors.push(String(e)));

    await bootVisible(page, `/?e2e-example=${id}`);

    // Geometry applied.
    await expect
      .poll(async () => (await readStats(page)).applied_scene_shapes ?? 0, {
        message: `${label} applied no scene shapes on ${testInfo.project.name}`,
        timeout: 30_000,
      })
      .toBeGreaterThan(0);

    const stats = (await readStats(page)) as DemoStats;
    // (a) Technology grafted: layers are NAMED, not "L#D#" placeholders. A bare import
    // (the white-blob bug) has 0 named layers.
    expect(
      stats.named_layers ?? 0,
      `${label} opened without a named technology (named_layers=${stats.named_layers}) on ${testInfo.project.name}: the layermap was not applied`,
    ).toBeGreaterThanOrEqual(5);
    // (c) The frame painted geometry.
    expect(stats.render_nonblank, `${label} render_nonblank`).toBe(true);

    // (b) The canvas shows MULTIPLE colors, not a flat single-color blob. A correct
    // SKY130 render has the background plus several colored, alpha-blended layers.
    await page.screenshot({ path: `${SCRATCH}/example-${id}-${testInfo.project.name}.png` });
    const buckets = await significantColorBuckets(page);
    // eslint-disable-next-line no-console
    console.log(`COLORLOG ${id} ${testInfo.project.name}: significantBuckets=${buckets} named_layers=${stats.named_layers}`);
    expect(
      buckets,
      `${label} canvas is a flat/near-flat fill (only ${buckets} significant colors) on ${testInfo.project.name}: layers overpainted, layermap not applied`,
    ).toBeGreaterThanOrEqual(3);

    const fatals = errors.filter(isFatal);
    expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
  });
}

test("streamed die archive renders resident, colored records", async ({
  page,
}, testInfo) => {
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
  expect(stats.render_nonblank, "streamed die render_nonblank").toBe(true);

  await page.screenshot({ path: `${SCRATCH}/example-die-${testInfo.project.name}.png` });
  const buckets = await significantColorBuckets(page);
  // eslint-disable-next-line no-console
  console.log(`COLORLOG die ${testInfo.project.name}: significantBuckets=${buckets}`);
  expect(
    buckets,
    `streamed die canvas is a flat/near-flat fill (only ${buckets} significant colors) on ${testInfo.project.name}`,
  ).toBeGreaterThanOrEqual(3);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

import { test, expect } from "@playwright/test";

// Served-archive streaming e2e (lane v8-2e).
//
// Opens the bundle with `?archive=<url>` pointing at a committed `.rtla` fixture served
// over HTTP Range by serve-archive.mjs (a LOCAL ranged server, regardless of any cloud
// hosting — a Wave 2 gate requirement). The bundle streams the archive into a read-only
// DocHost::Streamed and paints the die with progressive residency. This asserts, via the
// `window.__reticle_stats` seam the app publishes each frame, that tiles actually become
// resident over the network and records paint on the canvas.
//
// The fixture is served from a DIFFERENT port than the bundle (8082 vs 8080), so every
// tile fetch is cross-origin; the ranged server answers permissive CORS (including the
// OPTIONS preflight the `Range` request header triggers).

const ARCHIVE_PORT = Number(process.env.ARCHIVE_PORT || 8082);
const ARCHIVE_URL = `http://127.0.0.1:${ARCHIVE_PORT}/fixture.rtla`;

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

// The stats the app publishes to window.__reticle_stats for this browse.
type ArchiveStats = {
  archive_tiles_resident?: number;
  archive_bytes_fetched?: number;
  archive_records_painted?: number;
  archive_file_size?: number;
  archive_working_set_bytes?: number;
  tiles_resident?: number;
};

test.beforeEach(async ({ page }) => {
  // The served-archive project runs on headless Chromium (no WebGPU adapter); force the
  // WebGL2 fallback explicitly so the boot path is deterministic on any host.
  await page.addInitScript(() => {
    try {
      delete (Navigator.prototype as { gpu?: unknown }).gpu;
    } catch {
      /* already absent */
    }
  });
});

test("streams a served .rtla over HTTP Range and paints resident tiles", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`/?archive=${encodeURIComponent(ARCHIVE_URL)}`);

  // The renderer started (main.rs hides #overlay only after eframe's wgpu backend
  // initialized on the canvas) — the genuine "it rendered" signal.
  await expect(page.locator("#overlay")).toBeHidden();

  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box, "canvas has a bounding box").not.toBeNull();
  expect(box!.width).toBeGreaterThan(0);
  expect(box!.height).toBeGreaterThan(0);

  // Tiles fetched over the network become resident. Poll the stats the app republishes
  // every frame; the residency pass fetches the covering tiles and drains them in.
  await expect
    .poll(
      () => page.evaluate(() => (window as unknown as { __reticle_stats?: ArchiveStats }).__reticle_stats?.archive_tiles_resident ?? 0),
      { message: "no archive tiles became resident", timeout: 30_000 },
    )
    .toBeGreaterThan(0);

  const stats = (await page.evaluate(
    () => (window as unknown as { __reticle_stats?: ArchiveStats }).__reticle_stats ?? {},
  )) as ArchiveStats;

  // Bytes actually moved over the wire (the ranged tile fetches), and records painted on
  // the canvas (the streamed die is visible, not a blank canvas).
  expect(stats.archive_bytes_fetched ?? 0, "bytes were fetched over Range").toBeGreaterThan(0);
  expect(stats.archive_records_painted ?? 0, "records painted on the canvas").toBeGreaterThan(0);
  // The ranged size probe reported the archive's total size.
  expect(stats.archive_file_size ?? 0, "archive total size was probed").toBeGreaterThan(0);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

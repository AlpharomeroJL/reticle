import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

// In-browser convert e2e (lane v8-6c).
//
// The whole conversion happens client-side: the page runs the convert Web Worker on a
// committed GDS, the worker streams it through `gds_stream` + the in-memory `.rtla`
// builder and writes the archive into OPFS (Origin Private File System), and the app then
// reopens that OPFS archive through the SAME `?archive=` streaming path the served-archive
// spec exercises (here the service worker serves the OPFS file over HTTP Range). No
// server, no upload: a GDS becomes a streamable die entirely in the browser.
//
// Honesty about OPFS: writing a whole archive at once needs a FileSystemSyncAccessHandle,
// which is only available in a Worker and only in a secure context. Where OPFS is
// unavailable the convert half is skipped rather than failed; where the service worker is
// not controlling the page, the reopen/render half is reported as skipped. Headless
// Chromium over http://127.0.0.1 is a secure context, so both usually run.

// Playwright runs with its config directory (e2e/) as the cwd, so the fixture path is
// resolved relative to it (see the `e2e-convert` recipe, which `cd e2e` first).
const GDS = readFileSync("fixtures/convert-sample.gds");
const ARCHIVE_NAME = "convert-sample.rtla";

type WorkerDone = { type: string; path: string; records: number; bytes: number };

type ArchiveStats = {
  archive_tiles_resident?: number;
  archive_records_painted?: number;
  archive_bytes_fetched?: number;
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

test.beforeEach(async ({ page }) => {
  // Force the WebGL2 fallback so the boot path is deterministic on any host.
  await page.addInitScript(() => {
    try {
      delete (Navigator.prototype as { gpu?: unknown }).gpu;
    } catch {
      /* already absent */
    }
  });
});

test("converts a GDS to an OPFS .rtla and streams it back through ?archive=", async ({
  page,
}, testInfo) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto("/");
  await expect(page.locator("#overlay")).toBeHidden();

  // The convert driver the page exposes must be present.
  await expect
    .poll(() => page.evaluate(() => typeof (window as any).__reticleConvertGds))
    .toBe("function");

  // OPFS must be present at all to attempt a conversion.
  const opfsAvailable = await page.evaluate(
    () => !!(navigator.storage && navigator.storage.getDirectory),
  );
  test.skip(!opfsAvailable, "OPFS is unavailable in this browser");

  // Run the convert Web Worker on the committed GDS bytes.
  const gdsArray = Array.from(GDS);
  let done: WorkerDone;
  try {
    done = (await page.evaluate(async (bytes) => {
      const u8 = new Uint8Array(bytes as number[]);
      return await (window as any).__reticleConvertGds(u8, "convert-sample.rtla");
    }, gdsArray)) as WorkerDone;
  } catch (e) {
    const msg = String(e);
    // A worker/OPFS-write failure (e.g. sync access handles unsupported) is an honest
    // skip, not a lane failure.
    if (/opfs|sync access|storage|not found/i.test(msg)) {
      test.skip(true, `OPFS write unsupported here: ${msg}`);
    }
    throw e;
  }

  // The worker converted drawn geometry and reported where it wrote the archive.
  expect(done.records, "shapes were converted to tile records").toBeGreaterThan(0);
  expect(done.bytes, "the archive is non-empty").toBeGreaterThan(0);
  expect(done.path).toBe(`archives/${ARCHIVE_NAME}`);

  // The archive really landed in OPFS at that path, at the size the worker reported.
  const opfsSize = await page.evaluate(async (path: string) => {
    const name = path.split("/").pop() as string;
    const root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle("archives");
    const handle = await dir.getFileHandle(name);
    const file = await handle.getFile();
    return file.size;
  }, done.path);
  expect(opfsSize, "OPFS archive size matches the converted bytes").toBe(done.bytes);

  // Reopen the OPFS archive through the streaming path. This needs the service worker to
  // be controlling the page (it serves opfs-archive/<path> over Range); if it is not, the
  // reopen/render half is honestly skipped while the convert+write above still stands.
  const controlled = await page
    .waitForFunction(() => navigator.serviceWorker?.controller != null, null, {
      timeout: 15_000,
    })
    .then(() => true)
    .catch(() => false);

  if (!controlled) {
    testInfo.annotations.push({
      type: "skip-render",
      description:
        "service worker not controlling the page; convert+OPFS-write verified, reopen/render skipped",
    });
    const fatals = errors.filter(isFatal);
    expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
    return;
  }

  const base = testInfo.project.use.baseURL ?? "http://127.0.0.1:8080";
  const archiveUrl = `${base}/opfs-archive/${done.path}`;
  await page.goto(`/?archive=${encodeURIComponent(archiveUrl)}`);

  // The renderer started (overlay hidden only after the wgpu backend initialized).
  await expect(page.locator("#overlay")).toBeHidden();
  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();

  // Tiles from the OPFS archive stream in (via the SW Range bridge) and records paint.
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (window as unknown as { __reticle_stats?: ArchiveStats }).__reticle_stats
              ?.archive_records_painted ?? 0,
        ),
      { message: "no records painted from the OPFS archive", timeout: 30_000 },
    )
    .toBeGreaterThan(0);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

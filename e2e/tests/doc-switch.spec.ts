import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";

// Document-switch acceptance for RC1 (commit "exit archive browse and complete the reset
// on document open"). This is RC1's browser-level bar: it cannot live as a headless unit
// test because the bug is in what the LIVE app paints and which panels it enables after a
// real in-session open, not in any pure function.
//
// The flow a real visitor takes, headless:
//   1. Open document A by streaming a served `.rtla` over `?archive=` (a read-only
//      DocHost::Streamed die). The canvas paints archive tiles; the batch verify/export
//      and layer panels are suppressed because a streamed archive has no editable document.
//   2. Open document B in-session by DRAGGING A GDS ONTO THE CANVAS -- the same egui
//      dropped-files path a user drags a file through, NOT a page reload (a reload would
//      spin up a fresh wasm instance and never exercise the stale-state bug) and NOT the
//      debug-only `?e2e-example=` hook (no visitor takes that path).
//   3. Assert the RENDERED, interactive document is B: the editor scene is B's geometry,
//      the layer panel is B's, and -- the linchpin -- Run DRC is ENABLED again.
//
// Why "Run DRC enabled" is the RC1 linchpin, and why this goes RED without the fix:
//   `run_drc` (and the whole batch verify/export path) early-returns while
//   `self.archive.is_some()` -- a streamed archive has no editable document to check
//   (commit "disable batch DRC and SPICE export while browsing a streamed archive").
//   Opening a document is the ONLY event that clears `self.archive`, and that clear lives
//   inside `install_document` under the RC1 fix. WITHOUT RC1, `install_document` still
//   swaps in B's document (so the editor-scene and layer stats below already read B), but
//   `self.archive` stays `Some`, so the canvas keeps drawing the streamed tiles, the layer
//   panel stays suppressed, and `run_drc` stays a no-op. WITH RC1, `self.archive` becomes
//   `None`, the canvas draws B, the panels return, and a dispatched `verify.drc_run`
//   actually runs -- flipping `drc_ran` true. So `drc_ran === true` after the switch is a
//   signal ONLY the RC1 fix can produce.
//
// All stats come from the `window.__reticle_stats` seam the app republishes each frame,
// and `window.__reticle_e2e_dispatch` (both present in the debug e2e Trunk build `just e2e`
// builds, absent from the measured release bundle). Runs headless (WebGL2 fallback, wgpu
// with WebGPU hidden) alongside serve-dist (8080) and the ranged serve-archive (8082).

const ARCHIVE_PORT = Number(process.env.ARCHIVE_PORT || 8082);
const ARCHIVE_URL = `http://127.0.0.1:${ARCHIVE_PORT}/fixture.rtla`;

type Stats = {
  applied_scene_shapes?: number;
  applied_layers?: number;
  named_layers?: number;
  render_nonblank?: boolean;
  drc_ran?: boolean;
  archive_records_painted?: number;
  last_command_id?: string;
};

function stats(page: Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

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

test.beforeEach(async ({ page }) => {
  // Force the WebGL2 fallback on any host: delete navigator.gpu so `"gpu" in navigator`
  // and any adapter request both see it absent (a getter returning undefined would still
  // pass the `in` test). On a host without WebGPU this is a harmless no-op.
  await page.addInitScript(() => {
    try {
      delete (Navigator.prototype as { gpu?: unknown }).gpu;
    } catch {
      /* already absent or non-configurable */
    }
  });
});

test("archive browse then in-session open switches the rendered document to B (RC1)", async ({
  page,
}, testInfo) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(String(e)));

  // --- 1. Open document A: stream a served `.rtla` over `?archive=`. ---
  await page.goto(`/?archive=${encodeURIComponent(ARCHIVE_URL)}`);
  // main.rs hides #overlay only after eframe's wgpu backend initialized on the canvas: the
  // genuine "it rendered" signal, not a DOM-ready check.
  await expect(page.locator("#overlay")).toBeHidden();

  // The streamed die is actually painting on the canvas (records drawn from resident tiles).
  await expect
    .poll(async () => (await stats(page)).archive_records_painted ?? 0, {
      message: "the streamed archive never painted any records",
      timeout: 30_000,
    })
    .toBeGreaterThan(0);
  const a = await stats(page);
  expect(a.archive_records_painted ?? 0, "archive A is painting tiles").toBeGreaterThan(0);

  // --- 2. Open document B in-session by dragging a GDS onto the canvas. ---
  const gds = readFileSync(
    join(dirname(testInfo.file), "..", "fixtures", "convert-sample.gds"),
  ).toString("base64");
  await page.evaluate((b64) => {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const file = new File([bytes], "document-B.gds", { type: "application/octet-stream" });
    const canvas = document.querySelector("#reticle-canvas") as HTMLElement;
    const dt = new DataTransfer();
    dt.items.add(file);
    const init: DragEventInit = { bubbles: true, cancelable: true, composed: true, dataTransfer: dt };
    // egui/eframe listens for the HTML5 drag sequence on the canvas and reads the dropped
    // file's bytes; dragover must be delivered so the drop is accepted.
    canvas.dispatchEvent(new DragEvent("dragenter", init));
    canvas.dispatchEvent(new DragEvent("dragover", init));
    canvas.dispatchEvent(new DragEvent("drop", init));
  }, gds);

  // The drop reads the file bytes asynchronously (FileReader), then install_document swaps
  // in B's document. Wait until the live editable scene is no longer A's boot demo.
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, {
      message: "the dropped document B never became the live editable document",
      timeout: 30_000,
    })
    .not.toBe(a.applied_scene_shapes ?? -1);

  // --- 3. Assert the rendered, interactive document is B. ---
  const b = await stats(page);

  // The editor is painting real geometry this frame on the DOCUMENT render path (the
  // flattened editable scene has shapes and the camera is live). This is the app's own
  // "not a black canvas" signal.
  expect(b.render_nonblank, "the editor is painting B on the document render path").toBe(true);
  expect(b.applied_scene_shapes ?? 0, "B's editable scene has geometry").toBeGreaterThan(0);
  expect(
    b.applied_scene_shapes,
    "the live document changed from A to B (a different scene)",
  ).not.toBe(a.applied_scene_shapes);

  // The layer panel is B's: B carries its own layer rows, distinct from A's boot-demo set.
  expect(b.applied_layers ?? 0, "B contributes its own layer rows").toBeGreaterThan(0);
  expect(b.applied_layers, "the layer panel is B's, not A's").not.toBe(a.applied_layers);

  // RC1 linchpin: opening B ended the archive browse. Run DRC is only reachable when there
  // is an editable document (i.e. `self.archive` was cleared). Opening B clears any prior
  // DRC results, so drc_ran starts false; dispatch verify.drc_run and confirm it actually
  // ran. This assertion is RED without RC1: while browsing an archive `run_drc` is a no-op,
  // so drc_ran would stay false forever.
  expect(b.drc_ran ?? true, "DRC results were cleared when B opened").toBe(false);
  const dispatched = await page.evaluate(
    () =>
      (window as unknown as { __reticle_e2e_dispatch?: (id: string) => boolean }).__reticle_e2e_dispatch?.(
        "verify.drc_run",
      ) ?? false,
  );
  expect(dispatched, "the verify.drc_run command id is live in the registry bridge").toBe(true);
  await expect
    .poll(async () => (await stats(page)).drc_ran ?? false, {
      message:
        "Run DRC never executed after the switch: the archive browse did not end " +
        "(RC1 regression -- self.archive stayed Some, so run_drc is guarded off)",
      timeout: 10_000,
    })
    .toBe(true);

  // The canvas is a real, sized element (the document render surface, not a 300x150 stub).
  const canvas = page.locator("#reticle-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box, "canvas has a bounding box").not.toBeNull();
  expect(box!.width).toBeGreaterThan(0);
  expect(box!.height).toBeGreaterThan(0);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

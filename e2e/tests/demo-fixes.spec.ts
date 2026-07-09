import { test, expect } from "@playwright/test";

// v8.1-REGRESSION Stage 5: verify the two fixed classes headed, driving REAL
// controls in a foreground browser and reading __reticle_stats.
//   Class A: command palette activate (Enter + row-click).
//   Class B: canvas draw (rectangle by drag) + marquee-select by drag.
// Reproduction law: headed + foreground only.

const SCRATCH =
  "C:/Users/jo312/AppData/Local/Temp/claude/D--dev-reticle/ec20cae6-97a4-4cfe-a967-6454c37bc1bb/scratchpad/regression";

type Stats = Record<string, unknown>;
function stats(page: import("@playwright/test").Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

test("FIXED CLASSES A+B (sky130 editor, headed)", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push("PAGEERR " + String(e)));

  await page.goto("/?e2e-example=sky130");
  await page.bringToFront();
  await expect(page.locator("#overlay")).toBeHidden();
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  const canvas = page.locator("#reticle-canvas");
  const box = await canvas.boundingBox();
  const cx = box!.x + box!.width / 2;
  const cy = box!.y + box!.height / 2;

  // Give the app focus on empty canvas, reset to Select.
  await page.mouse.click(box!.x + 20, box!.y + 20);
  await page.keyboard.press("Escape");
  await page.keyboard.press("v");
  await page.waitForTimeout(150);

  // --- Class A.1: palette activate via ENTER ---
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(300);
  await page.keyboard.type("measure tool");
  await page.waitForTimeout(200);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(250);
  const aEnter = await stats(page);
  // eslint-disable-next-line no-console
  console.log(`FIX palette.enter last_command_id=${aEnter.last_command_id} tool=${aEnter.active_tool}`);
  expect(aEnter.active_tool, "palette Enter runs the top hit").toBe("Measure");

  // --- Class A.2: palette activate via ROW CLICK ---
  await page.keyboard.press("Escape");
  await page.waitForTimeout(120);
  await page.keyboard.press("v"); // back to Select first
  await page.waitForTimeout(120);
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(250);
  await page.keyboard.type("rectangle");
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${SCRATCH}/fix-palette-rows.png` });
  // The palette opens at default_pos (200,120); the first result row ("Rectangle
  // tool", Draw group) sits at ~(265,259). Click it.
  await page.mouse.click(265, 259);
  await page.waitForTimeout(250);
  const aRow = await stats(page);
  // eslint-disable-next-line no-console
  console.log(`FIX palette.rowclick last_command_id=${aRow.last_command_id} tool=${aRow.active_tool}`);
  await page.screenshot({ path: `${SCRATCH}/fix-palette-after-row.png` });
  expect(aRow.active_tool, "palette row-click runs the row").toBe("Rect");
  const toolNow = aRow;
  // eslint-disable-next-line no-console
  console.log(`FIX tool before draw=${toolNow.active_tool}`);

  // --- Class B.1: draw a rectangle by DRAG ---
  const drawBefore = await stats(page);
  await page.mouse.move(cx - 120, cy - 80);
  await page.mouse.down();
  await page.mouse.move(cx - 40, cy - 20, { steps: 6 });
  await page.mouse.move(cx + 120, cy + 80, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const drawAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/fix-after-rect.png` });
  // eslint-disable-next-line no-console
  console.log(
    `FIX draw.rect shapes ${drawBefore.applied_scene_shapes}->${drawAfter.applied_scene_shapes} mutated=${drawAfter.last_command_mutated} tool=${drawAfter.active_tool}`,
  );
  expect(
    (drawAfter.applied_scene_shapes as number),
    "rect draw commits a new shape",
  ).toBeGreaterThan(drawBefore.applied_scene_shapes as number);

  // --- Class B.2: marquee-select by DRAG (Select tool) ---
  await page.keyboard.press("v");
  await page.waitForTimeout(120);
  // Clear selection by clicking empty space.
  await page.mouse.click(box!.x + 20, box!.y + 20);
  await page.waitForTimeout(120);
  const selBefore = await stats(page);
  // Drag a band across the design center to catch several shapes.
  await page.mouse.move(cx - 160, cy - 120);
  await page.mouse.down();
  await page.mouse.move(cx - 40, cy, { steps: 8 });
  await page.mouse.move(cx + 160, cy + 120, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const selAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/fix-after-marquee.png` });
  // eslint-disable-next-line no-console
  console.log(
    `FIX marquee selection ${selBefore.selection_count}->${selAfter.selection_count} errors=${errors.length}`,
  );
  expect(
    (selAfter.selection_count as number),
    "marquee selects shapes inside the band",
  ).toBeGreaterThan(0);

  expect(errors, "no console errors during the fixed-class drive").toEqual([]);
});

test("DRAW polygon + path (sky130 editor, headed)", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push("PAGEERR " + String(e)));

  await page.goto("/?e2e-example=sky130");
  await page.bringToFront();
  await expect(page.locator("#overlay")).toBeHidden();
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  const box = await page.locator("#reticle-canvas").boundingBox();
  const cx = box!.x + box!.width / 2;
  const cy = box!.y + box!.height / 2;
  await page.mouse.click(box!.x + 20, box!.y + 20);
  await page.keyboard.press("Escape");

  // Reliable tool set via the (now-working) palette Enter.
  async function setTool(query: string, expected: string) {
    await page.keyboard.press("v");
    await page.waitForTimeout(120);
    await page.keyboard.press("Control+p");
    await page.waitForTimeout(200);
    await page.keyboard.type(query);
    await page.waitForTimeout(200);
    await page.keyboard.press("Enter");
    await page.waitForTimeout(200);
    expect((await stats(page)).active_tool, `set tool ${expected}`).toBe(expected);
  }

  // Place vertices as paced single clicks (350ms apart, spatially separated) so egui
  // never coalesces two into a double-click, then finish with Enter.
  async function placeAndFinish(pts: [number, number][]) {
    for (const [x, y] of pts) {
      await page.mouse.click(x, y);
      await page.waitForTimeout(350);
    }
    await page.keyboard.press("Enter");
    await page.waitForTimeout(250);
  }

  // --- Polygon ---
  await setTool("polygon", "Polygon");
  const polyBefore = await stats(page);
  await placeAndFinish([
    [cx - 110, cy - 90],
    [cx + 110, cy - 90],
    [cx + 110, cy + 90],
    [cx - 110, cy + 90],
  ]);
  const polyAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/fix-after-polygon.png` });
  // eslint-disable-next-line no-console
  console.log(`FIX draw.polygon shapes ${polyBefore.applied_scene_shapes}->${polyAfter.applied_scene_shapes}`);
  expect(
    polyAfter.applied_scene_shapes as number,
    "polygon commits a new shape",
  ).toBeGreaterThan(polyBefore.applied_scene_shapes as number);

  // --- Path ---
  await setTool("path", "Path");
  const pathBefore = await stats(page);
  await placeAndFinish([
    [cx - 130, cy + 30],
    [cx, cy - 60],
    [cx + 130, cy + 30],
  ]);
  const pathAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/fix-after-path.png` });
  // eslint-disable-next-line no-console
  console.log(
    `FIX draw.path shapes ${pathBefore.applied_scene_shapes}->${pathAfter.applied_scene_shapes} errors=${errors.length}`,
  );
  expect(
    pathAfter.applied_scene_shapes as number,
    "path commits a new shape",
  ).toBeGreaterThan(pathBefore.applied_scene_shapes as number);

  expect(errors, "no console errors during polygon/path draw").toEqual([]);
});

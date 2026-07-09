import { test, expect } from "@playwright/test";

// v8.1-REGRESSION matrix (headed discovery). Drives REAL controls in a foreground
// headed browser and reads the __reticle_stats regression seams to record, per
// capability, whether the action FIRED and had an EFFECT. Emits MATRIXROW lines the
// harness collects into scratch/regression/matrix.md.
//
// Reproduction law: headed + foreground only. Never headless.

const SCRATCH =
  "C:/Users/jo312/AppData/Local/Temp/claude/D--dev-reticle/ec20cae6-97a4-4cfe-a967-6454c37bc1bb/scratchpad/regression";

type Stats = Record<string, unknown>;
function stats(page: import("@playwright/test").Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

test("REGRESSION MATRIX (sky130 editor, headed)", async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  const rows: string[] = [];
  const row = (cap: string, ok: boolean, note: string) => {
    rows.push(`${ok ? "WORKS" : "BROKEN"} | ${cap} | ${note}`);
    // eslint-disable-next-line no-console
    console.log(`MATRIXROW ${ok ? "WORKS " : "BROKEN"} | ${cap} | ${note}`);
  };
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
  const s0 = await stats(page);
  row("example.open (sky130 colored)", (s0.applied_scene_shapes as number) > 0 && (s0.named_layers as number) >= 5, `shapes=${s0.applied_scene_shapes} named_layers=${s0.named_layers}`);

  // Give the app focus without selecting: click empty top-left of the canvas area.
  await page.mouse.click(box!.x + 20, box!.y + 20);
  await page.keyboard.press("Escape");

  // --- Keyboard-shortcut capabilities (chorded commands -> dispatch) ---
  async function chord(cap: string, keys: string, expectId: string, effect: (before: Stats, after: Stats) => [boolean, string]) {
    const before = await stats(page);
    await page.keyboard.press(keys);
    await page.waitForTimeout(200);
    const after = await stats(page);
    const fired = after.last_command_id === expectId;
    const [had, note] = effect(before, after);
    row(cap, fired && had, `keys=${keys} last_command_id=${after.last_command_id} ${note}`);
  }

  // Switch away from Select first so pressing V actually dispatches tool.select
  // (pressing a tool's chord while it is already active is a no-op).
  await page.keyboard.press("m");
  await page.waitForTimeout(120);
  await chord("tool.select (V)", "v", "tool.select", (_b, a) => [a.active_tool === "Select", `tool=${a.active_tool}`]);
  await chord("tool.measure (M)", "m", "tool.measure", (_b, a) => [a.active_tool === "Measure", `tool=${a.active_tool}`]);
  await chord("tool.pan (S)", "s", "tool.pan", (_b, a) => [a.active_tool === "Pan", `tool=${a.active_tool}`]);
  await chord("view.zoom_fit (F)", "f", "view.zoom_fit", (b, a) => {
    const bc = (b.camera as { pixels_per_dbu?: number })?.pixels_per_dbu ?? 0;
    const ac = (a.camera as { pixels_per_dbu?: number })?.pixels_per_dbu ?? 0;
    return [ac > 0, `ppd ${bc}->${ac}`];
  });
  await chord("view.grid (Ctrl+G)", "Control+g", "view.grid", (_b, a) => [a.last_command_id === "view.grid", "toggled"]);
  await chord("view.labels (L)", "l", "view.labels", (_b, a) => [a.last_command_id === "view.labels", "toggled"]);
  await chord("view.minimap (N)", "n", "view.minimap", (_b, a) => [a.last_command_id === "view.minimap", "toggled"]);

  // --- Selection: click a shape (canvas Select tool) ---
  await page.keyboard.press("v"); // select tool
  await page.waitForTimeout(100);
  await page.mouse.click(cx, cy);
  await page.waitForTimeout(200);
  const selAfter = await stats(page);
  row("selection.click", (selAfter.selection_count as number) > 0, `selection_count=${selAfter.selection_count}`);

  // --- Palette: open (Ctrl+P) ---
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${SCRATCH}/matrix-palette-open.png` });
  // Palette open is visual; approximate by trying to run a command via Enter next.
  // --- Palette activate via Enter (type + Enter) ---
  const beforePal = await stats(page);
  await page.keyboard.type("measure tool");
  await page.waitForTimeout(200);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(250);
  const afterPalEnter = await stats(page);
  row(
    "palette.activate (Enter)",
    afterPalEnter.last_command_id === "tool.measure" && afterPalEnter.active_tool === "Measure",
    `last_command_id=${afterPalEnter.last_command_id} tool=${afterPalEnter.active_tool} (was ${beforePal.active_tool})`,
  );

  // Close palette if still open, reset to select tool.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);

  // --- Draw a rectangle: set DrawRect via palette row CLICK, then canvas drag ---
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(250);
  await page.keyboard.type("rectangle tool");
  await page.waitForTimeout(200);
  // Click the first result row ("Rectangle tool", Draw group). The palette opens at
  // default_pos (200,120); the first row sits at ~(265,259).
  await page.mouse.click(265, 259);
  await page.waitForTimeout(250);
  const afterRowClick = await stats(page);
  row(
    "palette.activate (row click)",
    afterRowClick.active_tool === "Rect",
    `tool=${afterRowClick.active_tool} last_command_id=${afterRowClick.last_command_id}`,
  );
  // Now draw with whatever tool is active.
  const drawBefore = await stats(page);
  await page.mouse.move(cx - 100, cy - 70);
  await page.mouse.down();
  await page.mouse.move(cx + 100, cy + 70, { steps: 10 });
  await page.mouse.up();
  await page.waitForTimeout(250);
  const drawAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/matrix-after-draw.png` });
  row(
    "edit.draw_rect (canvas drag)",
    (drawAfter.applied_scene_shapes as number) > (drawBefore.applied_scene_shapes as number),
    `shapes ${drawBefore.applied_scene_shapes}->${drawAfter.applied_scene_shapes} tool=${drawAfter.active_tool}`,
  );

  // --- Undo / Redo (Ctrl+Z / Ctrl+Y) ---
  const undoBefore = await stats(page);
  await page.keyboard.press("Control+z");
  await page.waitForTimeout(200);
  const undoAfter = await stats(page);
  row(
    "edit.undo (Ctrl+Z)",
    undoAfter.last_command_id === "edit.undo" && (undoAfter.undo_depth as number) < (undoBefore.undo_depth as number || 99),
    `undo_depth ${undoBefore.undo_depth}->${undoAfter.undo_depth} shapes ${undoBefore.applied_scene_shapes}->${undoAfter.applied_scene_shapes}`,
  );
  await page.keyboard.press("Control+y");
  await page.waitForTimeout(200);
  const redoAfter = await stats(page);
  row("edit.redo (Ctrl+Y)", redoAfter.last_command_id === "edit.redo", `redo last_command_id=${redoAfter.last_command_id}`);

  // --- Replay theater (open via palette, then play) ---
  await page.keyboard.press("Escape");
  await page.goto("/?view=replay&e2e-autoplay=1");
  await page.bringToFront();
  await expect(page.locator("#overlay")).toBeHidden();
  await page.waitForTimeout(500);
  await expect
    .poll(async () => (await stats(page)).replay_total ?? 0, { timeout: 15_000 })
    .toBeGreaterThan(0);
  await expect
    .poll(async () => (await stats(page)).hash_check ?? "Pending", { timeout: 60_000 })
    .toMatch(/^(Match|Mismatch|Unverifiable)$/);
  const rep = await stats(page);
  row(
    "replay.autoplay_to_end",
    rep.hash_check === "Match" && (rep.replay_step as number) > 0,
    `step=${rep.replay_step}/${rep.replay_total} hash=${rep.hash_check}`,
  );

  // Write the matrix and dump errors.
  // eslint-disable-next-line no-console
  console.log("MATRIXERRORS=" + errors.length);
  const fs = await import("fs");
  fs.writeFileSync(
    `${SCRATCH}/matrix-raw.txt`,
    rows.join("\n") + `\n\nconsole errors: ${errors.length}\n` + errors.slice(0, 30).join("\n"),
  );
});

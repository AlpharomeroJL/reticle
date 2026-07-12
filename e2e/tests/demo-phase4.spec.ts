import { test, expect } from "@playwright/test";

// Phase-4 (Reach) comprehensive headed pass. Drives the surfaces Phase 4 added in a
// FOREGROUND headed browser on BOTH backends and asserts each had an EFFECT via the
// __reticle_stats seams, plus a screenshot per state:
//
//   * draw tools BEYOND sky130: a rectangle draw on the Tiny Tapeout (tt03) example
//     AND on a blank document (?e2e-example=blank), asserting a shape LANDS
//     (applied_scene_shapes grows), the Gate-3-ledgered per-example + blank-doc growth.
//   * plugin manager browse: plugin.browse fires and reveals the Plugins section; the
//     browser build shows the honest desktop-only disclaimer (ADR 0120). Screenshot it.
//   * image underlay: underlay.load drives the browser file picker with a committed
//     fixture photo; the browser decode path (createImageBitmap, ADR 0118) loads it,
//     asserted by the additive underlay_loaded seam. The under-geometry paint ORDER is
//     unit-proven (draw_underlay_paints_before_the_grid...); here the browser decode+load
//     path is what a unit test cannot reach.
//   * embed mode: ?embed=1 renders the design in embed chrome (embed seam true, geometry
//     still paints), and embed.toggle flips the mode live.
//
// Reproduction law (packet caveat): headed + foreground only; eframe pauses its rAF loop
// when the tab is occluded, blanking the canvas and nulling the seam.

const SCRATCH = "../scratch/logs/gate4-interactive";
const UNDERLAY_FIXTURE = "../crates/reticle-app/tests/fixtures/underlay/tiny.png";

type Stats = Record<string, unknown>;
function stats(page: import("@playwright/test").Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

// Dispatch a command by opening the palette, typing a distinctive fragment of its label,
// and pressing Enter (runs the top hit) -- the same path demo-phase3/demo-matrix use.
async function palette(page: import("@playwright/test").Page, query: string) {
  await page.keyboard.press("Escape");
  await page.waitForTimeout(80);
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(220);
  await page.keyboard.type(query);
  await page.waitForTimeout(200);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
}

function isFatal(text: string): boolean {
  const m = text.toLowerCase();
  return (
    m.includes("panic") ||
    m.includes("unreachable executed") ||
    m.includes("failed to start the reticle web app")
  );
}

// Boots the app, brings it to the foreground, and waits for the seam loop to be live.
// Returns the collected console-error sink so each test can assert no fatals.
async function boot(page: import("@playwright/test").Page, url: string): Promise<string[]> {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push("PAGEERR " + String(e)));
  await page.goto(url);
  await page.bringToFront();
  await expect(page.locator("#overlay")).toBeHidden();
  // The per-frame stats loop is live once active_tool is published (every editor frame).
  await expect
    .poll(async () => (await stats(page)).active_tool ?? null, { timeout: 30_000 })
    .not.toBeNull();
  return errors;
}

// Selects the Rectangle tool via the palette and drags a rect on the canvas centre.
async function drawRect(page: import("@playwright/test").Page, cx: number, cy: number) {
  await palette(page, "Rectangle tool");
  await expect
    .poll(async () => (await stats(page)).active_tool ?? "", { timeout: 10_000 })
    .toBe("Rect");
  await page.mouse.move(cx - 80, cy - 60);
  await page.mouse.down();
  await page.mouse.move(cx + 80, cy + 60, { steps: 10 });
  await page.mouse.up();
  await page.waitForTimeout(250);
}

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

test("PHASE-4 draw tools: tt03 (beyond sky130) and a blank document", async ({
  page,
}, testInfo) => {
  test.setTimeout(180_000);
  const proj = testInfo.project.name;

  // --- tt03: draw beyond the sky130 example the matrix already covers. ---
  const errorsTt = await boot(page, "/?e2e-example=tt03");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);
  const canvas = page.locator("#reticle-canvas");
  const box = (await canvas.boundingBox())!;
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.click(box.x + 20, box.y + 20);
  await page.keyboard.press("Escape");
  const ttBefore = await stats(page);
  await drawRect(page, cx, cy);
  const ttAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p4-draw-tt03-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(
    `PHASE4LOG draw.tt03 ${proj}: shapes ${ttBefore.applied_scene_shapes}->${ttAfter.applied_scene_shapes} undo ${ttBefore.undo_depth}->${ttAfter.undo_depth}`,
  );
  // A canvas-drag draw is a gesture, not a dispatched command, so it grows the scene
  // and the undo stack rather than setting last_command_mutated (which tracks palette
  // dispatches). Both frame seams prove the shape actually landed as an undoable edit.
  expect(
    ttAfter.applied_scene_shapes as number,
    "a rectangle drawn on tt03 lands (applied_scene_shapes grows)",
  ).toBeGreaterThan(ttBefore.applied_scene_shapes as number);
  expect(
    ttAfter.undo_depth as number,
    "the draw is an undoable edit (undo_depth grows)",
  ).toBeGreaterThan(ttBefore.undo_depth as number);

  // --- blank document: draw from ZERO geometry (?e2e-example=blank). ---
  const errorsBlank = await boot(page, "/?e2e-example=blank");
  const blankStart = await stats(page);
  expect(
    blankStart.applied_scene_shapes as number,
    "the blank document starts with no geometry",
  ).toBe(0);
  const box2 = (await canvas.boundingBox())!;
  const bx = box2.x + box2.width / 2;
  const by = box2.y + box2.height / 2;
  await page.mouse.click(box2.x + 20, box2.y + 20);
  await page.keyboard.press("Escape");
  await drawRect(page, bx, by);
  const blankAfter = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p4-draw-blank-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE4LOG draw.blank ${proj}: shapes 0->${blankAfter.applied_scene_shapes}`);
  expect(
    blankAfter.applied_scene_shapes as number,
    "a rectangle drawn on the blank document lands (0 -> >0)",
  ).toBeGreaterThan(0);

  const fatals = [...errorsTt, ...errorsBlank].filter(isFatal);
  expect(fatals, `fatal errors:\n${[...errorsTt, ...errorsBlank].join("\n")}`).toHaveLength(0);
});

test("PHASE-4 plugin manager: browse + honest browser disclaimer", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  // Browse the F5 plugin index: reveals the Inspector Plugins section. On the web the
  // section previews the index but shows the honest "plugins run in the desktop app"
  // disclaimer (ADR 0120); it never claims to run one.
  await palette(page, "Browse plugins");
  const pl = await stats(page);
  // The Plugins section sits at the bottom of the Automate inspector group, below the
  // Agent panel, so scroll the inspector to bring the browse list and the honest
  // "plugins run in the desktop app" disclaimer into frame for the screenshot evidence.
  await page.mouse.move(1150, 160);
  await page.mouse.wheel(0, 1600);
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${SCRATCH}/p4-plugin-browse-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE4LOG plugin.browse ${proj}: last=${pl.last_command_id}`);
  expect(pl.last_command_id, "Browse plugins dispatches plugin.browse").toBe("plugin.browse");

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

test("PHASE-4 image underlay: browser decode + load a fixture photo", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);
  expect((await stats(page)).underlay_loaded ?? false, "no underlay before load").toBe(false);

  // underlay.load opens the browser file picker (rfd AsyncFileDialog -> a real
  // <input type=file>); Playwright answers it via the filechooser event with a committed
  // fixture photo. The browser decodes it (createImageBitmap, ADR 0118) and adopts it.
  const chooserPromise = page.waitForEvent("filechooser", { timeout: 15_000 });
  await palette(page, "Load die-photo underlay");
  const chooser = await chooserPromise;
  await chooser.setFiles(UNDERLAY_FIXTURE);

  await expect
    .poll(async () => (await stats(page)).underlay_loaded ?? false, { timeout: 20_000 })
    .toBe(true);
  await page.screenshot({ path: `${SCRATCH}/p4-underlay-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE4LOG underlay ${proj}: underlay_loaded=${(await stats(page)).underlay_loaded}`);

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

test("PHASE-4 embed mode: renders in embed chrome and toggles on live", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;

  // Part 1: boot straight into embed mode and prove the design still paints (embed
  // strips the chrome, not the canvas).
  const errors = await boot(page, "/?e2e-example=sky130&embed=1");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);
  const embedded = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p4-embed-on-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE4LOG embed.render ${proj}: embed=${embedded.embed} shapes=${embedded.applied_scene_shapes}`);
  expect(embedded.embed, "?embed=1 boots in embed mode").toBe(true);
  expect(embedded.render_nonblank, "embed mode still paints real geometry").toBe(true);

  // Part 2: from a NORMAL boot (embed off, so the command palette is reachable), the
  // embed.toggle command flips the mode ON. Toggling is driven from normal mode because
  // embed deliberately hides the chrome the palette lives in.
  const errors2 = await boot(page, "/?e2e-example=sky130");
  expect((await stats(page)).embed ?? true, "a normal boot is not embedded").toBe(false);
  await palette(page, "Toggle embed mode");
  await expect
    .poll(async () => (await stats(page)).embed ?? false, { timeout: 10_000 })
    .toBe(true);
  const toggled = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p4-embed-toggled-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE4LOG embed.toggle ${proj}: embed=${toggled.embed} last=${toggled.last_command_id}`);
  expect(toggled.embed, "Toggle embed mode enters embed mode").toBe(true);
  expect(toggled.last_command_id, "Toggle embed mode dispatches embed.toggle").toBe("embed.toggle");

  const fatals = [...errors, ...errors2].filter(isFatal);
  expect(fatals, `fatal errors:\n${[...errors, ...errors2].join("\n")}`).toHaveLength(0);
});

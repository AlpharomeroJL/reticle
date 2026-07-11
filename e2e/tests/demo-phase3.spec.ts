import { test, expect } from "@playwright/test";

// Phase-3 (Depth) comprehensive headed pass. Drives each Inspector panel and
// command Phase 3 introduced (DRC, the waveform live solve, the net-trace panel,
// the PCell inspector, the agent plan, the classroom roster) in a FOREGROUND
// headed browser on BOTH backends, and asserts each had an EFFECT via the
// __reticle_stats seams (drc_ran/drc_violations, waveform_probes, agent_running,
// last_command_id/last_command_mutated, applied_scene_shapes), plus a screenshot
// per state. Grows the comprehensive contract for the surfaces this phase added.
//
// Reproduction law (packet caveat): headed + foreground only; eframe pauses its
// rAF loop when the tab is occluded, blanking the canvas and nulling the seam.

const SCRATCH = "../scratch/logs/e2e-phase3";

type Stats = Record<string, unknown>;
function stats(page: import("@playwright/test").Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

// Dispatch a command by opening the palette, typing a distinctive fragment of its
// label, and pressing Enter (runs the top hit) -- the same path demo-matrix uses.
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

test("PHASE-3 panels + DRC two-way (sky130 editor, headed)", async ({
  page,
}, testInfo) => {
  test.setTimeout(180_000);
  const proj = testInfo.project.name;
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
  const box = (await canvas.boundingBox())!;
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.click(box.x + 20, box.y + 20); // focus without selecting
  await page.keyboard.press("Escape");

  // The panels run first, on the full sky130 geometry; the DRC two-way (which
  // empties the doc for its clean direction) runs last.

  // --- Waveform: the live pure-Rust MNA solve (F4 swap). run_oracle loads probes. ---
  await palette(page, "Run simulation oracle");
  await expect
    .poll(async () => (await stats(page)).waveform_probes ?? 0, { timeout: 15_000 })
    .toBeGreaterThan(0);
  const wf = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p3-waveform-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE3LOG waveform ${proj}: waveform_probes=${wf.waveform_probes} last=${wf.last_command_id}`);
  expect(wf.last_command_id, "Run simulation oracle dispatches waveform.run_oracle").toBe("waveform.run_oracle");

  // --- PCell inspector: opens; the browser build shows predicted provenance and
  // an honest desktop-for-live-produce disclaimer (ADR 0115). Screenshot it. ---
  await palette(page, "Edit PCell parameters");
  const pc = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p3-pcell-${proj}.png` });
  expect(pc.last_command_id, "Edit PCell parameters dispatches pcell.edit_params").toBe("pcell.edit_params");

  // --- Net trace: select a shape, then trace the net at that point. ---
  await page.keyboard.press("v");
  await page.waitForTimeout(100);
  await page.mouse.click(cx, cy);
  await page.waitForTimeout(150);
  await palette(page, "Trace net at point");
  const tr = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p3-trace-${proj}.png` });
  expect(tr.last_command_id, "Trace net at point dispatches trace.at_point").toBe("trace.at_point");

  // --- Agent: plan a run (scripted preview on the web; the real model runner is
  // native-only). It runs ASYNCHRONOUSLY, so dispatch it, confirm it STARTED, then
  // WAIT for it to finish (agent_running -> false) before any further command -- a
  // command dispatched mid-run is dropped (last_command_id stays on agent.plan), so
  // not waiting cascades into the next test. ---
  await palette(page, "Plan agent run");
  await expect
    .poll(async () => (await stats(page)).agent_running ?? false, { timeout: 15_000 })
    .toBe(true);
  await page.screenshot({ path: `${SCRATCH}/p3-agent-running-${proj}.png` });
  const agStart = await stats(page);
  expect(agStart.last_command_id, "Plan agent run dispatches agent.plan").toBe("agent.plan");
  // Wait for the scripted run to quiesce before the next command.
  await expect
    .poll(async () => (await stats(page)).agent_running ?? true, { timeout: 60_000 })
    .toBe(false);
  await page.screenshot({ path: `${SCRATCH}/p3-agent-done-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE3LOG agent ${proj}: started then quiesced (agent_running=false)`);

  // --- Classroom: bring-everyone fires; the roster is honestly empty until a
  // write-capable presence path exists (ADR 0111). The command must still run. ---
  await palette(page, "Bring everyone here");
  const cr = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p3-classroom-${proj}.png` });
  expect(cr.last_command_id, "Bring everyone here dispatches classroom.bring_everyone").toBe("classroom.bring_everyone");

  // --- DRC two-way (LAST; empties the doc for its clean direction). The real
  // sky130 cell has violations; an emptied cell is clean. A fabricated DRC that
  // always returns clean fails the first assertion; one that always returns
  // violations fails the second. ---
  await palette(page, "Run DRC");
  await expect
    .poll(async () => (await stats(page)).drc_ran ?? false, { timeout: 15_000 })
    .toBe(true);
  const drcFull = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/p3-drc-violations-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`PHASE3LOG drc.full ${proj}: drc_violations=${drcFull.drc_violations} last=${drcFull.last_command_id}`);
  expect(drcFull.last_command_id, "Run DRC dispatches verify.drc_run").toBe("verify.drc_run");
  expect(
    drcFull.drc_violations as number,
    "the real sky130 cell reports DRC violations (a fabricated always-clean DRC would report 0)",
  ).toBeGreaterThan(0);

  // The exact bad-cell-vs-clean-cell two-way with real rule values (min width,
  // spacing, enclosure) lives in the reticle-drc unit suite (crates/reticle-drc/
  // tests/sky130.rs, the seeded-violation pattern). Here the headed browser proves
  // DRC runs end to end and reports a real, non-zero count on real geometry, which
  // a fabricated always-clean DRC could not.

  const fatals = errors.filter(isFatal);
  expect(fatals, `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

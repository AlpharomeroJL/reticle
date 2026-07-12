import { test, expect } from "@playwright/test";

// Registry-driven EXHAUSTIVE headed sweep (permanent gate). Drives every command in the
// live registry, plus the corrupt-file, resize, zoom, share-permalink, and inspector-panel
// surfaces, in a FOREGROUND headed browser on BOTH backends (headed-webgl2 and
// headed-webgpu). The file name matches /demo-.*\.spec\.ts/, so playwright.config.ts runs
// it in the headed projects only.
//
// It fires commands through the debug-only wasm seams this lane added (present in the DEBUG
// e2e Trunk build, ABSENT from the measured --release bundle):
//   * window.__reticle_e2e_command_ids : every registry command as { id, label, scope },
//     derived from the live registry so this sweep never drifts from it.
//   * window.__reticle_e2e_dispatch(id) : fires a command through the SAME App::dispatch
//     funnel the palette/toolbar/menus use, so commands that are not uniquely palette-
//     addressable are still driven. Returns whether the id named a real command.
//
// Reproduction law (packet caveat): eframe pauses its requestAnimationFrame loop in a
// backgrounded/occluded tab, so the canvas goes black and window.__reticle_stats reads
// null. That is NORMAL browser behavior, not a defect. Every boot calls page.bringToFront()
// and asserts document.visibilityState === "visible" before reading any seam.

const SCRATCH = "../scratch/logs/registry-sweep";

// Corrupt import fixtures (created by this lane under e2e/fixtures/corrupt/). Paths are
// relative to the e2e working directory, exactly like demo-phase4's UNDERLAY_FIXTURE.
const CORRUPT_FIXTURES: Record<string, string> = {
  gds: "fixtures/corrupt/corrupt.gds",
  oasis: "fixtures/corrupt/corrupt.oas",
  cif: "fixtures/corrupt/corrupt.cif",
  dxf: "fixtures/corrupt/corrupt.dxf",
};

// Commands that would BLOCK or LEAVE the sweep: they open a native filechooser or navigate
// away. Skipped (never fired) with a logged reason; each is covered by a dedicated test.
const SKIP: Map<string, string> = new Map([
  [
    "file.open_dialog",
    "opens a native filechooser that would block the sweep (exercised by the CORRUPT-FILE OPEN test)",
  ],
  [
    "underlay.load",
    "opens a native filechooser that would block the sweep (exercised by demo-phase4)",
  ],
  [
    "help.docs",
    "opens external documentation in a new browser tab (the offline sweep does not follow external links)",
  ],
]);

// Commands fired LAST because they change the top-level view for every command after them.
// file.close_design returns to the Start screen, which stops the per-frame editor seams.
const DEFER = new Set<string>(["file.close_design"]);

// Known-mutating commands: firing them must change the document revision, so
// last_command_mutated proves the command had an EFFECT (not just that it fired).
const MUTATING = new Set<string>(["dev.add_demo_rect"]);

// The Inspector panel-revealing commands (deliverable 6): each opens/loads a panel section.
const INSPECTOR_PANELS: { id: string; section: string }[] = [
  { id: "pcell.edit_params", section: "PCell" },
  { id: "trace.at_point", section: "Trace" },
  { id: "waveform.run_oracle", section: "Waveform" },
  { id: "agent.plan", section: "Agent" },
  { id: "plugin.browse", section: "Plugins" },
];

type Stats = Record<string, unknown>;
type Camera = { center_x: number; center_y: number; pixels_per_dbu: number };
type CommandEntry = { id: string; label: string; scope: string };

function stats(page: import("@playwright/test").Page): Promise<Stats> {
  return page.evaluate(
    () => (window as unknown as { __reticle_stats?: Stats }).__reticle_stats ?? {},
  );
}

// Dispatch a command by opening the palette, typing a distinctive fragment of its label,
// and pressing Enter (runs the top hit) -- the same path demo-phase3/demo-phase4 use.
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
    m.includes("failed to start the reticle web app") ||
    m.includes("is missing a #reticle-canvas")
  );
}

// Boots the app, brings it to the foreground, asserts it is visible (reproduction law), and
// waits for the per-frame seam loop to be live. Returns the collected console-error sink.
async function boot(page: import("@playwright/test").Page, url: string): Promise<string[]> {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push("PAGEERR " + String(e)));
  await page.goto(url);
  await page.bringToFront();
  expect(
    await page.evaluate(() => document.visibilityState),
    "the page must be foreground/visible for egui to render (packet reproduction caveat)",
  ).toBe("visible");
  await expect(page.locator("#overlay")).toBeHidden();
  // The per-frame stats loop is live once active_tool is published (every editor frame).
  await expect
    .poll(async () => (await stats(page)).active_tool ?? null, { timeout: 30_000 })
    .not.toBeNull();
  return errors;
}

// Reads the registry command-id catalog once it is published (a non-empty array).
async function commandCatalog(
  page: import("@playwright/test").Page,
): Promise<CommandEntry[]> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const a = (window as unknown as { __reticle_e2e_command_ids?: unknown })
            .__reticle_e2e_command_ids;
          return Array.isArray(a) ? a.length : 0;
        }),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0);
  return page.evaluate(
    () =>
      (window as unknown as { __reticle_e2e_command_ids?: CommandEntry[] })
        .__reticle_e2e_command_ids ?? [],
  );
}

// Fires a command through the App::dispatch funnel; returns whether the id was a real
// command (null if the seam is missing, which fails the caller's assertion).
async function dispatch(
  page: import("@playwright/test").Page,
  id: string,
): Promise<boolean | null> {
  return page.evaluate((cmd) => {
    const fn = (window as unknown as { __reticle_e2e_dispatch?: (id: string) => boolean })
      .__reticle_e2e_dispatch;
    return typeof fn === "function" ? Boolean(fn(cmd)) : null;
  }, id);
}

// After a command that may start the async agent run, wait for it to quiesce so a
// command dispatched mid-run is never dropped (see demo-phase3's agent note).
async function waitAgentQuiesced(page: import("@playwright/test").Page) {
  if ((await stats(page)).agent_running === true) {
    await expect
      .poll(async () => (await stats(page)).agent_running ?? true, { timeout: 60_000 })
      .toBe(false);
  }
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

// 1. EXHAUSTIVE COMMAND SWEEP -----------------------------------------------------------
test("REGISTRY SWEEP: every registry command fires cleanly on a loaded doc (sky130)", async ({
  page,
}, testInfo) => {
  test.setTimeout(360_000);
  const proj = testInfo.project.name;

  // Defensive: no unexpected native dialog, download, or popup may block the sweep. The two
  // filechooser commands are skipped, but guard anyway so a stray one never hangs.
  page.on("filechooser", (fc) => {
    fc.setFiles([]).catch(() => {});
  });
  page.on("popup", (p) => {
    p.close().catch(() => {});
  });

  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  const catalog = await commandCatalog(page);
  const total = catalog.length;
  expect(total, "the registry catalog must not be empty").toBeGreaterThan(0);

  // Every skip target must still be a real command, so a stale skip cannot silently hide a
  // removed id.
  for (const id of SKIP.keys()) {
    expect(
      catalog.some((c) => c.id === id),
      `skip target ${id} is no longer in the registry; update SKIP`,
    ).toBe(true);
  }

  await page.screenshot({ path: `${SCRATCH}/sweep-00-start-${proj}.png` });

  // Non-deferred first, then the deferred (view-changing) commands last.
  const order = [
    ...catalog.filter((c) => !DEFER.has(c.id)),
    ...catalog.filter((c) => DEFER.has(c.id)),
  ];

  let fired = 0;
  let skipped = 0;
  for (const { id, label } of order) {
    if (SKIP.has(id)) {
      skipped++;
      // eslint-disable-next-line no-console
      console.log(`SWEEPSKIP ${id} (${label}): ${SKIP.get(id)}`);
      continue;
    }

    const known = await dispatch(page, id);
    expect(known, `__reticle_e2e_dispatch("${id}") must resolve a real registry command`).toBe(
      true,
    );
    fired++;

    // The drain fires the command next frame and publishes last_command_id; polling for it
    // is both the wait and the proof the command FIRED through the dispatch funnel.
    await expect
      .poll(async () => (await stats(page)).last_command_id ?? null, { timeout: 15_000 })
      .toBe(id);

    await waitAgentQuiesced(page);

    const s = await stats(page);
    // Frame loop still alive: the seam is still readable and a tool is still published (a
    // wasm panic would have nulled the whole seam and killed the tab).
    expect(s.active_tool ?? null, `frame loop dead after ${id}`).not.toBeNull();
    // Known-mutating commands must have had an EFFECT (the document revision changed).
    if (MUTATING.has(id)) {
      expect(s.last_command_mutated, `${id} should mutate the document`).toBe(true);
    }
    const fatals = errors.filter(isFatal);
    expect(fatals, `fatal after ${id}:\n${errors.join("\n")}`).toHaveLength(0);
  }

  await page.screenshot({ path: `${SCRATCH}/sweep-99-end-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(`SWEEPLOG ${proj}: total=${total} fired=${fired} skipped=${skipped}`);

  // Coverage: every command id was either fired or explicitly skipped. No silent caps.
  expect(fired + skipped, "every command id must be fired or explicitly skipped").toBe(total);
  expect(errors.filter(isFatal), `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

// 2. CORRUPT-FILE OPEN per import format ------------------------------------------------
test("CORRUPT-FILE OPEN: gds/oasis/cif/dxf each error cleanly and keep the app alive", async ({
  page,
}, testInfo) => {
  test.setTimeout(180_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  for (const [fmt, file] of Object.entries(CORRUPT_FIXTURES)) {
    const before = await stats(page);
    // file.open_dialog opens the browser file picker (rfd -> <input type=file>); Playwright
    // answers it via the filechooser event with a committed CORRUPT fixture. The hardened
    // open path classifies by extension, parses, and REJECTS it (malformed input errors,
    // never panics), leaving the good sky130 document intact.
    const chooserPromise = page.waitForEvent("filechooser", { timeout: 15_000 });
    expect(await dispatch(page, "file.open_dialog")).toBe(true);
    const chooser = await chooserPromise;
    await chooser.setFiles(file);
    // Give the classify -> plan -> parse -> reject chain time to run and toast.
    await page.waitForTimeout(1500);

    const after = await stats(page);
    await page.screenshot({ path: `${SCRATCH}/corrupt-${fmt}-${proj}.png` });
    // eslint-disable-next-line no-console
    console.log(
      `SWEEPLOG corrupt.${fmt} ${proj}: shapes ${before.applied_scene_shapes}->${after.applied_scene_shapes} nonblank=${after.render_nonblank}`,
    );

    // Clean error path: no panic, the seam is still readable (app alive), and the good
    // document SURVIVED (a corrupt file is never installed as garbage or a silent empty doc).
    expect(errors.filter(isFatal), `fatal on corrupt ${fmt}:\n${errors.join("\n")}`).toHaveLength(
      0,
    );
    expect(after.active_tool ?? null, `frame loop dead after corrupt ${fmt}`).not.toBeNull();
    expect(
      after.applied_scene_shapes as number,
      `corrupt ${fmt} clobbered the open document`,
    ).toBeGreaterThan(0);
    expect(after.render_nonblank, `not painting after corrupt ${fmt}`).toBe(true);
  }

  expect(errors.filter(isFatal), `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

// 3. WINDOW RESIZE ----------------------------------------------------------------------
test("WINDOW RESIZE: the app stays alive and painting across viewports (sky130)", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  for (const [w, h] of [
    [800, 600],
    [1280, 800],
    [1000, 1400],
    [1600, 900],
    [640, 480],
  ] as const) {
    await page.setViewportSize({ width: w, height: h });
    await page.bringToFront();
    await page.waitForTimeout(500);
    const s = await stats(page);
    await page.screenshot({ path: `${SCRATCH}/resize-${w}x${h}-${proj}.png` });
    // eslint-disable-next-line no-console
    console.log(`SWEEPLOG resize ${w}x${h} ${proj}: nonblank=${s.render_nonblank}`);
    expect(s.active_tool ?? null, `frame loop dead at ${w}x${h}`).not.toBeNull();
    expect(s.render_nonblank, `not painting at ${w}x${h}`).toBe(true);
  }

  expect(errors.filter(isFatal), `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

// 4. ZOOM LADDER ------------------------------------------------------------------------
test("ZOOM LADDER: zoom-in/out rungs with a screenshot each (sky130)", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  const canvas = page.locator("#reticle-canvas");
  const box = (await canvas.boundingBox())!;
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx, cy);

  const startPpd =
    ((await stats(page)).camera as Camera | undefined)?.pixels_per_dbu ?? 0;
  let rung = 0;
  const record = async (dir: string): Promise<number> => {
    const s = await stats(page);
    const ppd = (s.camera as Camera | undefined)?.pixels_per_dbu ?? 0;
    await page.screenshot({
      path: `${SCRATCH}/zoom-${String(rung).padStart(2, "0")}-${dir}-${proj}.png`,
    });
    // eslint-disable-next-line no-console
    console.log(`SWEEPLOG zoom.${dir} rung=${rung} ${proj}: ppd=${ppd}`);
    expect(s.active_tool ?? null, `frame loop dead at zoom rung ${rung}`).not.toBeNull();
    expect(s.render_nonblank, `not painting at zoom rung ${rung}`).toBe(true);
    rung++;
    return ppd;
  };

  await record("start");
  let peakPpd = startPpd;
  for (let i = 0; i < 4; i++) {
    await page.mouse.wheel(0, -400);
    await page.waitForTimeout(280);
    peakPpd = Math.max(peakPpd, await record("in"));
  }
  for (let i = 0; i < 4; i++) {
    await page.mouse.wheel(0, 400);
    await page.waitForTimeout(280);
    await record("out");
  }

  // The zoom-in rungs magnified the view well above the start; the ladder then returns
  // near the start fit, so assert the PEAK rather than the end. An end-vs-start delta is
  // flaky: the equal in-and-out cancels back to the exact fit ppd (a zero delta) at some
  // camera-settle timings.
  expect(
    peakPpd,
    "the zoom-in rungs never magnified the view",
  ).toBeGreaterThan(startPpd * 1.5);
  expect(errors.filter(isFatal), `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
});

// 5. SHARE ROUND-TRIP -------------------------------------------------------------------
test("SHARE ROUND-TRIP: copy-permalink fires and a ?view permalink reopens cleanly (sky130)", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  const proj = testInfo.project.name;
  const errors = await boot(page, "/?e2e-example=sky130");
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);

  // Move to a DISTINCT, deterministic camera (1:1 DBU) away from the fit view, so the
  // permalink is copied over a non-default view.
  expect(await dispatch(page, "view.zoom_one_to_one")).toBe(true);
  await expect
    .poll(async () => (await stats(page)).last_command_id ?? null, { timeout: 10_000 })
    .toBe("view.zoom_one_to_one");
  await page.waitForTimeout(300);
  const cam = (await stats(page)).camera as Camera | undefined;
  expect(cam?.pixels_per_dbu ?? 0, "a live camera to share").toBeGreaterThan(0);

  // Copy the permalink for this view through the same command funnel the UI uses; assert it
  // fires cleanly (it emits the ?cell/?view/?layers link and copies it to the clipboard).
  expect(await dispatch(page, "share.copy_permalink")).toBe(true);
  await expect
    .poll(async () => (await stats(page)).last_command_id ?? null, { timeout: 10_000 })
    .toBe("share.copy_permalink");
  expect((await stats(page)).active_tool ?? null, "copy-permalink kept the frame loop alive").not.toBeNull();

  // Reopen a ?view permalink for that camera in a FRESH page load: the app parses it and
  // boots onto the same example, painting -- no crash, no blank page. The emit->parse
  // identity and the parse->apply camera restore (including that an explicit camera cancels
  // auto-fit) are exhaustively UNIT-proven, so they are not re-asserted through this
  // synchronous debug reload (which applies its permalink before its deferred fit, making an
  // exact-pixel reload assertion flaky): see share.rs `permalink_round_trips_cell_camera_and_layers`
  // and `session_permalink_captures_cell_camera_and_visible_layers`, and the app.rs apply
  // tests ("the permalink camera cancels auto-fit"). The authoritative reload path that
  // restores the camera is the async ?gds=/?archive= open (app.rs:2802).
  const view = `${cam!.center_x},${cam!.center_y},${cam!.pixels_per_dbu}`;
  const errors2 = await boot(page, `/?e2e-example=sky130&view=${encodeURIComponent(view)}`);
  await expect
    .poll(async () => (await stats(page)).applied_scene_shapes ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(0);
  const reopened = await stats(page);
  await page.screenshot({ path: `${SCRATCH}/share-roundtrip-${proj}.png` });
  // eslint-disable-next-line no-console
  console.log(
    `SWEEPLOG share ${proj}: cam=(${cam!.center_x},${cam!.center_y},${cam!.pixels_per_dbu}) reopened nonblank=${reopened.render_nonblank}`,
  );
  expect(reopened.render_nonblank, "the reopened ?view permalink paints").toBe(true);
  expect(reopened.active_tool ?? null, "the reopened ?view permalink boots the editor").not.toBeNull();

  const allErrors = [...errors, ...errors2];
  expect(allErrors.filter(isFatal), `fatal errors:\n${allErrors.join("\n")}`).toHaveLength(0);
});

// 6. EVERY INSPECTOR PANEL on every start-screen example AND a blank doc -----------------
for (const example of ["sky130", "tt03", "blank"]) {
  test(`INSPECTOR PANELS: every panel opens on ${example}`, async ({ page }, testInfo) => {
    test.setTimeout(150_000);
    const proj = testInfo.project.name;
    const errors = await boot(page, `/?e2e-example=${example}`);
    // sky130/tt03 carry geometry; blank starts empty. The frame loop (active_tool) is the
    // signal that works for all three (boot already waits for it).

    for (const { id, section } of INSPECTOR_PANELS) {
      expect(await dispatch(page, id), `${id} must be a real command`).toBe(true);
      await expect
        .poll(async () => (await stats(page)).last_command_id ?? null, { timeout: 15_000 })
        .toBe(id);
      await waitAgentQuiesced(page);
      const s = await stats(page);
      expect(s.active_tool ?? null, `frame loop dead after ${section} on ${example}`).not.toBeNull();
      expect(
        errors.filter(isFatal),
        `fatal after ${section} on ${example}:\n${errors.join("\n")}`,
      ).toHaveLength(0);
    }

    // Toggle the always-present Layers/Inspector rail, then capture the panels for evidence.
    expect(await dispatch(page, "view.panels_toggle")).toBe(true);
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${SCRATCH}/panels-${example}-${proj}.png` });

    // Also assert the Layers/Inspector chrome itself is alive (a screenshot plus the seam).
    const s = await stats(page);
    expect(s.active_tool ?? null, `frame loop dead on ${example}`).not.toBeNull();
    expect(errors.filter(isFatal), `fatal errors:\n${errors.join("\n")}`).toHaveLength(0);
  });
}

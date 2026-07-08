// UI baseline gallery capture (v8.1 interface packet, Wave 0). NOT part of the test run.
//
// Shoots the DEPLOYED bundle's URL-reachable UI states at fixed viewport sizes so the
// redesign has an honest "before" record, and (rerun with --label after at Wave 5) an
// "after" record from the same script. States that need in-app interaction (panels
// expanded, overlays toggled, dialogs open) are NOT captured here; those come from the
// native demo-script harness against the tagged before/after builds in the Wave 5
// GPU-serialized capture queue.
//
// Usage:
//   node baseline-gallery.mjs                                  # before, live site
//   node baseline-gallery.mjs --base <url> --out <dir> --label after
//
// Output: <out>/<state>--<size>.png plus <out>/manifest.md recording the base URL, the
// served bundle hash (parsed from the live index.html), the capture date, and per-state
// notes. Rendering uses Chromium's SwiftShader software GL (the same flags as the e2e
// webgl2 project) so captures do not depend on the host GPU driver.
import { chromium, devices } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { join, dirname } from "node:path";
import { mkdirSync, writeFileSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const BASE = arg("base", "https://alpharomerojl.github.io/reticle/").replace(/\/?$/, "/");
const LABEL = arg("label", "before");
const OUT = arg("out", join(here, "..", "docs", "design", "baseline"));

// The flagship live archive (content-hash key, see scratch/flagship/anchor-120m.hash).
const ARCHIVE_URL = "https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla";

// Software GL, same as the e2e webgl2 project, so SwiftShader renders deterministically.
const GL_ARGS = ["--use-angle=swiftshader", "--use-gl=angle"];

// URL-reachable states. `waitStat` names a window.__reticle_stats counter that must be
// positive before the shot (used for streaming, where "booted" is not "painted").
const STATES = [
  {
    name: "home-default",
    qs: "",
    settleMs: 5000,
    desc: "landing view exactly as a first visit gets it (replay theater is the public default)",
  },
  {
    name: "view-editor",
    qs: "?view=editor",
    settleMs: 5000,
    desc: "the editor entry (?view=editor) as it lands, start surface included if shown",
  },
  {
    name: "archive-stream",
    qs: `?archive=${encodeURIComponent(ARCHIVE_URL)}`,
    waitStat: "archive_records_painted",
    settleMs: 6000,
    desc: "streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active",
  },
  {
    name: "viewer-empty-room",
    qs: "?view=viewer&room=v81-baseline-gallery",
    settleMs: 9000,
    desc: "share-link viewer chrome joining a room with no publisher (no live content by design)",
  },
];

// Desktop sizes per the packet (three fixed sizes), plus touch devices for key states.
const DESKTOP_SIZES = [
  [1280, 800],
  [1600, 1000],
  [900, 600],
];
const DEVICE_STATES = ["home-default", "view-editor", "archive-stream"];
const DEVICES = [
  { tag: "phone", descriptor: devices["Pixel 7"] },
  { tag: "tablet", descriptor: devices["iPad Mini"] },
];

/** Loads a state and waits for the genuine boot signal, then any state-specific wait. */
async function settle(page, state) {
  await page.goto(BASE + state.qs, { waitUntil: "domcontentloaded", timeout: 60_000 });
  // web/src/main.rs hides #overlay only AFTER the wgpu renderer initialized; this is
  // the same "it rendered" signal the boot e2e gates on.
  await page.locator("#overlay").waitFor({ state: "hidden", timeout: 60_000 });
  if (state.waitStat) {
    try {
      await page.waitForFunction(
        (key) => {
          const s = window.__reticle_stats;
          return s && typeof s[key] === "number" && s[key] > 0;
        },
        state.waitStat,
        { timeout: 45_000 },
      );
    } catch {
      console.warn(`  ${state.name}: ${state.waitStat} never went positive; capturing anyway (noted)`);
      state.statTimedOut = true;
    }
  }
  await page.waitForTimeout(state.settleMs);
}

async function main() {
  mkdirSync(OUT, { recursive: true });

  // Record the served bundle hash for provenance.
  let bundleHash = "unknown";
  try {
    const html = await (await fetch(BASE)).text();
    const m = html.match(/web-([0-9a-f]{8,32})/);
    if (m) bundleHash = `web-${m[1]}`;
  } catch (e) {
    console.warn(`could not fetch ${BASE} for the bundle hash: ${e}`);
  }

  const browser = await chromium.launch({ args: GL_ARGS });
  const rows = [];

  for (const state of STATES) {
    for (const [w, h] of DESKTOP_SIZES) {
      const context = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
      const page = await context.newPage();
      const file = `${state.name}--${w}x${h}.png`;
      console.log(`capturing ${file}`);
      await settle(page, state);
      await page.screenshot({ path: join(OUT, file) });
      rows.push({ file, state, size: `${w}x${h}` });
      await context.close();
    }
  }

  for (const dev of DEVICES) {
    for (const state of STATES.filter((s) => DEVICE_STATES.includes(s.name))) {
      const context = await browser.newContext({ ...dev.descriptor });
      const page = await context.newPage();
      const vp = dev.descriptor.viewport;
      const file = `${state.name}--${dev.tag}-${vp.width}x${vp.height}.png`;
      console.log(`capturing ${file}`);
      await settle(page, state);
      await page.screenshot({ path: join(OUT, file) });
      rows.push({ file, state, size: `${dev.tag} ${vp.width}x${vp.height}` });
      await context.close();
    }
  }

  await browser.close();

  const date = new Date().toISOString().slice(0, 10);
  const lines = [
    `# UI ${LABEL} gallery (URL-reachable states)`,
    "",
    `Captured ${date} from ${BASE} (served bundle: ${bundleHash}) by e2e/baseline-gallery.mjs`,
    "using Chromium + SwiftShader software GL (the e2e webgl2 flags), device scale 1.",
    "",
    "Interior states that need in-app interaction (panels expanded, DRC/diff overlays,",
    "comments, agent panel, 3D stack, cross-section) are captured by the native",
    "demo-script harness in the Wave 5 capture queue, not by this script.",
    "",
    "| file | state | size | notes |",
    "|---|---|---|---|",
    ...rows.map((r) => {
      const note = r.state.statTimedOut
        ? `${r.state.desc} (stat wait timed out on this host; shot after boot + settle)`
        : r.state.desc;
      return `| ${r.file} | ${r.state.name} | ${r.size} | ${note} |`;
    }),
    "",
  ];
  writeFileSync(join(OUT, "manifest.md"), lines.join("\n"));
  console.log(`wrote ${rows.length} captures + manifest.md to ${OUT}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});

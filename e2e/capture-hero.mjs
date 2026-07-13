// Real streaming-hero GIF capture (README hero). NOT part of the test run.
//
// Records docs/media/hero-gallery-stream.gif for the featured 3.01 GiB streamed die, in
// three phases assembled into one GIF with gifski (on PATH):
//   1. Streaming HUD -- a brief beat on the die overview with the byte-count HUD in the
//                       top-left corner (bytes fetched vs the 3.0 GiB archive, tiles
//                       resident, records painted) climbing as tiles stream in. Capture
//                       does not start until the stream has SETTLED (tiles resident
//                       stable), so the HUD reads real numbers on a rendered overview.
//   2. 3D orbit      -- the MONEY SHOT. The featured die is a sparse lattice whose flat
//                       view is correct LOD points (not filled polygons), so the colored
//                       geometry is shown as the 3D layer stack (colored by Z-layer):
//                       open view.panel_3d and orbit it with a smooth drag. The byte HUD
//                       stays visible above it. Gated: the 3D panel region must render
//                       colored geometry (media-gate.mjs) or the capture aborts.
//   3. Share         -- share.copy_permalink fires, so the "Copied a permalink to this
//                       view" toast appears.
//
// If the 3D stack never renders colored geometry, the capture THROWS (nonzero exit) and no
// GIF is written: reject-and-recapture, per the media gate. No blank/flat media can ship.
//
// REMOTE INPUT: the live 3.01 GiB archive
//   https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla
// (the featured die the Start screen links, crates/reticle-app/src/startscreen.rs), whose
// CORS allows only origin https://alpharomerojl.github.io. So capture MUST run against the
// DEPLOYED site: set HERO_URL to the live site + ?archive=. A local 127.0.0.1 bundle cannot
// fetch the remote die (CORS); it can only stream a local archive (ARCHIVE_URL). Env:
//   HERO_URL     -- full page URL to open (use the live site; overrides everything below)
//   ARCHIVE_URL  -- the `.rtla` archive URL to stream (default: the live die above)
//
// Run via `just capture-hero` (builds the release bundle first). Requires: the Trunk
// bundle built (crates/web/dist), gifski on PATH, a GPU/display (opens a real window),
// and internet (for the default remote archive).
//
// Host caveat (verbatim from capture-share.mjs): on this Windows host Chromium's Dawn
// D3D12 backend fails to load dxil.dll, and once wgpu has picked WebGPU it cannot fall
// back, so the app errors out. Deleting navigator.gpu in every frame forces the WebGL2
// path the e2e already proves. That is why the addInitScript below deletes it.
import { chromium } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join, dirname } from "node:path";
import { mkdirSync, rmSync, readdirSync, existsSync } from "node:fs";
import { histogram, waitForColoredCanvas, MIN_NONBG_BUCKETS } from "./media-gate.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..");
const DIST_PORT = Number(process.env.PORT || 8080);

const ARCHIVE_URL =
  process.env.ARCHIVE_URL || "https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla";

const OUT = join(repo, "docs", "media", "hero-gallery-stream.gif");
const FRAMES_DIR = join(repo, "scratch", "hero-frames");

// Capture cadence and per-phase frame counts (~10 fps).
const FPS = 10;
const STREAM_FRAMES = 10; // ~1.0 s of the streaming HUD on the die overview
const ORBIT_FRAMES = 34; // ~3.4 s of the colored 3D layer-stack orbit (the money shot)
const SHARE_FRAMES = 15; // ~1.5 s of the "link copied" toast

// Headed window size. Kept modest so the assembled GIF stays under the 6 MB budget.
const VIEW_W = 1000;
const VIEW_H = 680;
const GIF_WIDTH = 900;

// The 3D layer-stack opens as a BOTTOM panel (egui::Panel::bottom("view.panel_3d")); orbit
// around the lower-centre of the window where it renders, and gate that region for color.
const ORBIT_CX = VIEW_W / 2;
const ORBIT_CY = Math.round(VIEW_H * 0.68);
const ORBIT_RX = 200;
const PANEL_CLIP = { x: 250, y: 350, width: 460, height: 250 };

/** Waits until `url` answers, or throws after `timeoutMs`. */
async function waitForHttp(url, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status === 404) return;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${url}`);
    await new Promise((r) => setTimeout(r, 300));
  }
}

const children = [];
function spawnChild(cmd, args, env) {
  const child = spawn(cmd, args, { cwd: here, env: { ...process.env, ...env }, stdio: "inherit" });
  children.push(child);
  return child;
}
function cleanup() {
  for (const c of children) {
    try {
      c.kill();
    } catch {
      /* already gone */
    }
  }
}

let frameIndex = 0;
async function snap(page) {
  await page.screenshot({
    path: join(FRAMES_DIR, `f${String(frameIndex++).padStart(4, "0")}.png`),
  });
}
async function captureFrames(page, frames) {
  for (let i = 0; i < frames; i++) {
    await snap(page);
    await page.waitForTimeout(1000 / FPS);
  }
}

/** Polls window.__reticle_stats until `pred(stats)` holds, or throws after `timeoutMs`. */
async function pollStats(page, pred, message, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const stats = await page.evaluate(() => window.__reticle_stats ?? {});
    if (pred(stats)) return stats;
    if (Date.now() > deadline) {
      throw new Error(`${message} (last stats: ${JSON.stringify(stats)})`);
    }
    await page.waitForTimeout(250);
  }
}

/**
 * Waits until `read(stats)` returns the SAME value on consecutive polls `stable` times in
 * a row (the signal has settled), or throws after `timeoutMs`. Holds capture until the
 * stream stops adding tiles, so the frame is not mid-fill.
 */
async function pollStatsStable(page, read, message, { stable = 3, pollMs = 400, timeoutMs = 60_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let prev = null;
  let steady = 0;
  for (;;) {
    const stats = await page.evaluate(() => window.__reticle_stats ?? {});
    const value = read(stats);
    if (prev !== null && value === prev && value > 0) {
      steady += 1;
      if (steady >= stable) return stats;
    } else {
      steady = 0;
    }
    prev = value;
    if (Date.now() > deadline) {
      throw new Error(`${message} did not stabilize (last: ${JSON.stringify(stats)})`);
    }
    await page.waitForTimeout(pollMs);
  }
}

// Dispatch a command via the palette (the same path the passing demo specs use).
async function palette(page, query) {
  await page.keyboard.press("Escape");
  await page.waitForTimeout(120);
  await page.keyboard.press("Control+p");
  await page.waitForTimeout(260);
  await page.keyboard.type(query);
  await page.waitForTimeout(260);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
}

async function main() {
  if (!existsSync(join(repo, "crates", "web", "dist", "index.html"))) {
    throw new Error("crates/web/dist is missing; build the bundle first (just capture-hero builds it).");
  }
  spawnChild("node", ["serve-dist.mjs"], { PORT: String(DIST_PORT) });
  await waitForHttp(`http://127.0.0.1:${DIST_PORT}/`);

  const base = `http://127.0.0.1:${DIST_PORT}`;
  const url = process.env.HERO_URL || `${base}/?archive=${encodeURIComponent(ARCHIVE_URL)}`;

  const browser = await chromium.launch({ headless: false });
  const context = await browser.newContext({ viewport: { width: VIEW_W, height: VIEW_H } });
  await context.addInitScript(() => {
    try {
      delete Navigator.prototype.gpu;
    } catch {
      /* already absent */
    }
  });
  const page = await context.newPage();

  rmSync(FRAMES_DIR, { recursive: true, force: true });
  mkdirSync(FRAMES_DIR, { recursive: true });

  try {
    await page.goto(url);
    await page.bringToFront();
    await page.locator("#overlay").waitFor({ state: "hidden", timeout: 90_000 });
    await page.locator("#reticle-canvas").waitFor({ state: "visible", timeout: 30_000 });
    await page.waitForTimeout(6_000);

    // Dismiss the first-visit chrome: the bottom-left WebGL2 notice and the centre-bottom
    // "Get started" onboarding card are egui-painted, so click their Dismiss buttons by
    // position (the onboarding Dismiss sits at ~0.40x, 0.89y for this window), then Escape.
    for (const [fx, fy] of [
      [0.403, 0.89],
      [0.16, 0.89],
      [0.5, 0.9],
    ]) {
      await page.mouse.click(Math.round(VIEW_W * fx), Math.round(VIEW_H * fy));
      await page.waitForTimeout(180);
    }
    await page.keyboard.press("Escape");
    await page.waitForTimeout(500);

    // --- Phase 0: settle ------------------------------------------------------------
    await pollStats(page, (s) => (s.archive_bytes_fetched ?? 0) > 0, "streaming never fetched bytes");
    await pollStatsStable(
      page,
      (s) => s.archive_tiles_resident ?? s.tiles_resident ?? 0,
      "resident tiles",
      { stable: 3, pollMs: 400, timeoutMs: 60_000 },
    );

    // --- Phase 1: streaming HUD (brief) ---------------------------------------------
    // The die overview with the byte HUD climbing as background tiles stream in.
    await captureFrames(page, STREAM_FRAMES);

    // --- Phase 2: 3D layer-stack orbit (the money shot) -----------------------------
    await page.mouse.click(ORBIT_CX, Math.round(VIEW_H * 0.3));
    await palette(page, "3D stack");
    await page.waitForTimeout(700);
    // Gate: the 3D panel must render colored geometry (not a blank/flat panel).
    const colored = await waitForColoredCanvas(page, {
      clip: PANEL_CLIP,
      minNonBg: 4,
      timeoutMs: 10_000,
      label: "3D layer stack",
    });
    // eslint-disable-next-line no-console
    console.log(
      `capture-hero: 3D stack colored (${colored.nonBackground} non-bg buckets, ` +
        `coverage ${colored.coverage.toFixed(2)})`,
    );

    // Smooth mouse-drag orbit across the panel.
    await page.mouse.move(ORBIT_CX - ORBIT_RX, ORBIT_CY);
    await page.mouse.down();
    for (let i = 0; i < ORBIT_FRAMES; i++) {
      const t = ORBIT_FRAMES > 1 ? i / (ORBIT_FRAMES - 1) : 1;
      const mx = ORBIT_CX - ORBIT_RX + 2 * ORBIT_RX * t;
      const my = ORBIT_CY + Math.sin(t * Math.PI) * (ORBIT_RX * 0.2);
      await page.mouse.move(mx, my, { steps: 3 });
      await snap(page);
      await page.waitForTimeout(1000 / FPS);
    }
    await page.mouse.up();

    // Gate: the orbit ended on colored geometry (the drag did not blank the panel).
    const orbitEnd = await histogram(page, { clip: PANEL_CLIP });
    if (orbitEnd.nonBackground < MIN_NONBG_BUCKETS) {
      throw new Error(
        `3D orbit ended on a blank panel (${orbitEnd.nonBackground} non-bg buckets); ` +
          `reject-and-recapture`,
      );
    }

    // --- Phase 3: copy permalink toast ----------------------------------------------
    await palette(page, "Copy permalink");
    await captureFrames(page, SHARE_FRAMES);
  } finally {
    await context.close();
    await browser.close();
  }

  const frames = readdirSync(FRAMES_DIR)
    .filter((f) => f.endsWith(".png"))
    .sort()
    .map((f) => join(FRAMES_DIR, f));
  if (frames.length === 0) throw new Error("no frames were captured");
  mkdirSync(dirname(OUT), { recursive: true });
  const res = spawnSync(
    "gifski",
    ["--fps", String(FPS), "--quality", "80", "--width", String(GIF_WIDTH), "-o", OUT, ...frames],
    { stdio: "inherit" },
  );
  if (res.status !== 0) throw new Error(`gifski failed with status ${res.status}`);
  console.log(`capture-hero: wrote ${OUT} (${frames.length} frames)`);
  if (!process.env.KEEP_FRAMES) {
    rmSync(FRAMES_DIR, { recursive: true, force: true });
  }
}

main()
  .then(() => {
    cleanup();
    process.exit(0);
  })
  .catch((err) => {
    console.error("capture-hero failed:", err);
    cleanup();
    process.exit(1);
  });

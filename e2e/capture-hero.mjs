// Real streaming-hero GIF capture (README hero). NOT part of the test run.
//
// Records docs/media/hero-gallery-stream.gif: the real streaming flow, in three phases
// assembled into one GIF with gifski (on PATH):
//   1. Streaming    -- a die streams in over the network with the byte-count HUD in the
//                      top-left corner (bytes fetched vs archive size, tiles resident,
//                      records painted), driven by the real `?archive=<url>` path.
//   2. 3D orbit     -- the 3D layer-stack panel is opened (command `view.panel_3d`) and
//                      orbited with a smooth mouse drag.
//   3. Share        -- the copy-permalink command (`share.copy_permalink`) fires, so the
//                      "Copied a permalink to this view" toast appears.
//
// REMOTE INPUT: by default this streams the live 3.01 GiB archive
//   https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla
// (the same featured die the Start screen links, crates/reticle-app/src/startscreen.rs).
// So the streaming HUD shows real byte counts (e.g. 188 KiB / 3.01 GiB / 0.006%). This
// URL is REMOTE and needs working internet at capture time. Override with env:
//   HERO_URL     -- full page URL to open (overrides everything below)
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

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..");
const DIST_PORT = Number(process.env.PORT || 8080);

// The featured live die (real remote archive). Override with ARCHIVE_URL; override the
// whole page URL with HERO_URL.
const ARCHIVE_URL =
  process.env.ARCHIVE_URL || "https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla";

const OUT = join(repo, "docs", "media", "hero-gallery-stream.gif");
const FRAMES_DIR = join(repo, "scratch", "hero-frames");

// Capture cadence and per-phase frame counts (~10 fps).
const FPS = 10;
const STREAM_FRAMES = 20; // ~2.0 s of streaming with the byte HUD climbing
const ORBIT_FRAMES = 25; // ~2.5 s of 3D orbit
const SHARE_FRAMES = 15; // ~1.5 s of the "link copied" toast

// Headed window size. Kept modest so the assembled GIF stays under the 6 MB budget.
const VIEW_W = 1000;
const VIEW_H = 680;
// gifski output width (<= 900 to stay comfortably under 6 MB for ~60 frames).
const GIF_WIDTH = 900;

// Orbit drag anchor, in CSS pixels. The 3D stack opens as a BOTTOM panel
// (egui::Panel::bottom("view.panel_3d"), crates/reticle-app/src/app.rs), so the drag is
// aimed at the lower-centre of the window where that panel renders. The orchestrator can
// nudge these while refining framing on the GPU.
const ORBIT_CX = VIEW_W / 2;
const ORBIT_CY = Math.round(VIEW_H * 0.66);
const ORBIT_RX = 220;

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

// One globally-ordered frame counter so gifski assembles the three phases in sequence.
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

// Dispatch a command by opening the palette, typing a distinctive fragment of its label,
// and pressing Enter -- the same path the passing demo-phase3 / registry-sweep specs use.
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

  // Serve the built release bundle, reusing the e2e static server.
  spawnChild("node", ["serve-dist.mjs"], { PORT: String(DIST_PORT) });
  await waitForHttp(`http://127.0.0.1:${DIST_PORT}/`);

  const base = `http://127.0.0.1:${DIST_PORT}`;
  const url = process.env.HERO_URL || `${base}/?archive=${encodeURIComponent(ARCHIVE_URL)}`;

  // No WebGPU flags: force the WebGL2 path by deleting navigator.gpu in every frame (see
  // the host caveat in the header comment).
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

    // The renderer started: main.rs hides #overlay only after eframe's wgpu backend
    // initialised on the canvas (the genuine "it rendered" signal). Remote archive + wasm
    // boot can be slow, so allow a generous window.
    await page.locator("#overlay").waitFor({ state: "hidden", timeout: 90_000 });
    await page.locator("#reticle-canvas").waitFor({ state: "visible", timeout: 30_000 });
    // Generous fixed settle for the wasm renderer and the first ranged tile fetches.
    await page.waitForTimeout(6_000);

    // Dismiss the first-visit chrome so the hero shows a clean canvas: the bottom-left
    // WebGL2 graphics notice and the bottom-right "Get started" onboarding checklist are
    // egui-painted (not DOM), so click their "Dismiss" buttons by position, then Escape as
    // a backstop.
    await page.mouse.click(Math.round(VIEW_W * 0.16), Math.round(VIEW_H * 0.89));
    await page.waitForTimeout(250);
    await page.mouse.click(Math.round(VIEW_W * 0.742), Math.round(VIEW_H * 0.89));
    await page.waitForTimeout(250);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(500);

    // --- Phase 1: streaming HUD -----------------------------------------------------
    // Wait until bytes have actually moved over the wire and the app is painting real
    // records (render_nonblank), then capture ~2 s while the byte HUD keeps climbing.
    await pollStats(
      page,
      (s) => (s.archive_bytes_fetched ?? 0) > 0 && (s.render_nonblank === true || (s.archive_records_painted ?? 0) > 0),
      "streaming HUD never populated (archive_bytes_fetched / render_nonblank)",
      60_000,
    );
    await captureFrames(page, STREAM_FRAMES);

    // --- Phase 2: 3D layer-stack orbit ----------------------------------------------
    // A single harmless click first so keyboard focus lands on the app, then open the 3D
    // stack panel via its real command id (view.panel_3d, label "3D stack panel").
    await page.mouse.click(VIEW_W / 2, Math.round(VIEW_H * 0.4));
    await palette(page, "3D stack");
    await page.waitForTimeout(500);

    // Smooth mouse-drag orbit across the panel: press, sweep left->right with a gentle
    // vertical arc capturing each frame, then release.
    await page.mouse.move(ORBIT_CX - ORBIT_RX, ORBIT_CY);
    await page.mouse.down();
    for (let i = 0; i < ORBIT_FRAMES; i++) {
      const t = ORBIT_FRAMES > 1 ? i / (ORBIT_FRAMES - 1) : 1;
      const mx = ORBIT_CX - ORBIT_RX + 2 * ORBIT_RX * t;
      const my = ORBIT_CY + Math.sin(t * Math.PI) * (ORBIT_RX * 0.22);
      await page.mouse.move(mx, my, { steps: 3 });
      await snap(page);
      await page.waitForTimeout(1000 / FPS);
    }
    await page.mouse.up();

    // --- Phase 3: copy permalink toast ----------------------------------------------
    // Fire the real share command (share.copy_permalink); the app stages the link and
    // shows the "Copied a permalink to this view" toast. Capture ~1.5 s.
    await palette(page, "Copy permalink");
    await captureFrames(page, SHARE_FRAMES);
  } finally {
    await context.close();
    await browser.close();
  }

  // Assemble with gifski. Downscale so the frame stays comfortably under the repo's 6 MB
  // tour-GIF budget while staying legible.
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

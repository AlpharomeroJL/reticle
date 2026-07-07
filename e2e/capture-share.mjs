// Real two-context share GIF capture (lane v8-1e). NOT part of the test run.
//
// This is the browser-and-relay capture the README's share section needs but the native
// UI-capture harness (xtask capture-ui) cannot record: a live-shared session is two
// browser contexts talking over the relay, not one native window. So this launches the
// relay + the bundle server, opens ONE headed Chromium window holding TWO iframes of the
// bundle side by side (the left an editor sharing a live room via `?share=1`, which also
// places one scripted rect via `?e2e-edit=1`; the right the read-only viewer of that room
// via `?view=viewer`), animates the sharer's cursor so the viewer shows the remote
// presence live, screenshots the window at ~10 fps, and assembles assets/tour-share.gif
// with gifski (on PATH). The two iframes are genuinely separate browsing contexts whose
// only channel is the relay, so the GIF shows the real mirror, not a staged mock.
//
// Run via `just capture-share`. Requires: the Trunk bundle built (crates/web/dist), the
// reticle-server binary built, gifski on PATH, and a GPU/display (opens a real window).
import { chromium } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import { join, dirname } from "node:path";
import { mkdirSync, rmSync, readdirSync, existsSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..");
const DIST_PORT = Number(process.env.PORT || 8080);
const RELAY = process.env.CAPTURE_RELAY || "127.0.0.1:3030";
const RELAY_HEALTH = Number(process.env.RELAY_HEALTH_PORT || 3031);
const ROOM = "tour-share";
const OUT = join(repo, "assets", "tour-share.gif");
const FRAMES_DIR = join(repo, "scratch", "capture-share-frames");

// Capture cadence: ~10 fps for ~8 s.
const FPS = 10;
const FRAMES = 80;
// Each pane, in CSS pixels; the window holds two side by side plus a caption strip.
const PANE_W = 560;
const PANE_H = 430;

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

async function main() {
  if (!existsSync(join(repo, "crates", "web", "dist", "index.html"))) {
    throw new Error("crates/web/dist is missing; build the bundle first (just web-build).");
  }

  // Bundle server + relay (with its health endpoint), reusing the e2e serve scripts.
  spawnChild("node", ["serve-dist.mjs"], { PORT: String(DIST_PORT) });
  spawnChild("node", ["serve-relay.mjs"], {
    RELAY_PORT: RELAY.split(":")[1] ?? "3030",
    RELAY_HEALTH_PORT: String(RELAY_HEALTH),
  });
  await waitForHttp(`http://127.0.0.1:${DIST_PORT}/`);
  await waitForHttp(`http://127.0.0.1:${RELAY_HEALTH}/`);

  // A tiny host page (served locally so the iframes are same-origin as their parent) that
  // frames the editor and the viewer side by side with captions.
  const base = `http://127.0.0.1:${DIST_PORT}`;
  const relayParam = encodeURIComponent(RELAY);
  const editorSrc = `${base}/?share=1&e2e-edit=1&room=${ROOM}&relay=${relayParam}`;
  const viewerSrc = `${base}/?view=viewer&room=${ROOM}&relay=${relayParam}`;
  const host = `<!doctype html><html><head><meta charset="utf-8"><style>
    html,body{margin:0;background:#0b0e14;font:13px/1.4 system-ui,sans-serif;color:#c8d0e0}
    .row{display:flex;gap:10px;padding:10px}
    .pane{width:${PANE_W}px}
    .cap{padding:6px 8px;font-weight:600;letter-spacing:.02em}
    .cap .dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:7px;vertical-align:middle}
    iframe{width:${PANE_W}px;height:${PANE_H}px;border:1px solid #223;border-radius:6px;background:#000}
  </style></head><body><div class="row">
    <div class="pane"><div class="cap"><span class="dot" style="background:#5ac8fa"></span>Editor: sharing a live room</div>
      <iframe src="${editorSrc}"></iframe></div>
    <div class="pane"><div class="cap"><span class="dot" style="background:#34c759"></span>Viewer: read-only, mirroring over the relay</div>
      <iframe src="${viewerSrc}"></iframe></div>
  </div></body></html>`;

  const hostServer = createServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end(host);
  });
  await new Promise((r) => hostServer.listen(0, "127.0.0.1", r));
  const hostPort = hostServer.address().port;

  const winW = PANE_W * 2 + 30;
  const winH = PANE_H + 46;
  const browser = await chromium.launch({
    headless: false,
    args: ["--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPU"],
  });
  const context = await browser.newContext({ viewport: { width: winW, height: winH } });
  const page = await context.newPage();

  rmSync(FRAMES_DIR, { recursive: true, force: true });
  mkdirSync(FRAMES_DIR, { recursive: true });

  try {
    await page.goto(`http://127.0.0.1:${hostPort}/`);
    // Give both wasm apps time to boot (renderer up) and the viewer to receive the
    // sharer's first frames over the relay. Cross-origin iframes can't be polled, so this
    // is a generous fixed settle.
    await page.waitForTimeout(12_000);

    // The sharer pane's canvas region (left iframe, below its caption strip).
    const ex0 = 11;
    const ey0 = 46;
    const cxA = ex0 + PANE_W / 2;
    const cyA = ey0 + PANE_H / 2;

    for (let i = 0; i < FRAMES; i++) {
      // Trace the sharer's cursor over its canvas in a smooth loop; each move publishes
      // presence, so the viewer draws the remote cursor moving in real time.
      const t = (i / FRAMES) * Math.PI * 2;
      const mx = cxA + Math.cos(t) * (PANE_W * 0.28);
      const my = cyA + Math.sin(t * 2) * (PANE_H * 0.22);
      await page.mouse.move(mx, my);
      await page.screenshot({
        path: join(FRAMES_DIR, `f${String(i).padStart(3, "0")}.png`),
      });
      await new Promise((r) => setTimeout(r, 1000 / FPS));
    }
  } finally {
    await context.close();
    await browser.close();
    hostServer.close();
  }

  // Assemble with gifski. Downscale so the two-pane frame stays comfortably under the
  // repo's 6 MB tour-GIF budget while staying legible.
  const frames = readdirSync(FRAMES_DIR)
    .filter((f) => f.endsWith(".png"))
    .sort()
    .map((f) => join(FRAMES_DIR, f));
  mkdirSync(dirname(OUT), { recursive: true });
  const res = spawnSync(
    "gifski",
    ["--fps", String(FPS), "--quality", "80", "--width", "960", "-o", OUT, ...frames],
    { stdio: "inherit" },
  );
  if (res.status !== 0) throw new Error(`gifski failed with status ${res.status}`);
  console.log(`capture-share: wrote ${OUT}`);
  rmSync(FRAMES_DIR, { recursive: true, force: true });
}

main()
  .then(() => {
    cleanup();
    process.exit(0);
  })
  .catch((err) => {
    console.error("capture-share failed:", err);
    cleanup();
    process.exit(1);
  });

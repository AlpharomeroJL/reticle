// Standalone gh-pages subpath boot check (no Playwright test runner required).
//
// It serves the Trunk bundle under `/reticle/` (via serve-subpath.mjs's logic,
// inlined so this file runs on its own), opens http://127.0.0.1:<port>/reticle/
// in headless Chromium, and asserts:
//   * no request for a .js or .wasm asset returned a 4xx/5xx (the base-path bug),
//   * every .js/.wasm request path is under /reticle/, and
//   * the app boots: #overlay becomes hidden (web/src/main.rs hides it only after
//     eframe::WebRunner::start().await resolves Ok), and no fatal console error.
//
// This is the belt-and-suspenders companion to the `ghpages-subpath` Playwright
// project: it needs only `@playwright/test`'s bundled chromium and a dist built
// with `--public-url /reticle/`. Run it after building that bundle:
//
//     cd crates/web && trunk build index.html --release --public-url /reticle/
//     node e2e/subpath-check.mjs
//
// Exit 0 on success, 1 on any failure, with a one-line reason.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, extname, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const here = fileURLToPath(new URL(".", import.meta.url));
const ROOT = join(here, "..", "crates", "web", "dist");
const HOST = "127.0.0.1";
const PORT = Number(process.env.PORT || 8091);
const PREFIX = "/reticle";
const BASE = `http://${HOST}:${PORT}${PREFIX}/`;

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".ico": "image/x-icon",
};

function startServer() {
  const server = createServer(async (req, res) => {
    try {
      const pathname = decodeURIComponent(new URL(req.url, `http://${HOST}`).pathname);
      if (pathname !== PREFIX && !pathname.startsWith(PREFIX + "/")) {
        res.writeHead(404).end(`outside ${PREFIX}`);
        return;
      }
      let rel = pathname.slice(PREFIX.length);
      if (rel === "" || rel === "/") rel = "/index.html";
      const abs = normalize(join(ROOT, rel));
      if (!abs.startsWith(ROOT)) {
        res.writeHead(403).end("forbidden");
        return;
      }
      let target = abs;
      try {
        const s = await stat(abs);
        if (s.isDirectory()) target = join(abs, "index.html");
      } catch {
        res.writeHead(404).end("not found");
        return;
      }
      const body = await readFile(target);
      res.writeHead(200, {
        "content-type": TYPES[extname(target)] || "application/octet-stream",
        "cache-control": "no-store",
      });
      res.end(body);
    } catch (err) {
      res.writeHead(500).end(String(err));
    }
  });
  return new Promise((resolve) => server.listen(PORT, HOST, () => resolve(server)));
}

function fail(msg) {
  console.error(`subpath-check: FAIL - ${msg}`);
  process.exitCode = 1;
}

async function main() {
  // The dist must exist and be built for the subpath.
  try {
    await stat(join(ROOT, "index.html"));
  } catch {
    fail(`no bundle at ${ROOT}. Build it first: trunk build index.html --release --public-url /reticle/`);
    return;
  }

  const server = await startServer();
  console.log(`subpath-check: serving ${ROOT} at ${BASE}`);

  const browser = await chromium.launch({
    headless: true,
    // Software GL so headless has a WebGL2 implementation to fall back onto.
    args: ["--use-angle=swiftshader", "--use-gl=angle"],
  });
  const badAssets = [];
  const offPrefix = [];
  const fatals = [];
  try {
    const page = await browser.newPage();

    page.on("requestfailed", (req) => {
      const u = req.url();
      if (/\.(js|wasm)(\?|$)/.test(u)) badAssets.push(`${u} (${req.failure()?.errorText ?? "failed"})`);
    });
    page.on("response", (resp) => {
      const u = resp.url();
      if (/\.(js|wasm)(\?|$)/.test(u)) {
        if (resp.status() >= 400) badAssets.push(`${u} -> HTTP ${resp.status()}`);
        try {
          const p = new URL(u).pathname;
          if (!p.startsWith(PREFIX + "/")) offPrefix.push(u);
        } catch {
          /* ignore non-URL */
        }
      }
    });
    page.on("console", (m) => {
      if (m.type() === "error") {
        const t = m.text().toLowerCase();
        if (
          t.includes("panic") ||
          t.includes("failed to start the reticle web app") ||
          t.includes("is missing a #reticle-canvas")
        ) {
          fatals.push(m.text());
        }
      }
    });
    page.on("pageerror", (e) => fatals.push(String(e)));

    await page.goto(BASE, { waitUntil: "load", timeout: 60000 });

    // The genuine boot signal: main.rs hides #overlay only after the renderer
    // initialized. Wait for it to become hidden.
    await page.waitForSelector("#overlay", { state: "hidden", timeout: 60000 });

    // The canvas must have a real size.
    const box = await page.locator("#reticle-canvas").boundingBox();
    if (!box || box.width <= 0 || box.height <= 0) {
      fail("canvas has no visible area after boot");
    }
  } catch (err) {
    fail(`boot did not complete: ${String(err)}`);
  } finally {
    await browser.close();
    server.close();
  }

  if (badAssets.length) fail(`js/wasm asset(s) failed to load:\n  ${badAssets.join("\n  ")}`);
  if (offPrefix.length) fail(`js/wasm asset(s) not under ${PREFIX}/:\n  ${offPrefix.join("\n  ")}`);
  if (fatals.length) fail(`fatal console/page error(s):\n  ${fatals.join("\n  ")}`);

  if (!process.exitCode) {
    console.log(`subpath-check: PASS - app booted under ${PREFIX}/ with no asset 404s.`);
  }
}

main();

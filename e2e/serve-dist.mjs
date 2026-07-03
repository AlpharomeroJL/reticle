// Minimal dependency-free static server for the Trunk-built web bundle.
// Playwright's webServer starts this, waits for the port, then runs the specs.
// Serves ../crates/web/dist with correct content types (application/wasm is the
// one that matters: the browser refuses to stream-compile wasm otherwise).
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, extname, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const ROOT = join(here, "..", "crates", "web", "dist");
const PORT = Number(process.env.PORT || 8080);
const HOST = process.env.HOST || "127.0.0.1";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
};

const server = createServer(async (req, res) => {
  try {
    let rel = decodeURIComponent(new URL(req.url, `http://${HOST}`).pathname);
    if (rel === "/" || rel === "") rel = "/index.html";
    // Contain the path to ROOT; reject traversal.
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
      // Unknown path: fall back to index.html (single-page harness).
      target = join(ROOT, "index.html");
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

server.listen(PORT, HOST, () => {
  console.log(`serve-dist: http://${HOST}:${PORT} -> ${ROOT}`);
});

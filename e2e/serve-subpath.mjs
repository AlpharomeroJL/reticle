// Static server that serves the Trunk bundle under the `/reticle/` SUBPATH,
// exactly as GitHub Pages serves https://alpharomerojl.github.io/reticle/.
//
// This is the deploy-shaped counterpart to serve-dist.mjs (which serves at root).
// It exists so the ghpages-subpath Playwright project can prove the app boots when
// mounted under /reticle/. A bundle built without `--public-url /reticle/` emits
// asset refs at absolute root (`/web-<hash>.js`); under this server those 404,
// which is the exact front-door regression we want to fail BEFORE deploy.
//
// Anything outside /reticle/ returns 404 (like Pages), so a stray absolute-root
// fetch is a hard failure rather than being silently rewritten to index.html.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, extname, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const ROOT = join(here, "..", "crates", "web", "dist");
const PORT = Number(process.env.PORT || 8081);
const HOST = process.env.HOST || "127.0.0.1";
// The subpath the site is deployed under. Keep the trailing slash off here; we
// match "/reticle" and "/reticle/...".
const PREFIX = process.env.SUBPATH || "/reticle";

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
    const url = new URL(req.url, `http://${HOST}`);
    let pathname = decodeURIComponent(url.pathname);

    // Everything must live under the deploy prefix. A request outside it 404s,
    // mirroring Pages and turning a bad absolute-root asset ref into a real error.
    if (pathname !== PREFIX && !pathname.startsWith(PREFIX + "/")) {
      res.writeHead(404, { "content-type": "text/plain" });
      res.end(`not found (outside ${PREFIX}/): ${pathname}`);
      return;
    }

    // Strip the prefix to get the path within the dist root.
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
      // Unknown asset under the prefix: 404 rather than masking a missing file.
      res.writeHead(404, { "content-type": "text/plain" });
      res.end(`not found: ${pathname}`);
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

server.listen(PORT, HOST, () => {
  console.log(`serve-subpath: http://${HOST}:${PORT}${PREFIX}/ -> ${ROOT}`);
});

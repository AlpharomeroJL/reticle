// Minimal dependency-free static server WITH HTTP Range support, for the served-archive
// e2e (lane v8-2e). Serves the committed `.rtla` fixture(s) under `e2e/fixtures/` so the
// browser's HttpRangeTileSource can fetch the header and each tile with
// `Range: bytes=start-end` and get a `206 Partial Content` back — which the bundle's own
// static server (serve-dist.mjs) does NOT do (it always answers `200` with the full body).
//
// It also answers permissive CORS (including the OPTIONS preflight the non-safelisted
// `Range` request header triggers) and exposes `Content-Range`/`ETag`, because the fixture
// is served from a different port than the bundle and so every fetch is cross-origin.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const ROOT = join(here, "fixtures");
const PORT = Number(process.env.ARCHIVE_PORT || 8082);
const HOST = process.env.HOST || "127.0.0.1";

/// Parses an HTTP `Range` header against a known `size`, returning a closed
/// `{ start, end }` byte range (inclusive), `null` for "no/!bytes range" (serve full), or
/// `"unsatisfiable"`. Handles closed (`bytes=a-b`), open-ended (`bytes=a-`), and suffix
/// (`bytes=-n`) forms, clamping to the file.
function parseRange(header, size) {
  if (!header || !header.startsWith("bytes=")) return null;
  const spec = header.slice("bytes=".length).trim();
  // Only a single range is supported (the tile reader never asks for more).
  if (spec.includes(",")) return "unsatisfiable";
  const dash = spec.indexOf("-");
  if (dash < 0) return "unsatisfiable";
  const startText = spec.slice(0, dash).trim();
  const endText = spec.slice(dash + 1).trim();

  let start;
  let end;
  if (startText === "") {
    // Suffix range: the last `endText` bytes.
    const suffix = Number(endText);
    if (!Number.isFinite(suffix) || suffix <= 0) return "unsatisfiable";
    start = Math.max(0, size - suffix);
    end = size - 1;
  } else {
    start = Number(startText);
    if (!Number.isInteger(start) || start < 0) return "unsatisfiable";
    if (endText === "") {
      end = size - 1;
    } else {
      end = Number(endText);
      if (!Number.isInteger(end)) return "unsatisfiable";
    }
    if (end > size - 1) end = size - 1;
  }
  if (start > end || start >= size) return "unsatisfiable";
  return { start, end };
}

function corsHeaders() {
  return {
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET, HEAD, OPTIONS",
    "access-control-allow-headers": "Range",
    "access-control-expose-headers":
      "Content-Range, Content-Length, Accept-Ranges, ETag",
  };
}

const server = createServer(async (req, res) => {
  const cors = corsHeaders();
  try {
    // Preflight for the `Range` request header (a non-safelisted header makes the
    // cross-origin fetch "non-simple", so the browser sends OPTIONS first).
    if (req.method === "OPTIONS") {
      res.writeHead(204, cors);
      res.end();
      return;
    }

    const rel = decodeURIComponent(new URL(req.url, `http://${HOST}`).pathname);
    const abs = normalize(join(ROOT, rel));
    if (!abs.startsWith(ROOT)) {
      res.writeHead(403, cors).end("forbidden");
      return;
    }

    let info;
    try {
      info = await stat(abs);
    } catch {
      res.writeHead(404, cors).end("not found");
      return;
    }
    if (info.isDirectory()) {
      res.writeHead(404, cors).end("not found");
      return;
    }

    const size = info.size;
    const etag = `"${size.toString(16)}-${Math.trunc(info.mtimeMs).toString(16)}"`;
    const base = {
      ...cors,
      "content-type": "application/octet-stream",
      "accept-ranges": "bytes",
      "cache-control": "no-store",
      etag,
    };

    if (req.method === "HEAD") {
      res.writeHead(200, { ...base, "content-length": String(size) });
      res.end();
      return;
    }

    const range = parseRange(req.headers.range, size);
    if (range === "unsatisfiable") {
      res.writeHead(416, { ...base, "content-range": `bytes */${size}` });
      res.end();
      return;
    }

    const body = await readFile(abs);
    if (range === null) {
      res.writeHead(200, { ...base, "content-length": String(size) });
      res.end(body);
      return;
    }

    const { start, end } = range;
    const slice = body.subarray(start, end + 1);
    res.writeHead(206, {
      ...base,
      "content-range": `bytes ${start}-${end}/${size}`,
      "content-length": String(slice.length),
    });
    res.end(slice);
  } catch (err) {
    res.writeHead(500, cors).end(String(err));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`serve-archive: http://${HOST}:${PORT} -> ${ROOT} (Range enabled)`);
});

// Reticle service worker: caches the app shell so the installed PWA loads
// offline.
//
// Subpath correctness: the app is served both at the dev root (`/`) and under
// the gh-pages subpath (`https://<user>.github.io/reticle/`). Every cached URL
// is derived from `self.registration.scope` (the absolute URL the SW controls),
// so the same worker works in BOTH deployments with NO hardcoded leading "/".
//
// Strategy:
//   * install  - pre-cache the navigation shell (the scope root and its
//                 index.html). The hashed `web-<hash>.js` / `web-<hash>_bg.wasm`
//                 names are NOT known here, so they are discovered and cached at
//                 runtime by the fetch handler on the first controlled load.
//   * activate - drop caches from older versions, then claim open clients so the
//                 already-open page becomes controlled without a manual reload.
//   * fetch    - navigation requests: network-first, falling back to the cached
//                 shell when offline. Other same-origin GETs (js/wasm/json/png):
//                 cache-first, populating the cache from the network on a miss.

const CACHE_VERSION = "reticle-pwa-v1";

// The scope root, e.g. "http://host/" or "http://host/reticle/". Everything the
// worker caches is resolved relative to this so it is subpath-correct.
const SCOPE = self.registration.scope;
const SHELL_URLS = [
  // The scope root itself (what a bare navigation to the app requests) and the
  // explicit index.html both map to the app shell HTML.
  new URL("./", SCOPE).toString(),
  new URL("./index.html", SCOPE).toString(),
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE_VERSION);
      // Best-effort: a failed shell fetch must not abort the install (e.g. a
      // transient error on one URL should not wedge the worker).
      await Promise.allSettled(
        SHELL_URLS.map((url) =>
          cache.add(new Request(url, { cache: "reload" })),
        ),
      );
      // Take over as the active worker as soon as install completes.
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(
        names
          .filter((name) => name !== CACHE_VERSION)
          .map((name) => caches.delete(name)),
      );
      // Control already-open clients so the current page is served by the SW
      // on its next request without needing a manual second reload.
      await self.clients.claim();
    })(),
  );
});

// True when the request targets our own origin (we never cache cross-origin
// requests such as a remote `?archive=<url>` stream).
function isSameOrigin(request) {
  return new URL(request.url).origin === self.location.origin;
}

// The virtual path prefix (relative to scope) under which converted `.rtla` archives in
// OPFS are served over HTTP Range (lane v8-6c). `?archive=<scope>opfs-archive/<opfsPath>`
// then streams straight through the existing HttpRangeTileSource path with no new reader:
// the worker writes the archive to OPFS, and this handler reads it back and answers
// ranged GETs from it. Same-origin, so no CORS/preflight is involved.
const OPFS_ARCHIVE_PREFIX = new URL("./opfs-archive/", SCOPE).toString();

// Reads the OPFS file at a "dir/dir/file" path, returning a File (Blob) or null if any
// segment is missing or OPFS is unavailable in this worker.
async function readOpfsFile(opfsPath) {
  if (!(navigator.storage && navigator.storage.getDirectory)) return null;
  let dir;
  try {
    dir = await navigator.storage.getDirectory();
  } catch {
    return null;
  }
  const parts = opfsPath.split("/").filter((p) => p.length > 0);
  if (parts.length === 0) return null;
  const fileName = parts.pop();
  for (const part of parts) {
    try {
      dir = await dir.getDirectoryHandle(part);
    } catch {
      return null;
    }
  }
  try {
    const handle = await dir.getFileHandle(fileName);
    return await handle.getFile();
  } catch {
    return null;
  }
}

// Serves an OPFS-backed archive, honoring a single `bytes=start-end` Range so the
// streaming reader can fetch the preamble, header/directory, and each tile by offset.
async function serveOpfsArchive(request, opfsPath) {
  const file = await readOpfsFile(opfsPath);
  if (!file) {
    return new Response("archive not found in OPFS", { status: 404 });
  }
  const total = file.size;
  const range = request.headers.get("Range");
  if (!range) {
    return new Response(file, {
      status: 200,
      headers: {
        "Content-Type": "application/octet-stream",
        "Accept-Ranges": "bytes",
        "Content-Length": String(total),
      },
    });
  }
  // Parse "bytes=start-end" (end optional, inclusive), clamped to the file.
  const match = /^bytes=(\d*)-(\d*)$/.exec(range.trim());
  if (!match) {
    return new Response("malformed Range", {
      status: 416,
      headers: { "Content-Range": `bytes */${total}` },
    });
  }
  let start = match[1] === "" ? 0 : Number(match[1]);
  let end = match[2] === "" ? total - 1 : Number(match[2]);
  if (Number.isNaN(start) || Number.isNaN(end) || start > end || start >= total) {
    return new Response("range not satisfiable", {
      status: 416,
      headers: { "Content-Range": `bytes */${total}` },
    });
  }
  end = Math.min(end, total - 1);
  const slice = file.slice(start, end + 1);
  return new Response(slice, {
    status: 206,
    headers: {
      "Content-Type": "application/octet-stream",
      "Accept-Ranges": "bytes",
      "Content-Range": `bytes ${start}-${end}/${total}`,
      "Content-Length": String(end - start + 1),
    },
  });
}

self.addEventListener("fetch", (event) => {
  const request = event.request;

  // OPFS archive bridge (lane v8-6c): serve converted archives back over Range from OPFS,
  // before the cache-first logic (these are never cached; they change on every convert).
  if (request.method === "GET" && request.url.startsWith(OPFS_ARCHIVE_PREFIX)) {
    const opfsPath = decodeURIComponent(request.url.slice(OPFS_ARCHIVE_PREFIX.length));
    event.respondWith(serveOpfsArchive(request, opfsPath));
    return;
  }

  // Only GET is cacheable; let everything else hit the network untouched.
  if (request.method !== "GET" || !isSameOrigin(request)) return;

  // Navigation (the HTML document): network-first so a live deploy always wins,
  // with the cached shell as the offline fallback.
  if (request.mode === "navigate") {
    event.respondWith(
      (async () => {
        try {
          const response = await fetch(request);
          const cache = await caches.open(CACHE_VERSION);
          cache.put(request, response.clone());
          return response;
        } catch (err) {
          const cache = await caches.open(CACHE_VERSION);
          // Fall back to the exact request, then to the scope shell.
          const cached =
            (await cache.match(request)) ||
            (await cache.match(new URL("./index.html", SCOPE).toString())) ||
            (await cache.match(new URL("./", SCOPE).toString()));
          if (cached) return cached;
          throw err;
        }
      })(),
    );
    return;
  }

  // Static assets (js/wasm/json/png/...): cache-first. On a miss, fetch and
  // populate the cache so the hashed bundle is available offline next time.
  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_VERSION);
      const cached = await cache.match(request);
      if (cached) return cached;
      const response = await fetch(request);
      // Only cache successful, basic (same-origin) responses.
      if (response && response.status === 200 && response.type === "basic") {
        cache.put(request, response.clone());
      }
      return response;
    })(),
  );
});

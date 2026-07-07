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

self.addEventListener("fetch", (event) => {
  const request = event.request;

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

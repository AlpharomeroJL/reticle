# 0099, Service-worker cache versioning and per-asset freshness

## Context

The PWA service worker (ADR 0078) keyed its cache on a fixed name,
`reticle-pwa-v1`, and served every shell asset cache-first. The `activate` purge
drops caches whose name is not the current one, but nothing ever changed the
name, so across deploys it never dropped anything. Two asset classes behaved very
differently under that rule:

- **Content-hashed bundle assets** (`web-<hash>.js` / `web-<hash>_bg.wasm`) dodge
  staleness for free: the hash in the filename changes every deploy, so a new
  bundle is a cache miss and is fetched fresh. Navigation (`index.html`) is
  network-first, so the HTML that names the new hash also refreshes online.
- **Stable-named shell assets** keep the SAME filename every deploy
  (`convert_worker.js` / `convert_worker_bg.wasm`, `manifest.json`, the icons).
  Cache-first with a never-purged cache served these from a previous deploy
  indefinitely. A new convert worker mixed with the new main bundle is the real
  hazard: the two wasm modules must match.

## Decision

**Version the cache name and split the fetch strategy by asset immutability.**

1. Bump `CACHE_VERSION` to `reticle-pwa-v2`. Because `activate` deletes every
   cache whose name is not the current version, the one-time bump drops the whole
   stale `reticle-pwa-v1` cache on the next controlled load, so a returning
   visitor cannot keep a stale stable-named asset.

2. In the fetch handler, keep **cache-first** for immutable content-hashed bundle
   assets (matched by a `-<hex>` content-hash segment) and use **network-first**
   for every other shell asset. Immutable assets are safe to cache-first (the name
   changes when the bytes change) and caching them is the offline contract;
   stable-named assets are fetched fresh when online, with the cached copy only as
   the offline fallback, so a deploy never mixes an old stable-named asset with the
   new bundle.

The worker script itself is a stable-named asset the browser byte-compares on its
own update check, so shipping this changed `sw.js` is what makes returning
visitors adopt the new strategy; no build-time hash injection is needed.

## Consequences

- A client holding the old `reticle-pwa-v1` cache upgrades on the next load with
  no manual clear. Proven by `just e2e-pwa` (`e2e/tests/pwa.spec.ts`): a seeded
  stale `reticle-pwa-v1` cache is gone after the worker activates.
- Offline still works: network-first falls back to the cached copy when the fetch
  throws, and the immutable bundle stays cache-first, so the existing offline
  shell test keeps passing.
- Honest scope. This fixes staleness of the convert worker, manifest, and icons.
  It is NOT the cause of the start-screen embedded-example (TT03, SKY130) white
  panes: those GDS bytes are `include_bytes!` compiled into the wasm, so a fresh
  hashed bundle always carries them. That symptom is chased on the render path
  (WebGPU/WebGL2 and the open path), not here.
- No Rust, GPU, or worker-binary code changed; this is a `sw.js`-only change,
  disjoint from the rest of the packet.

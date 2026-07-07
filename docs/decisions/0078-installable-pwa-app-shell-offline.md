# 0078, Installable PWA: a manifest, a scope-relative service worker, and an offline app shell

## Context

The deployed web bundle (`crates/web`, ADR 0026) was a plain page: it could not
be installed and it did nothing without the network. Two properties were worth
adding cheaply, entirely in the web shell (no Rust, no GPU): make it installable,
and let its app shell load offline. The one hard constraint is the deploy shape.
Production serves under a subpath, `https://alpharomerojl.github.io/reticle/`
(ADR 0027), so anything that assumes the domain root breaks the front door.

## Decision

**Ship a Progressive Web App as three web-shell pieces, all path-relative so the
same bundle is correct at the dev root and under `/reticle/`.**

1. **`crates/web/manifest.json`** with `start_url` and `scope` both `"."`
   (relative), `display: "standalone"`, the app's `#0b0e14` theme and background
   colors, and 192 and 512 px icons (a maskable variant included). It is linked
   from `index.html` with a relative `href="manifest.json"`.

2. **`crates/web/sw.js`**, a service worker that pre-caches the navigation shell
   on install, drops stale caches and claims clients on activate, and on fetch
   serves navigation network-first (offline fallback to the cached shell) with
   cache-first for static assets. **Every cached URL is derived from
   `self.registration.scope`**, not from a hardcoded `/`, which is what makes it
   subpath-correct. The hashed `web-<hash>.js` / `web-<hash>_bg.wasm` names are
   unknown at install time, so the worker discovers and caches them at runtime on
   the first controlled load rather than listing them in a pre-cache manifest.

3. **An inline registration script** in `index.html` that registers `./sw.js` on
   window load when service workers are supported and the page is not `file://`,
   logging (never throwing) on failure.

Trunk `copy-file` directives emit the manifest, worker, and icons into `dist`;
`scripts/deploy-pages.ps1` already copies `dist/*` wholesale, so the files reach
`gh-pages` under `/reticle/` with no deploy change.

## Consequences

- The app is installable and its shell loads offline. Proven by `just e2e-pwa`
  (`e2e/tests/pwa.spec.ts`): a linked, parseable manifest with name + start_url +
  resolvable icons, and a worker that registers and controls the page after a
  reload. Offline reload of the shell is asserted best-effort; it was PROVEN in
  the landing run (annotated `offline-reload`), but is not the hard gate in case
  Playwright's offline+SW behavior is flaky on a given host.
- Network-first navigation means a live deploy always wins; the cache is only a
  fallback, so a shipped update is never masked by a stale cached shell.
- Offline covers the **app shell**, not full app function. Streamed archives and
  live collaboration still need the network; caching those is out of scope here.
- No Rust, GPU, or worker code changed. The PWA is HTML + JSON + a service
  worker, disjoint from every other Wave 4 lane.

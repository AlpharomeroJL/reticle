# Install and offline (PWA)

The browser bundle is a Progressive Web App: it can be installed to the home
screen or desktop and it loads its app shell offline. Three pieces make that
work, all shipped from `crates/web` and copied verbatim into the Trunk `dist`:

- **`manifest.json`** declares the install metadata: the name, the standalone
  display mode, the `#0b0e14` theme and background colors, and 192 and 512 px
  icons (including a maskable variant). It is linked from `index.html` with a
  relative `href="manifest.json"`.
- **`sw.js`** is the service worker. On install it pre-caches the navigation
  shell; on activate it drops caches from older versions and claims open
  clients; on fetch it serves navigation network-first (falling back to the
  cached shell when offline) and static assets cache-first, populating the cache
  from the network on a miss.
- An inline registration script in `index.html` registers `./sw.js` on window
  load, when service workers are supported and the page is not opened from
  `file://`.

## Subpath correctness

Production serves the site under a subpath,
`https://alpharomerojl.github.io/reticle/`, not at the domain root. So every
PWA path is **relative**:

- the manifest link is `href="manifest.json"`, and its `start_url` and `scope`
  are both `"."`;
- the registration call is `register("./sw.js")`;
- inside the worker, every cached URL is derived from
  `self.registration.scope` (the absolute URL the worker controls), so the same
  worker is correct at the dev root **and** under `/reticle/` with no hardcoded
  leading `/`.

Because the hashed `web-<hash>.js` and `web-<hash>_bg.wasm` names are not known
at install time, the worker discovers and caches them at runtime on the first
controlled load rather than listing them in a pre-cache manifest.

## How it reaches the deploy artifact

The Trunk `copy-file` directives in `index.html` emit `manifest.json`, `sw.js`,
and both icons into `dist`. `scripts/deploy-pages.ps1` copies `dist/*`
wholesale into the Pages staging directory, so the PWA files ride along to
`gh-pages` under `/reticle/` with no extra deploy step.

## Proof

`just e2e-pwa` runs the `pwa` Playwright project (`e2e/tests/pwa.spec.ts`)
against the root-served `dist`. It asserts a linked, parseable manifest with a
name, a `start_url`, and resolvable icons; that the service worker registers and
controls the page after a reload; and, best-effort, that the app shell still
renders after the network is cut and the page reloaded. The subpath boot gate
(`just e2e-subpath`) separately proves the relative asset paths resolve under
`/reticle/`.

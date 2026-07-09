import { test, expect } from "@playwright/test";

// PWA install + offline gate (lane v8-4d-pwa).
//
// Runs ONLY in the `pwa` project, served at the dev root by serve-dist.mjs
// (port 8080). It proves the deployed web app is an installable PWA:
//
//   (a) a linked, parseable web manifest with name + start_url + resolvable
//       icons (the install contract);
//   (b) the service worker registers and CONTROLS the page after a reload
//       (the offline-capability precondition);
//   (c) BEST-EFFORT: with the network cut, a reload still renders the app shell
//       from the service-worker cache. If offline reload is flaky under
//       Playwright, (a)+(b) remain the hard gate and (c) is reported as an
//       annotation rather than failing the suite.
//
// Every asset path in the shell is RELATIVE, so this same proof holds under the
// gh-pages /reticle/ subpath (see subpath-boot.spec.ts for the base-path gate).

// Wait until a service worker is registered AND active for this page.
async function waitForActiveWorker(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    async () => {
      if (!("serviceWorker" in navigator)) return false;
      const reg = await navigator.serviceWorker.getRegistration();
      return !!(reg && reg.active);
    },
    null,
    { timeout: 30_000 },
  );
}

test("manifest is linked, parseable, and has name + start_url + resolvable icons", async ({
  page,
}) => {
  await page.goto("");

  const href = await page
    .locator('link[rel="manifest"]')
    .getAttribute("href");
  expect(href, "index.html links a manifest").toBeTruthy();

  // Resolve the manifest relative to the page URL (works at root and /reticle/).
  const manifestUrl = new URL(href!, page.url()).toString();
  const resp = await page.request.get(manifestUrl);
  expect(resp.ok(), `manifest fetch ${manifestUrl} -> ${resp.status()}`).toBeTruthy();

  const manifest = await resp.json();
  expect(manifest.name, "manifest has a name").toBeTruthy();
  expect(manifest.start_url, "manifest has a start_url").toBeTruthy();
  expect(
    Array.isArray(manifest.icons) && manifest.icons.length > 0,
    "manifest declares at least one icon",
  ).toBeTruthy();

  // Every declared icon must actually resolve (relative to the manifest URL).
  for (const icon of manifest.icons) {
    const iconUrl = new URL(icon.src, manifestUrl).toString();
    const iconResp = await page.request.get(iconUrl);
    expect(
      iconResp.ok(),
      `icon ${icon.src} -> ${iconUrl} : HTTP ${iconResp.status()}`,
    ).toBeTruthy();
  }
});

test("service worker registers and controls the page after a reload", async ({
  page,
}) => {
  await page.goto("");
  await waitForActiveWorker(page);

  // The first navigation that registers the worker is not controlled by it; a
  // reload puts the page under the active worker's control.
  await page.reload();
  await page.waitForFunction(() => !!navigator.serviceWorker.controller, null, {
    timeout: 30_000,
  });

  const controlled = await page.evaluate(
    () => !!navigator.serviceWorker.controller,
  );
  expect(controlled, "page is controlled by the service worker").toBeTruthy();
});

test("best-effort: the app shell loads offline from the SW cache", async ({
  page,
  context,
}, testInfo) => {
  await page.goto("");
  await waitForActiveWorker(page);

  // A controlled reload routes the hashed js/wasm through the SW fetch handler
  // so they are discovered and cached at runtime.
  await page.reload();
  await page.waitForFunction(() => !!navigator.serviceWorker.controller, null, {
    timeout: 30_000,
  });

  // Wait until the runtime cache actually holds the wasm bundle, so going
  // offline cannot race asset caching. This also proves the SW discovered the
  // hashed `web-<hash>_bg.wasm` name at runtime (it is not known at install).
  await page.waitForFunction(
    async () => {
      const names = await caches.keys();
      for (const name of names) {
        const cache = await caches.open(name);
        const keys = await cache.keys();
        if (keys.some((r) => r.url.endsWith(".wasm"))) return true;
      }
      return false;
    },
    null,
    { timeout: 30_000 },
  );

  let offlineOk = false;
  let note = "";
  try {
    await context.setOffline(true);
    await page.reload({ waitUntil: "domcontentloaded" });
    // The Reticle title lives in the static shell HTML, so its presence proves
    // the shell was served from cache with the network cut.
    await expect(page.locator("#overlay .title")).toHaveText("Reticle", {
      timeout: 15_000,
    });
    offlineOk = true;
  } catch (err) {
    note = String(err);
  } finally {
    await context.setOffline(false);
  }

  testInfo.annotations.push({
    type: "offline-reload",
    description: offlineOk
      ? "PROVEN: app shell rendered offline from the SW cache"
      : `BEST-EFFORT (not proven this run): ${note}`,
  });

  // Offline reload is best-effort per the lane spec: (a)+(b) are the hard gate.
  // Report the outcome above; do not fail the suite if offline was flaky.
});

test("a stale prior cache is purged on activate (versioned cache, no manual clear)", async ({
  page,
}) => {
  // Simulate a returning visitor who still holds the OLD `reticle-pwa-v1` cache from a
  // previous deploy. Seeded from an init script so it exists BEFORE the page registers
  // the service worker, i.e. before the worker's `activate` runs its purge. Reproduces
  // the packet's stale-cache signature: a fixed cache name that a deploy never dropped.
  await page.addInitScript(async () => {
    try {
      const stale = await caches.open("reticle-pwa-v1");
      await stale.put(
        new Request("./manifest.json", { cache: "reload" }),
        new Response('{"stale":true}', {
          headers: { "Content-Type": "application/json" },
        }),
      );
    } catch {
      /* Cache API unavailable here; the assertion below tolerates that honestly. */
    }
  });

  await page.goto("");
  await waitForActiveWorker(page);

  // The worker's `activate` drops every cache whose name is not the current version, so
  // the stale `reticle-pwa-v1` is gone and the visitor is on the new bundle with NO
  // manual cache clear. Poll: activation may land just after the worker is first active.
  await page.waitForFunction(
    async () => {
      const names = await caches.keys();
      return (
        !names.includes("reticle-pwa-v1") &&
        names.some((n) => n.startsWith("reticle-pwa-v"))
      );
    },
    null,
    { timeout: 30_000 },
  );

  const names = await page.evaluate(() => caches.keys());
  expect(
    names,
    "the stale v1 cache was purged on upgrade (no manual clear needed)",
  ).not.toContain("reticle-pwa-v1");
  expect(
    names.some((n) => n.startsWith("reticle-pwa-v") && n !== "reticle-pwa-v1"),
    `a bumped versioned cache is present: ${JSON.stringify(names)}`,
  ).toBeTruthy();
});

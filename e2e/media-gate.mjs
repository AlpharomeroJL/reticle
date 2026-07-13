// The mechanical media gate: the SAME color-histogram check the app e2e uses, shared
// by the headed capture scripts (capture-hero.mjs, capture-share.mjs) and the render
// guard spec (tests/demo-examples.spec.ts). No media ships that has not passed it.
//
// Every function drives a Playwright `page` (the capture scripts and the specs both do),
// so one implementation serves all callers with no extra dependency: a canvas screenshot
// is decoded in-page via an Image element (which draws to a 2D canvas cleanly, unlike the
// wgpu canvas that has no readback), then histogrammed into 5-bit-per-channel buckets.
//
// A CORRECT multi-layer render has several significant non-background buckets (background
// + layer colors + alpha blends). A starry frame (sparse LOD points on a dark field, the
// v8.2.0 hero failure) has only one or two; a blank/flat frame has none. So the gate
// counts NON-BACKGROUND buckets -- the single most common bucket is the background and is
// excluded -- and requires at least MIN_NONBG_BUCKETS.

/** A bucket must cover at least this fraction of pixels to be "significant". */
export const SIGNIFICANT_FRACTION = 0.005;

/** The minimum non-background significant buckets a shipped frame must show. */
export const MIN_NONBG_BUCKETS = 3;

/** The non-background buckets a streamed "money shot" (filled multi-layer die) reaches. */
export const MONEY_SHOT_NONBG = 6;

/**
 * Histograms pixels into 5-bit-per-channel buckets and returns the bucket statistics.
 *
 * In this egui app the WHOLE UI paints to one `#reticle-canvas`, so a plain canvas
 * screenshot includes the toolbar, panels, HUD, and minimap chrome (already several
 * colors) and cannot tell a starry geometry viewport from a filled one. Pass a `clip`
 * (a page-pixel rectangle over the geometry viewport, excluding panels/HUD/minimap) to
 * histogram only the geometry. `opts` may be a selector string or `{selector, clip}`.
 *
 * @param {import("@playwright/test").Page} page
 * @param {string | {selector?: string, clip?: {x: number, y: number, width: number, height: number}}} [opts]
 * @returns {Promise<{significant: number, nonBackground: number, coverage: number, width: number, height: number}>}
 *   `significant` counts buckets over {@link SIGNIFICANT_FRACTION} (background included);
 *   `nonBackground` excludes the single most common (background) bucket; `coverage` is the
 *   fraction of pixels NOT in the background bucket (a starry frame has low coverage).
 */
export async function histogram(page, opts = {}) {
  const { selector = "#reticle-canvas", clip } =
    typeof opts === "string" ? { selector: opts } : opts;
  const png = clip
    ? await page.screenshot({ clip })
    : await page.locator(selector).screenshot();
  const b64 = png.toString("base64");
  return page.evaluate(
    async ({ data, fraction }) => {
      const img = new Image();
      await new Promise((res, rej) => {
        img.onload = () => res();
        img.onerror = () => rej(new Error("decode"));
        img.src = "data:image/png;base64," + data;
      });
      const c = document.createElement("canvas");
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext("2d");
      ctx.drawImage(img, 0, 0);
      const { data: px } = ctx.getImageData(0, 0, c.width, c.height);
      const total = c.width * c.height;
      const counts = new Map();
      for (let i = 0; i < px.length; i += 4) {
        const key = ((px[i] >> 3) << 10) | ((px[i + 1] >> 3) << 5) | (px[i + 2] >> 3);
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
      let significant = 0;
      let background = 0;
      for (const n of counts.values()) {
        if (n >= total * fraction) significant++;
        if (n > background) background = n;
      }
      return {
        significant,
        nonBackground: Math.max(0, significant - 1),
        coverage: total > 0 ? 1 - background / total : 0,
        width: c.width,
        height: c.height,
      };
    },
    { data: b64, fraction: SIGNIFICANT_FRACTION },
  );
}

/**
 * The number of significant color buckets (background included). This is the exact check
 * `tests/demo-examples.spec.ts` asserts (>= 3); kept for that spec's compatibility.
 *
 * @param {import("@playwright/test").Page} page
 * @param {string} [selector]
 * @returns {Promise<number>}
 */
export async function significantColorBuckets(page, selector = "#reticle-canvas") {
  return (await histogram(page, selector)).significant;
}

/**
 * The number of significant NON-background buckets: the gate metric. A starry/blank/flat
 * frame returns fewer than {@link MIN_NONBG_BUCKETS}.
 *
 * @param {import("@playwright/test").Page} page
 * @param {string} [selector]
 * @returns {Promise<number>}
 */
export async function nonBackgroundBuckets(page, selector = "#reticle-canvas") {
  return (await histogram(page, selector)).nonBackground;
}

/**
 * Polls the canvas until it shows at least `minNonBg` non-background buckets, or throws
 * after `timeoutMs`. Use as an explicit render-settle wait before capturing.
 *
 * @param {import("@playwright/test").Page} page
 * @param {{selector?: string, minNonBg?: number, timeoutMs?: number, pollMs?: number, label?: string}} [opts]
 * @returns {Promise<{significant: number, nonBackground: number, coverage: number}>}
 */
export async function waitForColoredCanvas(page, opts = {}) {
  const {
    selector = "#reticle-canvas",
    clip,
    minNonBg = MIN_NONBG_BUCKETS,
    timeoutMs = 30_000,
    pollMs = 400,
    label = "canvas",
  } = opts;
  const deadline = Date.now() + timeoutMs;
  let last = { significant: 0, nonBackground: 0, coverage: 0 };
  for (;;) {
    last = await histogram(page, { selector, clip });
    if (last.nonBackground >= minNonBg) return last;
    if (Date.now() > deadline) {
      throw new Error(
        `${label} never reached ${minNonBg} non-background color buckets ` +
          `(last: ${JSON.stringify(last)}); a starry/blank frame would ship`,
      );
    }
    await page.waitForTimeout(pollMs);
  }
}

/**
 * Zooms the canvas in with real mouse-wheel input until it shows filled colored geometry
 * (at least `minNonBg` non-background buckets), the fix for the v8.2.0 starry hero: at
 * whole-die zoom a multi-gigabyte layout is sub-pixel and renders as correct-but-sparse
 * LOD points, so the capture must zoom in "until you hit the transistors" before it is
 * the money shot. Throws if filled geometry is never reached (reject-and-recapture).
 *
 * @param {import("@playwright/test").Page} page
 * @param {{selector?: string, center: {x: number, y: number}, minNonBg?: number, maxSteps?: number, wheelDelta?: number, settleMs?: number, label?: string}} opts
 * @returns {Promise<{steps: number, significant: number, nonBackground: number, coverage: number}>}
 */
export async function zoomToFilledGeometry(page, opts) {
  const {
    selector = "#reticle-canvas",
    clip,
    center,
    minNonBg = MONEY_SHOT_NONBG,
    maxSteps = 60,
    wheelDelta = -240,
    settleMs = 150,
    label = "streamed die",
  } = opts;
  let best = { significant: 0, nonBackground: 0, coverage: 0 };
  for (let step = 0; step <= maxSteps; step++) {
    const stats = await histogram(page, { selector, clip });
    if (stats.nonBackground > best.nonBackground) best = stats;
    if (stats.nonBackground >= minNonBg) {
      return { steps: step, ...stats };
    }
    await page.mouse.move(center.x, center.y);
    await page.mouse.wheel(0, wheelDelta);
    await page.waitForTimeout(settleMs);
  }
  throw new Error(
    `${label} never reached ${minNonBg} non-background color buckets after ${maxSteps} ` +
      `zoom steps (best: ${JSON.stringify(best)}); the die did not render filled geometry`,
  );
}

/**
 * Asserts a captured GIF's sampled frame stats pass the shipping gate: the MEDIAN frame
 * must have at least {@link MIN_NONBG_BUCKETS} non-background buckets (a mostly-starry GIF
 * fails; a few transition or legitimate solid-fill frames are tolerated), and NO frame may
 * be a dead single-color screenshot (>= 1 non-background bucket each). Returns the sorted
 * per-frame non-background counts for logging. Throws on failure.
 *
 * @param {number[]} perFrameNonBg  non-background bucket count for each sampled frame
 * @param {string} name
 * @returns {number[]}
 */
export function assertGifNotStarry(perFrameNonBg, name) {
  if (perFrameNonBg.length === 0) throw new Error(`${name}: no frames to gate`);
  const sorted = [...perFrameNonBg].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  const min = sorted[0];
  if (min < 1) {
    throw new Error(`${name}: a captured frame is blank (0 non-background buckets); [${sorted}]`);
  }
  if (median < MIN_NONBG_BUCKETS) {
    throw new Error(
      `${name}: median frame has ${median} non-background buckets (< ${MIN_NONBG_BUCKETS}); ` +
        `the clip is starry/flat. Distribution: [${sorted}]`,
    );
  }
  return sorted;
}

import { test, expect, type Page, type ConsoleMessage } from "@playwright/test";

// Browser-level proof of the share-link LIVE transport (ADR 0058).
//
// Two contexts against ONE real relay (reticle-server, started by serve-relay.mjs on
// port 3030): context A boots the editor and goes live (publishing its document and
// presence into a room), context B opens the read-only viewer link for that room. The
// spec asserts B's browser viewer transport actually connects and receives A's live
// frames over the socket.
//
// What is proven headlessly here:
//   * both bundles boot as their respective modes (the renderer starts: #overlay hides
//     only after eframe's wgpu backend, WebGPU or its WebGL2 fallback, initializes);
//   * B's read-only viewer WebSocket OPENS to the relay room (`reticle-live: socket
//     open`), i.e. the wasm `web_sys::WebSocket` transport dialed `?mode=view`;
//   * B RECEIVES A's live frames and DECODES them (`reticle-live: first frame ...`),
//     i.e. a real `SyncMessage` frame crossed A -> relay -> B and the viewer routed it;
//   * neither context logs a fatal error.
//
// What is NOT asserted headlessly, and why (honest scope):
//   * pixel-level rendering of A's geometry in B's canvas. Headless Chromium here uses
//     the WebGL2 fallback (no WebGPU adapter), and reading back exact canvas pixels is
//     unreliable; the egui UI is canvas-painted, so there are no DOM nodes for the
//     mirrored shapes to assert on either. The AUTHORITATIVE proof that a viewer
//     materializes the sharer's geometry and presence, and that a viewer's frame is
//     dropped server-side, is the headless Rust relay test
//     crates/reticle-server/tests/share_live.rs. This Playwright spec is the
//     browser-level proof that the wasm transport boots, opens the socket, and carries
//     frames end to end.
//   * A's "Go live" is triggered by the `?share=1` boot flag rather than a click,
//     because that button is painted on the egui canvas and is not a DOM element a
//     headless click can reach. The publish PATH exercised is identical.

const RELAY = "127.0.0.1:3030";
const ROOM = "e2e-share";

/** Console/page text that signals a real failure rather than benign noise. */
function isFatal(text: string): boolean {
  const m = text.toLowerCase();
  return (
    m.includes("panic") ||
    m.includes("unreachable executed") ||
    m.includes("failed to start the reticle web app") ||
    m.includes("is missing a #reticle-canvas")
  );
}

/** Forces the WebGL2 fallback on the webgl2 project (hides WebGPU), matching the
 * other specs so this gates on a headless host too. */
test.beforeEach(async ({ page }, testInfo) => {
  if (testInfo.project.name === "webgl2") {
    await page.addInitScript(() => {
      try {
        delete (Navigator.prototype as { gpu?: unknown }).gpu;
      } catch {
        /* already absent */
      }
    });
  }
});

/** Collects console lines matching `reticle-live:` and any fatal errors from a page. */
function hookConsole(page: Page): { live: string[]; fatals: string[] } {
  const live: string[] = [];
  const fatals: string[] = [];
  const onMsg = (m: ConsoleMessage) => {
    const t = m.text();
    if (t.includes("reticle-live:")) live.push(t);
    if (m.type() === "error" && isFatal(t)) fatals.push(t);
  };
  page.on("console", onMsg);
  page.on("pageerror", (e) => {
    if (isFatal(String(e))) fatals.push(String(e));
  });
  return { live, fatals };
}

test("a viewer streams a live shared session over the relay", async ({ browser }, testInfo) => {
  // Two isolated browser contexts, so A and B are genuinely separate clients.
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const pageA = await ctxA.newPage();
  const pageB = await ctxB.newPage();

  if (testInfo.project.name === "webgl2") {
    for (const p of [pageA, pageB]) {
      await p.addInitScript(() => {
        try {
          delete (Navigator.prototype as { gpu?: unknown }).gpu;
        } catch {
          /* already absent */
        }
      });
    }
  }

  const a = hookConsole(pageA);
  const b = hookConsole(pageB);

  try {
    // A: boot the editor and go live automatically for the room (publisher).
    await pageA.goto(`/?share=1&room=${ROOM}&relay=${encodeURIComponent(RELAY)}`);
    await expect(pageA.locator("#overlay"), "publisher renderer starts").toBeHidden();
    // A's sharer socket should open.
    await expect
      .poll(() => a.live.some((l) => l.includes("socket open")), {
        message: "publisher socket did not open",
        timeout: 30_000,
      })
      .toBe(true);

    // B: open the read-only viewer link for the same room (viewer).
    await pageB.goto(
      `/?view=viewer&room=${ROOM}&relay=${encodeURIComponent(RELAY)}`,
    );
    await expect(pageB.locator("#overlay"), "viewer renderer starts").toBeHidden();

    // B's viewer socket opens to the relay room (`?mode=view`).
    await expect
      .poll(() => b.live.some((l) => l.includes("socket open")), {
        message: "viewer socket did not open",
        timeout: 30_000,
      })
      .toBe(true);

    // B receives and decodes at least one live frame published by A. The relay replays
    // the room log to a late joiner, so B gets A's frames even though it joined second.
    await expect
      .poll(() => b.live.some((l) => l.includes("first frame")), {
        message: `viewer received no frame. viewer live log:\n${b.live.join("\n")}`,
        timeout: 30_000,
      })
      .toBe(true);

    // Record what arrived for the report, and assert no fatal errors on either side.
    const firstFrame = b.live.find((l) => l.includes("first frame")) ?? "(none)";
    testInfo.annotations.push({
      type: "share-live",
      description: `viewer first frame: ${firstFrame}; publisher live: ${a.live.join(
        " | ",
      )}`,
    });
    expect(a.fatals, `publisher fatals:\n${a.fatals.join("\n")}`).toHaveLength(0);
    expect(b.fatals, `viewer fatals:\n${b.fatals.join("\n")}`).toHaveLength(0);
  } finally {
    await ctxA.close();
    await ctxB.close();
  }
});

// --------------------------------------------------------------------------
// Lane v8-1e: an edit paints in the viewer, and a view-mode socket cannot write.
//
// These extend the boot/transport proof above into a behavioral one, reading the
// wasm `window.__reticle_stats` seam (ADR 0058, lane v8-1c): a viewer bumps
// `applied_frames` each time it applies a live frame and sets `applied_shapes` to the
// mirrored top cell's shape count. Pixels stay out of scope (the canvas is GPU-painted
// and pixel readback under the headless WebGL2 fallback is unreliable, as documented
// above); the counter seam is the browser-observable proof that A's geometry reached B.

/** A fresh, sanitized relay room per run, so re-running against a reused relay never
 * replays another run's log into this one (`room_id` keeps `[a-z0-9_-]`). */
function roomName(prefix: string): string {
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

/** The raw relay WebSocket URLs a forged client would dial: edit mode (publishes) and
 * read-only view mode (`?mode=view`, whose sends the relay drops server-side). */
const editWsUrl = (room: string) => `ws://${RELAY}/ws/${room}`;
const viewWsUrl = (room: string) => `ws://${RELAY}/ws/${room}?mode=view`;

/** The viewer's applied-frame / applied-shape counters, 0 if the seam is absent. */
function appliedFrames(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (window as unknown as { __reticle_stats?: { applied_frames?: number } })
        .__reticle_stats?.applied_frames ?? 0,
  );
}
function appliedShapes(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (window as unknown as { __reticle_stats?: { applied_shapes?: number } })
        .__reticle_stats?.applied_shapes ?? 0,
  );
}

/** Boots a `(sharer, viewer)` pair on `room`: the sharer goes live (optionally placing
 * the one scripted rect with `?e2e-edit=1`), the viewer opens the read-only link, and
 * both bundles' sockets are confirmed open with a frame decoded. Contexts are pushed
 * onto `ctxs` for the caller to close. Returns the two pages. */
async function bringUpRoom(
  browser: import("@playwright/test").Browser,
  ctxs: import("@playwright/test").BrowserContext[],
  room: string,
  edit: boolean,
): Promise<{ pageA: Page; pageB: Page; a: ReturnType<typeof hookConsole>; b: ReturnType<typeof hookConsole> }> {
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  ctxs.push(ctxA, ctxB);
  const pageA = await ctxA.newPage();
  const pageB = await ctxB.newPage();
  const a = hookConsole(pageA);
  const b = hookConsole(pageB);
  const relay = encodeURIComponent(RELAY);

  const shareFlags = edit ? "share=1&e2e-edit=1" : "share=1";
  await pageA.goto(`/?${shareFlags}&room=${room}&relay=${relay}`);
  await expect(pageA.locator("#overlay"), "publisher renderer starts").toBeHidden();
  await expect
    .poll(() => a.live.some((l) => l.includes("socket open")), {
      message: "publisher socket did not open",
      timeout: 30_000,
    })
    .toBe(true);

  await pageB.goto(`/?view=viewer&room=${room}&relay=${relay}`);
  await expect(pageB.locator("#overlay"), "viewer renderer starts").toBeHidden();
  await expect
    .poll(() => b.live.some((l) => l.includes("socket open")), {
      message: "viewer socket did not open",
      timeout: 30_000,
    })
    .toBe(true);
  await expect
    .poll(() => b.live.some((l) => l.includes("first frame")), {
      message: `viewer received no frame. viewer live log:\n${b.live.join("\n")}`,
      timeout: 30_000,
    })
    .toBe(true);
  return { pageA, pageB, a, b };
}

/** Waits until the viewer's `applied_shapes` is positive and has stopped changing (the
 * sharer's full state has been merged), then returns that settled count. */
async function settledShapes(page: Page): Promise<number> {
  await expect
    .poll(() => appliedShapes(page), {
      message: "viewer never applied any of the sharer's shapes",
      timeout: 30_000,
    })
    .toBeGreaterThan(0);
  let prev = -1;
  for (let i = 0; i < 25; i++) {
    const cur = await appliedShapes(page);
    if (cur === prev) return cur;
    prev = cur;
    await page.waitForTimeout(300);
  }
  return prev;
}

/** Brings up one `(sharer, viewer)` room, returns the viewer's settled shape count, and
 * tears the room's two contexts down before returning. Rooms are run one at a time (not
 * held open together) so at most two heavy WebGL2 bundles boot at once, matching the load
 * the other specs prove is stable on the headless host. */
async function roomViewerShapes(
  browser: import("@playwright/test").Browser,
  room: string,
  edit: boolean,
): Promise<number> {
  const ctxs: import("@playwright/test").BrowserContext[] = [];
  try {
    const { pageB, a, b } = await bringUpRoom(browser, ctxs, room, edit);
    const shapes = await settledShapes(pageB);
    expect(a.fatals, `publisher fatals:\n${a.fatals.join("\n")}`).toHaveLength(0);
    expect(b.fatals, `viewer fatals:\n${b.fatals.join("\n")}`).toHaveLength(0);
    return shapes;
  } finally {
    for (const c of ctxs) await c.close();
  }
}

test("an edit made in the sharer paints in a read-only viewer", async ({ browser }) => {
  // Two rooms identical EXCEPT for the scripted edit: the sharer with `?e2e-edit=1`
  // places one rect after going live, the control sharer places nothing. Comparing the
  // two viewers' shape counts isolates the edit itself, with no dependency on the demo
  // document's own shape count (both rooms mirror the same demo). The delta being
  // exactly one is the edit reaching the viewer. The rooms run sequentially so the host
  // never boots more than two bundles at once.
  const editedShapes = await roomViewerShapes(browser, roomName("paint-edit"), true);
  const controlShapes = await roomViewerShapes(browser, roomName("paint-ctrl"), false);

  expect(
    editedShapes,
    "the viewer mirrors the sharer's geometry including the scripted rect",
  ).toBeGreaterThan(0);
  expect(
    editedShapes - controlShapes,
    `the scripted edit adds exactly one shape the control viewer never sees ` +
      `(edited=${editedShapes}, control=${controlShapes})`,
  ).toBe(1);
});

test("a view-mode socket cannot write to the shared session, but an edit-mode socket can", async ({
  browser,
}) => {
  // The relay drops binary frames sent by a `?mode=view` connection server-side (ADR
  // 0038); this proves the BROWSER side of that contract, and, with a positive control
  // that the very same bytes DO apply when sent edit-mode, that the drop assertion is
  // not vacuous. Same bytes, mode the only difference: view-mode is dropped, edit-mode
  // is delivered.
  const room = roomName("write-guard");
  const ctxs: import("@playwright/test").BrowserContext[] = [];
  try {
    const { pageB: viewer, b } = await bringUpRoom(browser, ctxs, room, true);

    // Post-replay baseline: the viewer has applied the sharer's state.
    await expect
      .poll(() => appliedFrames(viewer), {
        message: "viewer never applied the sharer's initial state",
        timeout: 30_000,
      })
      .toBeGreaterThan(0);

    // Capture a REAL frame the relay holds in this room's log by joining read-only and
    // grabbing the first `Update` it replays (SyncMessage.payload = Update is proto
    // field 1, so the frame's first byte is 0x0A; presence/comment frames are skipped).
    // Using the sharer's own frame means the bytes are known-valid and decodable, so the
    // positive control below proves an accepted injection genuinely applies, not that
    // our forged bytes were merely junk the viewer would ignore anyway.
    const frame: number[] = await viewer.evaluate(
      (url) =>
        new Promise<number[]>((resolve, reject) => {
          const ws = new WebSocket(url);
          ws.binaryType = "arraybuffer";
          const timer = setTimeout(() => {
            try {
              ws.close();
            } catch {
              /* already closed */
            }
            reject(new Error("no update frame replayed within timeout"));
          }, 20_000);
          ws.onmessage = (e: MessageEvent) => {
            if (e.data instanceof ArrayBuffer && e.data.byteLength > 0) {
              const u8 = new Uint8Array(e.data);
              if (u8[0] === 0x0a) {
                clearTimeout(timer);
                const bytes = Array.from(u8);
                try {
                  ws.close();
                } catch {
                  /* already closed */
                }
                resolve(bytes);
              }
            }
          };
          ws.onerror = () => {
            clearTimeout(timer);
            reject(new Error("listener socket errored"));
          };
        }),
      viewWsUrl(room),
    );
    expect(frame.length, "captured a non-empty Update frame from the relay").toBeGreaterThan(0);

    // Helper: open a raw socket in the page and keep it on `window.__forge[key]`.
    const openForge = (key: string, url: string) =>
      viewer.evaluate(
        ({ key, url }) =>
          new Promise<boolean>((resolve, reject) => {
            const w = window as unknown as { __forge?: Record<string, WebSocket> };
            w.__forge = w.__forge || {};
            const ws = new WebSocket(url);
            ws.binaryType = "arraybuffer";
            w.__forge[key] = ws;
            const timer = setTimeout(() => reject(new Error("open timeout")), 10_000);
            ws.onopen = () => {
              clearTimeout(timer);
              resolve(true);
            };
            ws.onerror = () => {
              clearTimeout(timer);
              reject(new Error("forge socket errored"));
            };
          }),
        { key, url },
      );
    const forgeSend = (key: string, bytes: number[]) =>
      viewer.evaluate(
        ({ key, bytes }) => {
          const w = window as unknown as { __forge?: Record<string, WebSocket> };
          const ws = w.__forge?.[key];
          if (!ws || ws.readyState !== 1) throw new Error("forge socket not open");
          ws.send(new Uint8Array(bytes));
          return true;
        },
        { key, bytes },
      );
    const forgeOpen = (key: string) =>
      viewer.evaluate((key) => {
        const w = window as unknown as { __forge?: Record<string, WebSocket> };
        return w.__forge?.[key]?.readyState === 1;
      }, key);

    // NEGATIVE: send that exact frame from a view-mode socket. The relay drops it, so the
    // viewer's applied-frame counter does not move over a generous settle window.
    const framesBeforeView = await appliedFrames(viewer);
    const liveClosesBefore = b.live.filter((l) => l.includes("socket close")).length;
    await openForge("view", viewWsUrl(room));
    await forgeSend("view", frame);
    await viewer.waitForTimeout(3_000);
    const framesAfterView = await appliedFrames(viewer);
    expect(
      framesAfterView,
      "a view-mode forged frame is dropped server-side; the viewer applies nothing",
    ).toBe(framesBeforeView);
    // The forging socket is not killed, and the viewer's own session stays live.
    expect(await forgeOpen("view"), "the view-mode socket stays open, just muted").toBe(true);
    expect(
      b.live.filter((l) => l.includes("socket close")).length,
      "the viewer's live session is not torn down by the forged send",
    ).toBe(liveClosesBefore);

    // POSITIVE control: the same bytes over an edit-mode socket ARE broadcast, and the
    // viewer applies them (its CRDT merge is idempotent, so `applied_shapes` need not
    // change, but `applied_frames` bumps once per applied frame, the delivery proof).
    const framesBeforeEdit = await appliedFrames(viewer);
    await openForge("edit", editWsUrl(room));
    await forgeSend("edit", frame);
    await expect
      .poll(() => appliedFrames(viewer), {
        message: "an edit-mode injection of the same bytes should reach and apply in the viewer",
        timeout: 15_000,
      })
      .toBeGreaterThan(framesBeforeEdit);

    expect(b.fatals, `viewer fatals:\n${b.fatals.join("\n")}`).toHaveLength(0);
  } finally {
    for (const c of ctxs) await c.close();
  }
});

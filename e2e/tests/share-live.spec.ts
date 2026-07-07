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

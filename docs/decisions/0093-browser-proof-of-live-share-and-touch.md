# 0093, Browser-proof of live share and touch: what a counter seam can honestly assert, and what stays the Rust test's job

## Context

ADR 0058 shipped the wasm live-share transport with a two-context Playwright spec, but
that spec deliberately stopped at *boot and plumbing*: it proved the viewer bundle boots,
its `?mode=view` socket opens, and `SyncMessage` frames arrive and decode. It asserted
nothing about **behavior**: that an edit made in the sharer actually reaches a viewer,
that a viewer's browser genuinely cannot write to the shared session, or that a phone
navigates a design by touch. Those were left to the headless Rust relay test
(`crates/reticle-server/tests/share_live.rs`) and to unit tests. The README's share
section, lacking a real capture of the two-context flow, reused the browse GIF under an
explicit honesty note.

The obstacle is the same one ADR 0058 named: the egui canvas is GPU-painted, so there is
no DOM node for a mirrored shape or the camera, and pixel readback under the headless
WebGL2 fallback is unreliable. Lane v8-1c answered this for the transport by exposing a
wasm-only counter seam, `window.__reticle_stats` (`applied_frames`, `applied_shapes`),
incremented when a viewer applies a live frame, plus a `?share=1&e2e-edit=1` boot flag
that makes the sharer place one scripted rect. This ADR records how lane v8-1e turns that
seam into browser-verified *behavioral* proof, and where the honest line stays.

## Decision

**1. An edit paints in a viewer, proven by an isolated counter delta, not by pixels.**
The spec boots two rooms identical except for the scripted edit: one sharer with
`?e2e-edit=1` (which places one rect), one control sharer with none. Both viewers mirror
the same demo document, so the difference in their settled `applied_shapes` is exactly the
edit, asserted to be `+1`, with no dependency on the demo document's own shape count.
Pixels stay out of scope; the counter seam is the browser-observable proof that the
sharer's geometry crossed the relay into the viewer's mirrored document.

**2. A view-mode socket cannot write, proven with a non-vacuous positive control.** The
relay drops binary frames from a `?mode=view` connection server-side (ADR 0038); this lane
proves the *browser* side of that contract without re-proving the Rust side. The spec
captures a real `Update` frame the relay holds in the room log (by joining read-only and
grabbing the first frame whose leading byte is `0x0A`, the `SyncMessage.Update` proto
tag), then sends **those same bytes** two ways: from a `?mode=view` socket (the viewer's
`applied_frames` must not move over a settle window, and neither the forging socket nor the
viewer's session is torn down) and from an edit-mode socket (the viewer's `applied_frames`
must increase). Same bytes, mode the only variable, so the drop assertion cannot pass
vacuously on bytes the viewer would have ignored anyway. The Rust relay test remains the
authority on the server-side drop and the read-only log invariant.

**3. Touch navigation, proven by a camera-readout seam on a phone viewport.** A new
Playwright `phone` project (a Pixel 7 device descriptor, `hasTouch`) opens an example
document via an intercepted `?gds=` fetch and synthesizes a two-finger pinch and drag with
CDP `Input.dispatchTouchEvent`, which egui aggregates into the same multi-touch gesture a
phone produces. To observe the result, the wasm build publishes the live view camera into
`window.__reticle_stats.camera` (`{ center_x, center_y, pixels_per_dbu }`), written every
editor frame; the spec reads a baseline and asserts the pinch changed the zoom and the drag
changed the center. The pinch/pan math itself stays unit-tested window-free in
`crates/reticle-app/src/camera.rs`; the seam is only the browser-observable proxy that the
gesture reached that math.

**4. A real two-context share GIF.** `e2e/capture-share.mjs` (run by `just capture-share`,
not part of the test run) drives a headed Chromium window holding the editor and the
read-only viewer side by side as two real iframes whose only channel is the relay, animates
the sharer's cursor so the viewer shows the remote presence live, and assembles
`assets/tour-share.gif` with gifski under the repo's 6 MB tour-GIF budget. The README share
section now references it, and the honesty note that the section reused the browse GIF is
deleted.

**Where the camera seam lives, and why it is the one cross-boundary touch.** The applied-
frame counters are written from the viewer's frame-apply path; the camera is private to the
editor `App` and has no per-frame hook reachable from `crates/web`. So the camera readout is
a wasm-only method on `App` written each editor frame, mirroring v8-1c's counters and
extending the *same* `__reticle_stats` object. It only ever writes (never reads app state
back into rendering), so it cannot perturb what it measures, and it is compiled out of
native builds.

## Consequences

* `just e2e-share` now proves behavior, not just plumbing: the edit-paints-in-B delta and
  the view-mode-cannot-write pair (with its positive control) run alongside the original
  boot/transport assertions.
* A new `phone` project proves touch pan-zoom; it needs no relay and runs against the same
  bundle server as `webgl2`.
* The honest scope is unchanged in spirit from ADR 0058 and stated in every new spec: no
  pixel-level assertion. The counter and camera seams are the browser-observable proxies;
  the Rust relay test is the authority on the read-only contract, and `camera.rs` on the
  transform. The seams are wasm-only instrumentation, absent from native builds.
* Flake control, given the shared relay: `workers=1`, fixed ports, and a random room suffix
  per run so a reused relay never replays one run's log (or a forged frame) into the next.

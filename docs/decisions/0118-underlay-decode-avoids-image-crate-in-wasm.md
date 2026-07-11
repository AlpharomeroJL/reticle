# 0118, the image underlay decodes through the browser's own codec on wasm32, not the `image` crate

## Context

The underlay lane (Phase 4, Reach) adds a positioned, scaled,
opacity-controlled raster backdrop (a die photo or a datasheet figure)
rendered under the layout, for tracing. The brief's default-safe plan
(mirroring the native-only rhai precedent, ADR 0115) was: PNG decode
unconditional (native and wasm32, via the `image` crate, already a dev-only
dependency of `reticle-render` for its golden-image tests), JPEG decode
native-only, measured with `just bundle-gate` before deciding whether JPEG
could also ship in the browser.

Headroom at dispatch was tight: Phase 3 closed at +433.0 KiB gz over the
v8.0-baseline (ADR 0115), leaving roughly +17 KiB before the +450 KiB budget,
of which this lane's brief allotted about +10.6 KiB (the rest assumed spent by
other concurrent Phase 4 lanes in their own worktrees).

A first measurement, promoting `image` from a dev-only dependency to a real
one with `default-features = false, features = ["png"]` (no JPEG at all),
came back:

```
bundle-size: FAIL, gz total 4504242 exceeds v8.0-baseline 3999044 by +493.4 KiB (budget +450 KiB).
```

+493.4 KiB, already 43.4 KiB over the absolute ceiling, before JPEG was even
considered. Inspecting `image` 0.25.10's own manifest explained why:
`[dependencies.moxcms]` (a color-management / ICC-profile library) is an
**unconditional** dependency of the `image` crate itself, version 0.8.0, with
no `optional = true` and gated behind no feature flag. Enabling `image` at
all, for any format, pulls in `moxcms` and its own transitive graph
regardless of which decoder feature is selected. Since `image` had only ever
been a dev-dependency before (compiled for tests, never shipped in the actual
wasm bundle), this was the first time any of it reached the shipped browser
build, and the unconditional color-management stack dominated the addition.

## Decision

The wasm32 build does not depend on the `image` crate at all. It decodes
underlay images through the **browser's own image codec** instead:

1. Wrap the picked bytes in a `web_sys::Blob`.
2. Decode with `window.createImageBitmap(blob)` (the same decoder path the
   browser already uses for `<img>` tags and CSS backgrounds; it accepts
   PNG, JPEG, and anything else the browser supports natively, with no
   Rust-side format-specific code at all).
3. Draw the resulting `ImageBitmap` onto a detached (never DOM-attached)
   `<canvas>` element.
4. Read the pixels back with `getImageData`, giving the same tightly packed
   straight-alpha RGBA8 bytes the native `image` crate's `to_rgba8()` already
   produced.

`createImageBitmap` is promise-based, so this whole decode is async
(`reticle_app::underlay::decode_via_browser`); it runs inside the same async
picker task that already reads the file bytes (`rfd::AsyncFileDialog`), and
posts the finished `DecodedImage` into the existing pending-pick mailbox
(mirroring `crate::webopen::WebOpenInbox`) for the next frame's update to
adopt. Native keeps the straightforward synchronous path: `rfd::FileDialog`
reads the file, `reticle_render::decode_underlay_image` (the `image` crate,
`png` and `jpeg` features, now a native-only dependency) decodes it inline.

`reticle-render`'s Cargo.toml moved `image` to
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` with both `png`
and `jpeg` features; the wasm32 dependency graph carries none of it.
`reticle-app`'s wasm32 `web-sys` feature list gained `Document`, `Element`,
`HtmlCanvasElement`, `CanvasRenderingContext2d`, `ImageBitmap`, and
`ImageData` (all already in the lockfile via `eframe`/the `web` crate, so this
added no new dependency, only feature flags on the existing one).

The untrusted-input discipline applies on both routes: the wasm32 path caps
the encoded read (`decode_via_browser`'s own `MAX_ENCODED_BYTES`, 64 MiB, the
same value the native decode uses) and the claimed decoded pixel count
(`MAX_DECODED_PIXELS`, 64 megapixels, checked against `ImageBitmap`'s own
`width()`/`height()` before the `getImageData` readback allocates the full
buffer) before ever reaching that allocation. There is no Rust-side byte
parsing of the image format on the wasm32 path at all (the browser's own
decoder does that, off any Rust-controlled panic surface): a malformed image
there just rejects the `createImageBitmap` promise, surfaced as an ordinary
`Err(String)`.

## Measurement

Re-measured after the pivot:

```
bundle-size: PASS, gz total 4455780 vs v8.0-baseline 3999044: +446.0 KiB (budget +450 KiB).
```

+446.0 KiB, PASS, with 4.0 KiB of headroom left under the absolute +450 KiB
ceiling (this lane's own marginal addition over the +433.0 KiB Phase-3
closing point is about +13.0 KiB: the underlay state, the Inspector panel,
the texture-cache glue, and the wasm32 decode/canvas plumbing above; none of
it is a Rust image decoder). This is markedly better than the brief's
fallback plan would have measured (PNG-only via the `image` crate alone was
already 43.4 KiB over budget) and, as a side effect, gives full PNG and JPEG
parity in the browser rather than the anticipated native-only JPEG
disclaimer: the browser was never going to be worse than native here, since
it reuses whatever codecs the browser ships.

## Consequences

- The browser underlay supports PNG and JPEG equally with native, with no
  format gap and no disclaimer needed; the earlier
  `UnderlayImageError::JpegNativeOnly` shape was designed but never shipped
  once this path was found, and was removed.
- Headroom after this lane is thin: +4.0 KiB under the absolute ceiling in
  this lane's own worktree measurement. The integration gate must re-measure
  the merged Phase 4 tree; other concurrent lanes' additions are not
  reflected here (lanes are isolated worktrees measured independently until
  merge).
- PRECEDENT: a format-decode dependency that is meaningfully sized in wasm
  (here, `image`'s unconditional `moxcms` dependency, not even the format
  codec itself) is worth checking for a browser-native equivalent before
  assuming "native-only" is the only alternative to a bundle breach. Browsers
  already ship image codecs; `createImageBitmap` plus a detached-canvas
  readback is a general pattern for any future raster-decode need that would
  otherwise add a Rust codec to the wasm graph.
- If the `image` crate's `moxcms` dependency becomes optional in a future
  release, or a lighter decode-only crate replaces it, the wasm32 path could
  revisit shipping a Rust decoder; that is a measured decision for whoever
  next touches this, not assumed here.

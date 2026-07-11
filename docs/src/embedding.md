# Embedding Reticle

The browser bundle can run inside another page's `<iframe>`. `?embed=1` (catalog 94)
hides every menu, panel, and dialog and leaves only the canvas, so the frame shows
just the layout. It is the same wasm bundle the full app ships: embedding changes
only the chrome, never the renderer or the import/streaming data path.

## The iframe snippet

```html
<iframe
  src="https://alpharomerojl.github.io/reticle/?embed=1&archive=https://example.com/chip.rtla"
  width="960"
  height="600"
  style="border: 0"
  loading="lazy"
  title="Reticle layout viewer"
></iframe>
```

The canvas fills whatever box the `<iframe>` is given (`width`/`height` on the
element, or CSS); there is no minimum size.

## Pointing the frame at a design

`?embed=1` alone shows the bundle's built-in default document, a static canvas with
nothing loaded. The published bundle's own public default view is the replay
theater (ADR 0026), but the theater is a separate docked panel that never renders in
embed's minimal chrome, so a bare `?embed=1` link is not useful on its own. Add one of
these to load real content:

- `&archive=<url>` streams a served `.rtla` archive read-only over HTTP Range
  requests, without importing the whole file (see [Streamed documents](streaming.md)
  and [Archive hosting](archive-hosting.md)). Best for a large die.
- `&gds=<url>` fetches a `.gds`/`.oas` file from a URL into an editable document.
  Embed's hidden menus and toolbar make it read-only in practice, since there is
  nothing to invoke an edit from. Best for a small design.

Either composes with the existing permalink seam (see
[Permalinks](collaboration.md#permalinks)), unaffected by embed mode:

- `&cell=<name>` focuses a cell.
- `&view=<x>,<y>,<zoom>` sets the initial camera: the world point at the canvas
  center and the zoom in pixels per DBU.
- `&layers=<csv>` shows only the listed `layer/datatype` pairs (for example
  `68/20,69/20`), hiding every other layer.

A full example, a streamed archive framed on one cell at a fixed zoom with two
layers visible:

```text
https://alpharomerojl.github.io/reticle/?embed=1&archive=https://example.com/chip.rtla&cell=top&view=0,0,4.0&layers=68/20,69/20
```

## Minimal chrome

`?embed=1` suppresses, unconditionally:

- the Start screen (the worked-use-case chooser), even on a first-time visit;
- the menu bar, toolbar, and every docked panel (Layers, Inspector): the canvas
  layout selection (`App::chrome_layout`) picks the embed layout ahead of the full
  editor, presentation mode, and the read-only viewer, so none of their chrome can
  render while embedded, whatever else is also requested;
- the command palette, floating windows, and the guided-tour overlay.

What remains is the canvas, plus a small "Open in Reticle" link in the bottom corner
that reopens the same URL in a new tab with `embed=1` turned off, so a visitor can
always reach the full app. Keyboard shortcuts still fire in embed mode; there is no
overlay to discover them from, but nothing about embed disables input handling.

This is asserted directly, not just eyeballed: `cargo test -p reticle-app embed`
runs headless tests (no GPU, no window; plain app state) that check the exact gates
`App::ui` reads, including that embed wins the chrome layout even when presentation
mode or a read-only viewer session is requested at the same time.

## Previewing embed chrome without an iframe

The `embed.toggle` command (palette-only, no default chord) flips embed mode on and
off inside the full app, so the minimal chrome can be previewed without standing up
an iframe. It sets the same flag `?embed=1` does; toggling it again, or the corner
link, returns the full chrome.

## Cross-origin requirements

The embedding page, the Reticle host, and a served design can all be different
origins. Two things have to allow it:

- **Framing.** The Reticle host must not send `X-Frame-Options: DENY` or a
  restrictive `Content-Security-Policy: frame-ancestors`, or the browser refuses to
  render the frame. The published demo sends neither header: checked 2026-07-11 with
  `curl -sD - -o /dev/null https://alpharomerojl.github.io/reticle/`; re-run that
  command against the live deploy before relying on it, since GitHub Pages' response
  headers are outside this repository's control.
- **The design fetch.** A design loaded over `&archive=` or `&gds=` from a third
  origin must itself answer with a permissive `Access-Control-Allow-Origin` and, for
  `&archive=`, allow `Range` requests (see [Archive hosting](archive-hosting.md)).
  This is the same requirement the non-embedded browse already has; embed mode adds
  no new cross-origin surface.

## What embed mode does not do yet

Embed mode is chrome-only today: there is no `postMessage` host API to script the
frame from the embedding page, and the frame posts no resize or scroll events back
out. Both are natural follow-ons, not yet built.

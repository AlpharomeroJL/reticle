# 0067. Permalinks reuse `?view=`, disambiguated by shape

## Context

A shareable permalink needs to carry a camera (center and zoom) on top of the
existing `?gds=<url>` open, alongside a focus `?cell=` and a `?layers=` visibility
set. The natural query key for "where the camera is looking" is `view`, but `?view=`
is already taken: since ADR 0026 the web bundle reads `?view=viewer|editor|replay`
to pick a start view, and ADR 0038/0058 layer `?view=viewer` for the read-only
viewer link. Introducing a second camera-only key (say `?camera=`) would work but
leaves two view-ish parameters a user has to keep straight, and a camera is
conceptually "the view", so overloading `view` reads better in a shared URL.

The two uses never need to coexist in one link: a start-view keyword selects *which
app* boots, and a camera spec positions the canvas *after* a document opens. So the
ambiguity is only syntactic.

## Decision

`share::parse_permalink` disambiguates `?view=` **by the shape of its value**, not by
a second key:

* If the value parses as exactly three comma-separated finite `f64`s
  (`view=<x>,<y>,<zoom>`), it is a **camera spec** and fills `Permalink::camera`.
* Otherwise (a `viewer`/`editor`/`replay` keyword, or anything malformed) it is left
  for the start-view selector (`StartView::from_query_value`) and `Permalink::camera`
  stays `None`.

`emit_permalink` always writes the camera in the three-float form, so a permalink
this crate emits is unambiguous. A camera-shaped `?view=` is inert to the boot's
start-view selector (it is not a known keyword, so it falls back to the default start
view), and a keyword `?view=` is inert to the permalink camera parser: the two
readers ignore each other's values. The full permalink parse is total and lenient:
a bad float, an unknown or out-of-range layer, or an empty value is ignored, never a
panic, so a hand-edited link degrades gracefully instead of failing the open.

Permalink values (the `?cell=` name and the `?gds=` URL) use a full RFC 3986
component percent-encoding (`share::encode_permalink_value`), distinct from the
existing `encode_query_component`, which only escapes the handful of reserved
characters a relay host can contain. A cell name may hold spaces, `/`, `,`, or
non-ASCII text, so it is encoded and decoded byte-wise through UTF-8 to round-trip
exactly (`emit -> parse` is the identity on the `Permalink` fields).

## Consequences

* One view-ish key instead of two; a shared URL reads `?gds=...&cell=...&view=x,y,z&layers=...`.
* The disambiguation is a pure, testable rule: unit tests pin both readings
  (`view=1000,-500,0.25` is a camera; `view=editor` is not) and the round-trip and
  encoding edge cases (spaces, unicode, empty layer list).
* A future third meaning for `view=` would have to stay shape-distinct from three
  floats; that is a real constraint, accepted because a camera is the only other
  "view" a permalink needs and the keyword set is closed.
* The camera is stored as `(f64, f64, f64)` and applied as
  `ViewCamera::new(Point, zoom)`; the world coordinates round-trip through the
  camera's DBU integer center, which is exact for the values a session produces.

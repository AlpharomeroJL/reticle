# 0103, F3: the trace-query contract carries a revision envelope

## Context

The v8.2 campaign adds trace UI (Phase 2): click-to-trace a net, show a net's extent, and a
navigable shorts/opens list. The trace-api lane provides these as read-only spatial queries
over an already-extracted netlist, cached per document revision so a repeated query on an
unchanged document is free. The trace UI must build against a frozen record shape before the
query layer exists, and the cache and the UI must agree on how a result is keyed and known
to be stale.

`reticle-extract` already has the internal `Net`, `Short`, and `Open` types the extractor
works with, but those are not the query RESULTS a UI renders, and `reticle_geometry::Rect`
is not serde-serializable.

## Decision

F3 lives in `reticle-extract` (`query.rs`) as serializable result records, each carrying a
`revision: u64` envelope. `NetAtPoint { revision, net: Option<NetRef> }` answers
click-to-trace; `NetExtent { revision, net, bbox, shape_count }` answers a net's extent; and
`ShortsOpensReport { revision, shorts: Vec<ShortRecord>, opens: Vec<OpenRecord> }` is the
navigable list, with `is_clean` / `len` helpers. Rectangles are a standalone
`RectRecord { min_x, min_y, max_x, max_y }` of `i64` DBU, so every record is serde
round-trippable and byte-stable.

The `revision` is the cache key: a result is computed against a document revision, and the
UI knows a result (and the whole query snapshot that shares the revision) is stale when the
document's revision moves past it. This matches `reticle-model`'s monotonic
`EditableDocument::revision`, which the trace-api lane caches on.

The fixture is `crates/reticle-extract/tests/fixtures/contracts/f3_trace.json` (canned
net-at-point, net-extent, and shorts/opens responses at one revision); the cross-test
(`tests/f3_query.rs`) parses each record, checks the shared-revision invariant, and
round-trips them.

## Consequences

The trace UI (net highlight overlay, click-to-trace, the DRC-list-pattern shorts/opens
navigator) builds entirely against the fixture before the query layer lands. Because the
records are byte-stable and revision-keyed, the trace-api lane can cache them per document
revision and the UI can detect staleness without re-querying. The extractor's internal
types are unchanged: F3 is a new sibling record layer, and any change to the extraction
internals it summarizes is a separate decision (by ADR, with cause), as the plan requires.

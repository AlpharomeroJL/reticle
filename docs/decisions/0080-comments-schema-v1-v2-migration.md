# 0080, Anchored comments and the schema V1 to V2 migration contract

## Context

The `reticle.proto` schema is a frozen Wave 0 contract, versioned by a
`SchemaVersion` enum so a reader can pick a migration path. Until now only
`SCHEMA_VERSION_V1` existed, so nothing exercised the migration story: no document
had ever been read by a build newer than the one that wrote it.

Adding anchored comments to the persisted document is the first real schema
evolution. The risk is not the new feature, it is regression: a schema edit that
silently breaks every pre-existing document. The whole point of a version field is
that old bytes still load, and the only honest proof of that is to read *actual*
old bytes with the *new* code, not to reason about it.

## Decision

**Capture a frozen V1 golden fixture BEFORE editing the schema, then prove the V2
build still reads it.** The ordering is the load-bearing decision.

1. **The golden fixture is committed first, from the pre-V2 build.** A
   representative `Document` (two layers, a rule, rect/polygon/path shapes, an
   instance, an array) is serialized under `SCHEMA_VERSION_V1` to
   `crates/reticle-proto/tests/fixtures/v1_document_golden.bin` and committed
   *before* any schema change. `examples/gen_v1_fixture.rs` records its provenance
   and regenerates it deterministically. Had the schema changed first, the fixture
   would be a V2 artifact and prove nothing.

2. **V2 is purely additive.** `SCHEMA_VERSION_V2 = 2` is added to the enum, and a
   `repeated Comment comments = 5` field is appended to `Document` with a fresh
   field number. Because protobuf skips absent fields and never reuses numbers,
   pre-V2 bytes decode unchanged (comments default-empty), and the V1 golden
   fixture regenerates byte-for-byte under the V2 build (an empty repeated field
   emits nothing).

3. **`migrate_document` upgrades V1 to V2 losslessly.** It stamps
   `schema_version` to V2 and leaves the already-empty `comments` list untouched;
   technology, cells, and top cells are never read or moved. An unspecified (0) or
   future version is refused with a `MigrationError`. The migration is idempotent.

4. **The migration test is the review target.** It decodes the committed pre-V2
   fixture with the V2 code, asserts `comments` is empty and the geometry is
   intact, migrates, and asserts the version becomes V2 while the geometry is
   **byte-for-byte identical** (comparing a canonical re-encoding of just the
   technology/cells/top-cells projection before and after). This proves a genuine
   pre-V2 document still loads without loss.

**Comments reuse the existing `reticle_sync::Comment`.** Its `anchor_ref` binds a
comment to a cell (or a `cell/element-id` path). `to_proto_comments` /
`from_proto_comments` carry a live comment set into `Document.comments` and back; a
two-way test round-trips a V2 document (root + reply thread) through encode/decode
and confirms the comments, their thread structure, and their anchor bindings all
survive, and that a migrated V1 document lists zero.

**The app consumes it through a new egui-free `comment_pins` module** (mirroring
the `drc_panel`/`app` split) plus a minimal additive mount: a "Comments" side
panel to add a comment on the top cell, list, and select, and a numbered pin
painted at each anchor's cell-geometry centre on the canvas.

## Consequences

- Every pre-V2 document remains readable forever, pinned by a real committed
  artifact rather than an argument. The fixture is a permanent regression guard:
  any future schema change that breaks V1 decoding fails this test.
- The `SCHEMA_VERSION` constant is now 2; `migrate::is_supported` accepts 1 and 2.
  The next schema bump adds `SCHEMA_VERSION_V3`, a new additive field, and extends
  `migrate_document` to stamp forward, with its own golden fixture captured first.
- Comments are additive metadata on the document; a reader that ignores the field
  still sees identical geometry, so the change is safe for older tooling.
- **App persistence gap (honest).** The app holds comments in-memory in
  `CommentPins`; the proven `Document.comments` persistence path is exercised by
  the `reticle-sync` two-way test, but the app does not yet serialize its live pins
  into a saved V2 document or reload them on open. Wiring `CommentPins` to the
  document save/load path (and into the CRDT for live collaboration) is open work.
- Pins anchor to a cell's geometry centre, not to an individual shape or a free
  point; a per-shape or point anchor is a straightforward extension of
  `anchor_point`.

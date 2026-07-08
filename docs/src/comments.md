# Comments and annotations

A comment is a note anchored to a piece of a layout: a reviewer marks a shape or a
cell and leaves text next to it, so a review conversation lives on the geometry
rather than in a separate document. Comments persist inside the layout document
through a schema version bump, V1 to V2, with a proven-lossless migration of every
pre-V2 document (see [ADR 0080](../decisions/0080-comments-schema-v1-v2-migration.md)).

## The comment model

A comment (`reticle_sync::Comment`) carries a stable `id`, the `thread_id` it
belongs to, an `anchor_ref` binding it to a shape or cell (a cell name, or a
`cell/element-id` path), an `author`, the `body` text, a creation timestamp, and an
`in_reply_to` pointing at the comment it replies to (empty for a thread root).
Replies inherit their parent's thread and anchor, so a `CommentThread` groups a root
with its replies in creation order for display.

## Persistence: schema V1 to V2

Comments are stored on the document itself. The `reticle.proto` schema is versioned
by a `SchemaVersion` enum, and adding comments is the first real evolution of that
schema, so it is also the first exercise of the migration contract.

The change is deliberately **additive**: `SCHEMA_VERSION_V2 = 2` is added to the
enum, and a `repeated Comment comments` field is appended to `Document` with a new
field number. Protobuf never reuses field numbers and skips absent fields, so a
document written under V1 still decodes under V2 with an empty comment list, and a
V1 document with no comments re-encodes byte-for-byte unchanged.

`reticle_proto::migrate::migrate_document` upgrades a document to the current
version: for V1 to V2 it stamps the version to V2 and leaves the (empty) comment
list alone. It never touches the technology, cells, or top-cell list, so all
geometry is preserved exactly. An unspecified (version 0) or newer-than-supported
document is refused rather than guessed at, and migrating a document already at the
current version is a no-op.

### Proving the migration is lossless

The migration is proven against a real pre-V2 artifact, not an argument. A
representative V1 document, serialized with the pre-V2 build, is committed as a
frozen **golden fixture** (`v1_document_golden.bin`). It was captured and committed
*before* the schema was edited: had the schema changed first, the fixture would be a
V2 document and prove nothing.

The migration test then decodes that committed fixture with the V2 code, confirms
the comment list is empty and the geometry is intact, runs `migrate_document`, and
asserts the version becomes V2 while the geometry is byte-for-byte identical before
and after. A second, two-way test builds a V2 document carrying a root-and-reply
thread, round-trips it through encode and decode, and confirms the comments, their
thread structure, and their anchor bindings all survive, and that a migrated V1
document lists zero comments.

## Comment pins in the app

The app surfaces comments as **pins** on the canvas and a list in the side panel,
following the same egui-free-logic split the DRC and diff panels use: a
`comment_pins` module holds the comment set, resolves an `anchor_ref` to a world
point (the centre of the anchored cell's geometry), and formats a comment for the
list, all unit-tested without a UI. The "Comments" panel adds a comment on the
current top cell, lists the comments, and selects one; the canvas paints a numbered
pin at each anchor, with the selected pin drawn larger and highlighted.

The in-app pins are held in memory today. Serializing them into a saved V2 document
(and into the collaborative CRDT for live sharing) reuses the same
`Document.comments` field and the `to_proto_comments` / `from_proto_comments`
converters the persistence tests already exercise; that save/load wiring is open
work.

# 0015, OASIS subset extended to paths, instances, and arrays

## Context

ADR 0004 shipped an in-house OASIS subset that encoded only rectangles and polygons.
Paths, instances (placements), and arrays returned a `ModelError::Unsupported` rather
than being written. That was honest but limited: a hierarchical design could not
round-trip through OASIS, only GDSII preserved the full hierarchy.

## Decision

Extend the in-house OASIS subset to encode and decode paths, instances, and arrays,
for both write and read, and remove the Unsupported errors. Bump the format version
from 0x01 to 0x02 (there are no committed on-disk v1 OASIS files; the tests build
documents in memory). New records, all little-endian, alongside the existing
RECTANGLE (0x10) and POLYGON (0x11):

- PATH 0x12: layer u16, datatype u16, width i32, endcap (kind u8 then ext i32; Flat,
  Square, and Round carry ext 0, Custom carries the extension), point count u32, then
  the points.
- PLACEMENT 0x20 (a single instance): cell reference (u16 length then UTF-8), then the
  transform (dx i32, dy i32, orientation u8 over the eight R0 through MirrorX270
  discriminants, magnification as f64 bits in a u64).
- ARRAY 0x21: cell reference, transform, columns u32, rows u32, column pitch i32, row
  pitch i32.

The CELL record now writes an instance count and its placements, then an array count
and its arrays, after the shapes. The reader is the exact inverse with bounds checks,
returning Malformed on any unknown tag, endcap kind, orientation, or truncation.

## Consequences

- Hierarchical designs now round-trip through the OASIS subset, not just GDSII.
- Verified by extended round-trip tests (a path with each endcap, an instance with a
  mirror orientation, an array) and a new 256-case proptest that round-trips random
  documents containing every shape, instance, and array kind, plus a byte-idempotent
  second export.
- This supersedes the coverage note in ADR 0004. The subset is still not the full
  OASIS standard (no CBLOCK compression, no repetition beyond arrays, no strict-mode
  record set); it covers what Reticle emits and round-trips it losslessly.
- Magnification is encoded as f64 bits because the model's Magnification stores a
  private rational. The wire value reconstructs to the same rational for unity and for
  exact n/1000000 values; a magnification that is not expressible in that form is not
  guaranteed to round-trip bit-exactly. Reticle's own geometry uses unit and simple
  rational magnifications, so this is not a limitation in practice, and it is recorded
  here honestly.

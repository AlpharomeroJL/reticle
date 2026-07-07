# File formats

`reticle-io` reads and writes the layout interchange formats and the technology
description.

## GDSII

GDSII is the long-standing binary interchange format for IC layout. Reticle reads
and writes it through `gds21`, the de facto Rust GDSII library (part of the
Layout21 project), mapping its structures, boundaries, paths, and references onto
Reticle cells, shapes, instances, and arrays. Round-trip fidelity is tested: a
document exported to GDSII and re-imported preserves its geometry, layers, and
hierarchy.

## OASIS

OASIS is the newer, compressed successor to GDSII. It has no mature Rust library,
so Reticle implements a focused in-house subset covering the records it emits plus
the common records seen in practice, sufficient to round-trip its own exports and
import typical files. The supported record set and its limits are documented, and
anything unsupported is a clear error rather than silent data loss. See ADR 0004.

## Technology files

A technology file describes the process: the database resolution, the layer table
(numbers, datatypes, names, and display colors), and the design rules. Reticle
parses a simple, readable text format into the model's `Technology`, which then
drives layer display and the [design-rule checker](drc.md).

## Streaming a die: the record reader and the `.rtla` builder

The GDSII importer above reads a whole library into memory under a 256 MiB cap. A
full shuttle die is several gigabytes and the in-browser converter runs in a worker,
so both need to pull one record at a time without ever holding the whole file. Two
pieces make that possible (Wave 2, ADR 0062 and ADR 0063).

`GdsRecordReader<R: Read>` is a forward-only GDSII reader over any byte source. It
hand-rolls the record framing (`2-byte length, record type, data type, payload`) with
no `gds21` dependency, so it is wasm-clean, and it yields a small flat vocabulary of
`GdsEvent`s (library and struct boundaries, boundaries, paths, references, arrays,
text) in document order. It carries the same hardening as the DOM importer: a
zero-length string record is rejected before anyone can index `data[-1]`, dates are
skipped rather than parsed (so the out-of-range-date panic class cannot fire), and a
record length is a 16-bit field, so no count ever drives an allocation past the
remaining input. A differential test asserts the streaming reader accepts everything
the DOM importer accepts and reports the same cells and per-layer shape counts across
the real corpora; a fuzz target (`gds_stream`) drives it over arbitrary bytes.

`build_rtla` writes a `.rtla` streamed archive from a lazy record source using bounded
memory. It is external and two-pass: pass 1 streams the records and spills them to
sorted run files on disk; pass 2 merges the runs and emits the tiles in directory
order, holding at most one sort chunk and one tile in memory. The finest pyramid level
is exact (every record reaches it and round-trips); coarser levels are subsampled
paint-only approximations. On the 30M-entry generated layout the build peaks at
127 MiB of RSS (far under a 2 GiB budget), and a 120M-record build produces a 2.42 GB
archive to completion under the same bound. The on-disk framing (a 32-byte preamble
locating the rkyv header and directory blocks, then byte-contiguous tiles) is
specified in ADR 0063 so the native and wasm tile sources read to the same layout.

### Converting a GDS to a streamable archive

`reticle convert <in.gds> <out.rtla>` joins those two pieces into one command: it turns
a GDSII file into a `.rtla` archive you can browse over an HTTP range or a memory map
without ever loading the whole die. It streams the input twice, holding no whole-file
model. Pass one scans every record to find the world bounding box, recover the database
scale, and count the drawn elements; pass two reopens the file as a lazy record source
and feeds it to `build_rtla`. Peak memory is the builder's spill budget, not the file
size, so the command scales to the same gigabyte dies the builder targets. The
conversion is byte-deterministic: records are emitted in document order, the world box
is an order-independent union, the pyramid depth is a pure function of the world span,
and the builder writes no timestamps, so the same input always produces identical
bytes.

Its v1 flatten scope is deliberately narrow: only *directly drawn* geometry becomes a
record. Each boundary and path bounding box is one tile record, in database units as
authored (a path is inflated by half its width). Instance and array references are
**not** composed into world space, because expanding a placement needs random access to
the referenced cell's geometry (a whole-file model), the very thing the streaming path
avoids. A referenced cell's own shapes are still captured where they are drawn, in that
cell's own frame, so a flat or already-flattened GDS (what the tile generator and the
export path emit) converts faithfully, while a deeply hierarchical one does not yet
reproduce its placements. True hierarchical flattening is a documented follow-up. See
ADR 0072 for the flatten and leveling choices.

## Robustness

The parsers are fuzzed. A parser must never panic or hang on malformed input; it
either produces a document or returns an error. The streaming `GdsRecordReader` holds
the same guarantee: its `gds_stream` fuzz target seeds from the committed GDS crash
fixtures so it cannot reintroduce a fixed panic class, and a native regression test
drives it over those fixtures on every platform (libFuzzer cannot link on MSVC).

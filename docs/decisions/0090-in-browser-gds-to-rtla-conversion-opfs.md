# 0090, in-browser GDS-to-`.rtla` conversion into OPFS (PLACEHOLDER number)

> Placeholder ADR number: this lane (v8-6c) parks green ahead of the Wave 6 gate; the
> final number is assigned at merge to avoid collisions with sibling lanes.

## Context

The native `reticle convert` (ADR 0072) turns a GDSII file into a streamable `.rtla`
archive, and the browser already streams a hosted archive over HTTP Range (ADR
0062/0073). What was missing is the browser doing the *conversion*: dropping a GDS into
the page and getting a streamable die, with no server round trip and no upload.

Two facts shape the design. First, the frozen archive builder (`build_rtla`, ADR 0068) is
irreducibly disk-based: it spills sorted runs to a scratch directory and seek-backpatches
its output file, so it cannot run on `wasm32-unknown-unknown`, which has no filesystem.
Second, the browser has no addressable filesystem to hand a converted archive to the
existing `?archive=` reader, which fetches an archive by URL over Range.

## Decision

**An in-memory builder, additive to the frozen surface.** Add `build_rtla_to_vec`
alongside `build_rtla`: it does the same work with one in-memory sort instead of an
external merge sort and assembles the archive into a `Vec<u8>`. It reuses the module's
framing (preamble, header, directory, aligned tiles), collection order, tile-index sort,
and per-tile cap, so for any input that fits in a single sort chunk (every browser-scale
layout) it is **byte-identical** to `build_rtla`. `build_rtla` is untouched; a test pins
the byte equality. The trade-off is memory: the in-memory builder holds the whole archive,
so it is for browser-scale layouts, while the native converter keeps the bounded-memory
streaming builder for multi-gigabyte dies.

**A conversion core reusable in wasm.** `convert_gds_to_rtla(&[u8]) -> (Vec<u8>, summary)`
(in the `web` crate) runs the same v1 flatten and world-span leveling as the native
converter (ADR 0072): drawn `BOUNDARY`/`PATH` geometry only, `SREF`/`AREF` not composed,
pyramid depth from the world span. It reads the GDS from a byte slice through the frozen
streaming `GdsRecordReader` and builds with `build_rtla_to_vec`. It is a library function,
so a native test converts a fixture and reads it back through the real `MmapTileSource`.

**A Web Worker owns the conversion and the OPFS write.** `bin/convert_worker.rs` runs the
conversion off the main thread and writes the archive into the Origin Private File System
(OPFS) at `archives/<name>.rtla`, using a `FileSystemSyncAccessHandle` (worker-only, so
the write lives in the worker rather than the frozen `window`-bound tile-cache glue). It
posts `ready`/`status`/`done`/`error` messages. Trunk builds it as a `data-type="worker"`
bin; a small committed classic-worker bootstrap `importScripts` and inits it, since Trunk
emits no spawn shim.

**Reopen through the existing streaming path via a service-worker bridge.** Rather than a
new OPFS reader, the service worker serves `opfs-archive/<path>` by reading the OPFS file
and answering `Range` requests (`206` + `Content-Range`). The app opens `?archive=<that
url>` and the frozen `HttpRangeTileSource` streams the converted archive unchanged.

## Consequences

- A GDS becomes a streamable, locally stored die entirely client-side; the converted
  archive is byte-for-byte what the CLI would have produced, so browser and native output
  are interchangeable.
- The frozen `reticle-index`/`reticle-io` surface is respected: `build_rtla` and
  `gds_stream` are called, not changed; the only addition is one small `build_rtla_to_vec`
  helper. The disk-based streaming builder and the multi-gigabyte scale it targets are
  unaffected.
- OPFS is required only for the browser convenience path and is used honestly: a
  `FileSystemSyncAccessHandle` needs a Worker and a secure context, so where OPFS is
  absent the conversion is reported unsupported and the browser e2e skips that half rather
  than failing. The `browser-convert` e2e (`just e2e-convert`) proves the full path
  headless, including the reopen and render.
- In-browser hierarchical flattening is deferred with the native converter's, and very
  large dies stay a native-converter job (the in-memory builder is browser-scale).

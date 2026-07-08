# In-browser conversion

A `.rtla` archive is normally produced by the native `reticle convert` command (see [ADR
0072](decisions/0072-gds-to-rtla-converter-flatten-and-leveling.md)) and then hosted for
streaming. The browser can also do the conversion itself: drop a GDS into the page and it
becomes a streamable archive stored locally, with no server and no upload. This is the
in-browser converter (ADR 0090).

## What happens

1. **A Web Worker converts.** Picking a GDS runs `convert_gds_to_rtla` inside a dedicated
   Web Worker (`crates/web/src/bin/convert_worker.rs`), off the main thread, so a large
   file never freezes the UI. The worker streams the bytes through the same frozen Wave 2
   surface the native converter uses: the forward-only `GdsRecordReader` (ADR 0062) pulls
   one record at a time, and the archive builder assembles the tiled `.rtla`.

2. **The archive is written to OPFS.** The finished bytes are written into the Origin
   Private File System (OPFS) under `archives/<name>.rtla`, using a synchronous access
   handle. OPFS is a per-origin, private, persistent store; the archive stays in the
   browser and survives a reload.

3. **The app reopens it through the streaming path.** The page then opens
   `?archive=<url>` pointing at the OPFS file, and the existing streamed-archive reader
   ([Streamed documents](streaming.md)) pages tiles in on demand. The service worker
   bridges the two: it serves `opfs-archive/<path>` by reading the OPFS file and answering
   HTTP `Range` requests, so the archive streams through the ordinary `HttpRangeTileSource`
   with no new reader. The converted die opens read-only, exactly like a hosted one.

## Scope (v1)

The browser converter mirrors the native converter's v1 scope exactly (ADR 0072): only
directly drawn geometry becomes a record (each `BOUNDARY` and `PATH` bounding box, in
authored database units), `SREF`/`AREF` placements are not composed into world space, and
the pyramid depth follows the world span. A flat GDS (the common tape-out case) converts
faithfully; a deeply hierarchical one drops its instanced placements until hierarchical
flattening lands.

The one implementation difference from the native path is memory. The native builder
spills sorted runs to disk so it scales to multi-gigabyte dies; a browser tab has no
filesystem for that, so the browser uses an in-memory builder (`build_rtla_to_vec`) that
holds the finished archive in RAM. For any input that fits in a single sort chunk (every
browser-scale layout) the two builders produce **byte-identical** output, so a die
converted in the browser is the same archive the CLI would have written. Very large dies
remain a native-converter job.

## OPFS availability

Writing a whole archive at once needs a `FileSystemSyncAccessHandle`, which is only
available inside a Worker and only in a secure context (HTTPS or `localhost`). Where OPFS
is unavailable, the conversion is reported as unsupported rather than failing loudly, and
the browser end-to-end test skips the convert step honestly (mirroring the streamed-cache
OPFS fallback). On a current Chromium or Firefox over a secure origin it runs; the
`browser-convert` e2e (`just e2e-convert`) exercises the full path headless.

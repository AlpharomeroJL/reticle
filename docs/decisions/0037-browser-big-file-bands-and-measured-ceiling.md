# 0037, Browser big-file bands: an in-memory/streaming split and a measured ceiling

## Context

wasm32 is a 32-bit target: the linear memory a browser tab grants the module is
bounded, and far less is usable in practice before an allocation fails than the
theoretical address space. Importing a layout builds an in-memory `Document` plus a
flattened spatial index that together run several times the size of the input bytes,
so the largest file that actually opens is much smaller than the raw memory cap.
Opening a big file with one synchronous in-memory import would either stall the tab or
abort it with an out-of-memory error and a blank canvas, with no honest signal to the
visitor about what happened. The workspace already has an out-of-core streaming index
(`reticle_index::streaming::StreamingIndex`, ADR 0016) that demand-pages only the tiles
a viewport touches, which is exactly the tool for large inputs.

## Decision

Encode three size bands in one pure `LoadPlan::for_size(size)`, from a single measured
ceiling and a documented streaming threshold:

- **Below the streaming threshold (32 MiB):** `InMemory`, open the whole document and
  index directly, keeping every editor feature live.
- **Threshold to ceiling (32 MiB to 256 MiB):** `Streaming`, engage the streaming
  index so a viewport query pages in only the tiles it touches, with a progressive load
  and a determinate/indeterminate progress indicator (`LoadProgress`) rather than a
  synchronous stall.
- **Above the ceiling (over 256 MiB):** `TooLarge`, refuse up front with a clear,
  non-technical message naming the size and the ceiling ("exceeds the 256 MiB that the
  browser build can open; open it in the desktop app"), rather than crashing the tab.

The **ceiling is measured**, not guessed. `WASM_OPEN_CEILING_BYTES = 256 MiB` of input
bytes was arrived at on this build (wasm32-unknown-unknown, eframe 0.35 wgpu backend,
Chrome/Edge with the default per-tab wasm memory budget) by feeding the browser build
progressively larger generated GDS inputs (the TinyTapeout corpus generator scaled up,
the same shapes the importer sees in production) until an import first failed with a
wasm allocation error, then backing off to the last size that opened, framed, and
stayed interactive. It is deliberately conservative: it caps the *input* so the derived
in-memory structures still fit with renderer headroom, not the theoretical
address-space limit. The threshold is a tuning choice set where a straight in-memory
import stops feeling instant and the demand-paged index starts paying for itself, kept
well below the ceiling so there is a real streaming band rather than a cliff. Both
constants and the band decision are unit-tested; the ceiling is documented in a doc
comment for the orchestrator to fold into `docs/PERF.md` at Wave 5.

## Consequences

A big file degrades gracefully instead of crashing the tab: it streams with a progress
indicator, and a genuinely-too-large file is refused with a message a person can act
on. The two thresholds are a single source of truth in `webopen`, so the numbers are
revisitable against `docs/PERF.md` measurements without hunting through UI code. The
ceiling is honest, a real measurement on this build rather than an aspiration, so the
"this file exceeds what the browser build handles" message is truthful. The measurement
is environment-specific (browser, memory budget, and the import's allocation profile),
so it is documented as such and re-measured if the importer or the target changes; the
band logic itself is pure and does not depend on any of that. The streaming path reuses
the existing `StreamingIndex` primitive rather than inventing a second out-of-core
mechanism.

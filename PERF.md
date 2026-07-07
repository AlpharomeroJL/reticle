# Performance measurements

Measured numbers with their methodology, so a regression is visible against a
recorded baseline rather than a remembered one. See `docs/src/performance.md` for the
general methodology and `docs/src/benchmark.md` for the agent-benchmark harness.

## DRC as you type: per-edit re-check latency

DRC-as-you-type re-checks only the region an edit dirtied, synchronously, on every
edit (`reticle_app::live_drc::LiveDrc::recheck` over
`reticle_drc::PreparedDrc::check_region`). The expensive index rebuild
(`LiveDrc::reprepare`) runs on a throttle off that hot path, so the per-edit cost is
the `check_region` query, not the rebuild.

Measured by the app-level test
`per_edit_recheck_under_one_millisecond_at_one_million_shapes` in
`crates/reticle-app/tests/drc_live.rs`:

- **Scene**: a 1000 x 1000 grid of 50-DBU rects on a 100-DBU pitch, one million
  shapes, spacing-clean under a 10-DBU minimum-spacing rule.
- **Method**: prepare the live index once, then time `LiveDrc::recheck` over 500
  edit-sized windows (250 DBU square, each covering a handful of rects) scattered
  deterministically across the populated area. Report the sorted median, p99, and max.
- **Profile**: the workspace `test` profile (optimized + debuginfo), single-threaded,
  no GPU.

Recorded on the lane development machine (Windows 11, results vary by host):

| Quantity | Value |
| --- | --- |
| Per-edit `check_region`, median | 6.0 us |
| Per-edit `check_region`, p99 | 30.5 us |
| Per-edit `check_region`, max | 134 us |
| One-time index prepare (1M shapes, off the hot path) | 1131 ms |

The per-edit re-check is three to four orders of magnitude under the one-millisecond
budget, so the underline lands well within a single frame. The test asserts both the
median and the p99 stay under 1 ms, so a regression fails the gate rather than only
the record.

The one-time prepare is the throttled step, not the per-edit path: it includes
flattening the top cell (a full clone of the million shapes) before the engine's own
`prepare`, which is why it runs above the engine's standalone prepare figure. It never
runs on the synchronous edit path, so it does not enter the per-edit budget.

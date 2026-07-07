# 0056, GDSII export is byte-reproducible: a fixed date stamp, reconciled from an orphaned debug worktree

## Context

A spawned debug session (git worktree `.claude/worktrees/amazing-payne-7080ae`, branch
`claude/amazing-payne-7080ae`, wip checkpoint `043e40a`) investigated why two exports of
the same generated design produced different bytes. It found the cause and left the fix
uncommitted and unmerged. The v7 finish reconciles that work onto main.

**The cause.** `gds21`'s writer defaults every BGNLIB and BGNSTR date record to
`Utc::now`, so an otherwise fully deterministic `Document` exports to different bytes on
every run: the seconds field ticks between two `xtask gen-layout` invocations. This
breaks the generator's determinism contract (the same parameters must yield the same
file) and makes any byte-level reproducibility check flaky.

**The overlap with work already on main.** While the debug session ran, main separately
landed `2d816f9` (`export_order.rs`), which pins the *other* determinism property: both
exporters sort cells by name before writing, so a `HashMap`'s randomized iteration order
never leaks into the output. The debug worktree's `export_determinism.rs` re-tested that
same cell-order property (in two of its four tests) in addition to the new timestamp
behaviour. Merging it verbatim would duplicate the cell-order coverage that `2d816f9`
already owns.

## Decision

**Stamp a fixed, valid date into every GDSII date record on export.**
`reproducible_dates()` returns `2023-01-01T00:00:00` for both the `modified` and
`accessed` fields of the library and of every struct. The constant deliberately matches
the corpus generator's `valid_dates()` (verified: `gen_tinytapeout_corpus.rs` uses
`[2023, 1, 1, 0, 0, 0, ...]`), so every reproducible GDSII in the tree carries the same
stamp. `chrono` is already in the dependency graph via `gds21`; adding it as a direct
dependency of `reticle-io` (pinned `0.4.45`, the version already resolved in the lock)
only records the exporter's use of `NaiveDate`.

**Carry only the non-duplicated part of the debug worktree onto main.** The substantive
gap main lacked is the timestamp fix and its two regression tests
(`gds_export_is_byte_reproducible`, `gds_export_carries_no_wallclock_time`). Those land.
The worktree's two cell-order tests are not carried, because `export_order.rs` (already on
main at `2d816f9`) covers that property directly, including the stronger assertion that
the emitted order equals the name-sorted sequence. `export_determinism.rs` is trimmed to
the timestamp tests and cross-references `export_order.rs` in its module docs, so the two
files partition the determinism surface (order versus timestamps) with no overlap.

## Consequences

- Exporting the same `Document` twice now yields byte-identical GDSII. The two
  `xtask gen-layout` runs that motivated the investigation are reproducible on disk.
- `reticle-io` gains a direct `chrono 0.4.45` dependency (one added line in `Cargo.lock`,
  no new transitive crates: `chrono` was already resolved through `gds21`).
- Nothing from the debug worktree is silently discarded: the timestamp fix and its tests
  are on main; the cell-order tests are intentionally dropped as duplicates of `2d816f9`,
  recorded here; the wip checkpoint `043e40a` preserves the raw investigation for audit
  before the worktree and branch are removed.
- The OASIS writer already embeds no timestamps and sorts cells by name, so it needed no
  change; its reproducibility remains pinned in `export_order.rs`.

# 0057, The "GDS AREF-decode off-by-one" was a measurement misdiagnosis, not a parser bug

## Context

Wave 5B filed a follow-up item: a "GDS AREF-decode off-by-one found in passing", recorded
in `docs/PERF.md`, `docs/STATUS.md`, and `docs/TASKS.md`. The symptom was that a generated
hierarchical design appeared to flatten to 4,194,304 leaves when `reticle` was run directly
but to 1,882,384 leaves when launched by `scripts/measure-run.ps1` (`UseShellExecute=false`,
stdout redirected to a pipe). The note attributed this to "an off-by-one in the AREF COLROW
decode, consistent with an uninitialized read" and warned that the v5 scale-proof figures,
gathered the same way, reflected the smaller (wrong) parse.

A spawned debug session (git worktree `.claude/worktrees/festive-sammet-785693`, branch
`claude/festive-sammet-785693`, wip checkpoint `baf627c`) re-investigated and reached the
opposite conclusion. The v7 finish confirms it against the code and closes the item.

**What the code actually does.** The AREF import (`array_ref_to_array` in
`crates/reticle-io/src/gds.rs`) copies `aref.cols`/`aref.rows` verbatim into
`ArrayInstance { columns, rows }` (only clamping a negative count to zero) and derives the
pitch as `span / count`, exactly per the GDSII spec (the three `xy` points are the origin,
a point `columns` pitches away, and a point `rows` pitches away). `Document::flatten` loops
`for col in 0..columns { for row in 0..rows { .. } }`, and `ArrayInstance::total()` is
`columns * rows`. The whole path is `gds21` plus safe Rust with no `unsafe` and no
uninitialized memory, so identical bytes yield identical counts on every launch. There is
no off-by-one.

**What actually differed.** Two distinct designs are written to the same `scratch/gen.gds`
path in this repo: `xtask gen-layout --shapes 2000000 --layers 8 --depth 3` (the
4,194,304-leaf design) and `just gen-layout 1000000 8 3 scratch/gen.gds` (a 1,882,384-leaf
design used in the README/user guide). `measure-run.ps1` started the child via .NET
`ProcessStartInfo` without setting `WorkingDirectory`, so a relative `scratch/gen.gds`
resolved against `[Environment]::CurrentDirectory` (which PowerShell's `Set-Location` does
not update) rather than the shell's current location. When those two directories held
different `scratch/gen.gds` files, the harness silently measured whichever file it found.
Each count is the correct flatten of the file that was actually read. The physical
cross-check confirms it: the DRC/extract peak memory for the larger run (1426 MB / 1074 MB)
is about double the same generator at 1,882,384 leaves (695 MB / 500 MB), consistent with
twice the leaves, not a misparse.

## Decision

**Record the item as a misdiagnosis and retract the off-by-one claim** in all three
documents. The parser is correct.

**Fix the real bug in the measurement harness.** `measure-run.ps1` gains a
`-WorkingDirectory` parameter defaulting to the caller's current location and sets
`$psi.WorkingDirectory` on the child, so relative arguments resolve where the caller
expects. The scale-proof designs are also passed by absolute path, which closes the
ambiguity independently.

**Pin the decode's correctness with a regression test** built from the same kind of nested
square-array design the generator produces (the "sample that exposed it"). The test exports
an array-bearing document to GDSII, re-imports it, flattens, and asserts the exact leaf
count equals the product of the per-level `columns * rows`, plus a single-row/single-column
edge case. A real off-by-one in the COLROW decode or the flatten loop fails it.

## Consequences

- The filed "AREF off-by-one" is closed as not-a-bug. `docs/PERF.md` carries the corrected
  root-cause analysis (reconciled from `baf627c`); `docs/STATUS.md` and `docs/TASKS.md` link
  here instead of promising a phantom fix.
- The v5.0.0 scale-proof numbers stand as the 4,194,304-leaf figures. `docs/PERF.md` was
  re-measured on 2026-07-06 with the design pinned by absolute path and updated in place.
- `measure-run.ps1` no longer depends on the accident of `[Environment]::CurrentDirectory`
  matching the shell location. This was the actual defect behind the confusing counts.
- The AREF decode is now covered by an explicit round-trip leaf-count test, so a future
  regression in the COLROW handling is caught directly rather than misattributed.
- Lesson recorded: a launch-context-dependent number is a measurement-harness suspect first.
  A "parser off-by-one, consistent with an uninitialized read" hypothesis was wrong for code
  that is pure safe Rust; the physical memory cross-check and reading the actual decode would
  have caught it sooner.

# 0074, DRC as you type: a throttled snapshot rebuild with synchronous per-edit re-check and spell-checker underlines

> Placeholder number: the orchestrator assigns the final ADR number and renames this file
> and its README row at the Wave 3 completion merge.

## Context

`reticle-drc` already exposes an incremental checker: `DrcEngine::prepare(doc, cell)`
builds a `PreparedDrc` index, and `PreparedDrc::check_region(region)` re-checks only the
rules touching a rectangle (measured at microsecond scale even on a million-shape cell).
The DRC panel ran a full pass on demand and listed violations. What was missing was the
live wiring: violations underlined the moment geometry is drawn, as an editor squiggles a
misspelling while you type.

Two facts shaped the design. First, `check_region` is cheap (microseconds) but `prepare`
is expensive (tens to hundreds of milliseconds, and more in-app because the top cell is
flattened first). Second, a `PreparedDrc` is an immutable snapshot: mutating the document
does not update it, so an edit is reflected only after the index is rebuilt. Re-preparing
on every keystroke is therefore out of the question.

The engine is frozen for this wave, so this is app-crate plumbing over the existing
`PreparedDrc` and `check_region` surface, additive only.

## Decision

**Two costs on two cadences.** `reticle_app::live_drc::LiveDrc` owns the `PreparedDrc`
snapshot and the live violation set. The cheap `check_region` runs synchronously on the
edit's dirty region; the expensive rebuild runs on a throttle off that hot path. The app's
frame loop calls `LiveDrc::apply_dirty` once per frame: it accumulates the region dirtied
since the last rebuild and, when the throttle fires (a quarter-second interval, or the
first edit), rebuilds the index and re-checks the accumulated region against the fresh
snapshot. Between rebuilds the underlines show the last snapshot, a brief and bounded lag.

**The dirty region comes from the edit pipeline, computed before the edit lands.**
`History` gained a `Dirty` accumulator (`None` / `Region(Rect)` / `Full`) merged across
every `apply`. A shape add dirties its own bounding box; a shape remove dirties the box of
the shape still at that index (so it is read before the remove); a label edit dirties
nothing (it is not DRC geometry); a structural edit (cell, instance, array) or an undo or
redo dirties `Full`, because its region is not cheaply bounded. The app drains it each
frame with `History::take_dirty`. A `Full` re-check sweeps the whole indexed bounds, which
`LiveDrc` records at prepare time.

**Underlines are a spell-checker squiggle, distinct from the panel markers.**
`drc_panel::squiggle_points` builds a zig-zag polyline in constant screen pixels along a
violation's bottom edge, so it reads the same at any zoom. `App::draw_live_drc_underlines`
paints it for each live violation, layered over the existing boxed markers of a full DRC
run (`draw_drc_markers`) but visually separate: the boxes are a completed audit, the
squiggles are live "misspellings" at the edit.

**Off by default, opt-in from the panel.** A "Check as you type" toggle turns it on, so a
large load does not pay the first index build until the user asks. Loading a new document
or turning the toggle off drops the index and its underlines.

## Consequences

The per-edit path is the `check_region` query, not the rebuild. Measured at a million
shapes it is median 6 microseconds, p99 30 microseconds, three to four orders of magnitude
under the one-millisecond budget; `PERF.md` records the numbers and the app-level test
`per_edit_recheck_under_one_millisecond_at_one_million_shapes` asserts both the median and
the p99 stay under 1 ms so a regression fails the gate.

The snapshot model means a just-*added* shape is underlined only after the next rebuild
(up to the throttle interval later), not on the exact frame it is drawn. This is the
spell-checker lag, and it is deliberate: rebuilding on every edit would stall on large
cells. Moving or deleting geometry has the same bounded lag, because the snapshot must be
rebuilt to reflect any change.

The two-way behaviour (draw two rects too close, a violation is underlined; move one
apart, it clears) is covered headlessly with no GPU by
`crates/reticle-app/tests/drc_live.rs`, driving the real `History` edit pipeline and the
real `LiveDrc::apply_dirty` step. `LiveDrc::recheck` keeps the live set minimal by
replacing only the violations touching the region it re-checked, so the underlines
converge to the edited neighbourhoods without a whole-cell sweep.

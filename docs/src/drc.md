# Design-rule checking

`reticle-drc` verifies that a layout obeys the process design rules.

## Declarative rules

Rules come from the technology file, not from code. The engine understands width,
spacing, enclosure, extension, notch, area, density, and angle constraints, each
expressed as a rule over one or two layers with a threshold. Because the rule set
is data, a new process is a new technology file rather than a new build.

A built-in deck for SKY130 ships with the crate: `sky130_drc_rules()` loads the
committed subset of the SkyWater periphery rules. It is a subset and passing it
is not tape-out clean; see [SKY130 rule coverage](sky130-drc-coverage.md) for
exactly which rule ids are checked and which whole rule families are not.

## How checks run

A check evaluates each rule against the geometry using the spatial index to find
candidate pairs, and the geometry booleans and offsetting to test spacing and
enclosure. Violations carry the offending region's bounding box so the UI can zoom
straight to them.

## Incremental re-check

Editing a layout should not re-check the whole design. When a shape changes, only
the rules touching the affected region are re-run, using the index to bound the
work. The target is under a hundred milliseconds for a local edit, so the violation
overlay stays live as you draw.

## DRC as you type

The editor wires that incremental re-check to live editing, so a violation is
underlined the moment its geometry is drawn, the way a text editor squiggles a
misspelling as you type. The DRC panel's "Check as you type" toggle turns it on.

Two costs run on two cadences. The cheap step re-checks only the region an edit
dirtied (`check_region` over the edit's neighbourhood, measured at microsecond scale
even on a million-shape cell) and runs synchronously on every edit. The expensive step
rebuilds the whole prepared index and runs on a throttle, off the per-edit hot path,
because a `PreparedDrc` is an immutable snapshot: an edit is reflected only once the
index is rebuilt. Between rebuilds the underlines show the last snapshot, so a
just-drawn shape enters the checks after a brief, bounded lag, like a spell-checker
catching up after a burst of typing.

The dirty region comes from the edit pipeline: a shape add or remove dirties its
bounding box, while a structural change or an undo dirties the whole cell (its region
is not cheaply bounded). The live underlines are a spell-checker squiggle drawn beneath
each violation at constant on-screen size, deliberately distinct from the boxed markers
of a full DRC-panel run.

The measured per-edit `check_region` latency at a million shapes, and the methodology,
are recorded in `PERF.md`.

## Testing

The engine is checked against a naive reference implementation that tests every
rule the slow, obvious way over randomized inputs, so the fast path cannot silently
disagree with the specification.

The live wiring is covered at the app level by
`crates/reticle-app/tests/drc_live.rs`: drawing two rects too close underlines a
spacing violation, and moving one apart clears it, driving the real edit pipeline
headlessly with no GPU.

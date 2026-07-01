# Design-rule checking

`reticle-drc` verifies that a layout obeys the process design rules.

## Declarative rules

Rules come from the technology file, not from code. The engine understands width,
spacing, enclosure, extension, notch, area, density, and angle constraints, each
expressed as a rule over one or two layers with a threshold. Because the rule set
is data, a new process is a new technology file rather than a new build.

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

## Testing

The engine is checked against a naive reference implementation that tests every
rule the slow, obvious way over randomized inputs, so the fast path cannot silently
disagree with the specification.

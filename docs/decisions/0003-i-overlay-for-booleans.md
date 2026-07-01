# 0003, `i_overlay` for polygon booleans and offsetting

## Context

Reticle needs robust polygon boolean operations (union, intersection, difference,
XOR) and offset/sizing over integer coordinates, for DRC, extraction, and editing.
Hand-rolling a numerically robust clipper (Vatti/Greiner-Hormann with exact
predicates) is a multi-month effort and a classic source of subtle correctness
bugs on self-intersecting or degenerate input.

## Decision

Use the `i_overlay` crate for all boolean and offset work in `reticle-geometry`.
It provides robust boolean ops and offsetting with both integer and float APIs,
handles self-intersection and holes, and powers the booleans in the `geo` crate,
so it is well-exercised. Reticle wraps it behind our own `Polygon`/`Path` types so
the dependency stays swappable.

## Consequences

We get correct, fast booleans without owning the hardest geometry code. We still
own a brute-force oracle (grid rasterization or half-plane tests) to property-test
our wrappers against, and we fuzz the boolean entry points for crashes and invalid
output. The wrapper layer isolates any future `i_overlay` API drift to one module.
Hand-rolling booleans is explicitly rejected per the spec's Appendix A guidance.

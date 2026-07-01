# 0006 — Contract-first compiling skeleton

## Context

Wave 2 places `reticle-model` in the same wave as its consumers
(`reticle-render`, `-drc`, `-route`, `-extract`). Those consumers have a hard
compile-time dependency on model. If lanes started from empty crates, a consumer
could not compile until model was written, serializing what should be parallel work.

## Decision

In Wave 0, write a **compiling skeleton** of every workspace member: real,
frozen public types and trait signatures (the contracts) with stubbed bodies
(`todo!()`/`unimplemented!()` or trivial placeholders) so `cargo build --workspace`
is green before any lane starts. Each later wave replaces stubs with real, tested
implementations without changing the frozen signatures. The skeleton is std-only so
it never depends on external crate availability.

## Consequences

Every crate compiles at the end of every wave (satisfying "keep the project
runnable"), and lanes genuinely parallelize because they code against stable,
already-compiling interfaces. The discipline required is that signature changes
after Wave 0 are treated as contract changes: made deliberately, reconciled by the
integration role at the wave boundary, and noted if they ripple. Stub bodies are
greppable (`todo!`) so no placeholder is mistaken for a finished implementation.

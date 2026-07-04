# 0033, v7 housekeeping: prune offscreen media, regroup the changelog

## Context

Two small pieces of debt were carried into the v7.0.0 run. First, `assets/` still
held the original offscreen canvas-only renders (`browse.gif`, `agent.gif`,
`agent.png`, and the `drc`/`route`/`collab`/`minimap`/`stack3d` stills) that the
v6.0.1 media truth pass superseded with real UI captures (`hero.png` plus the five
`tour-*.gif`); none of the old set is referenced by the README or any book chapter,
and `browse.gif` alone was 5.6 MB of dead weight in the tree. Second, `cliff.toml`
grouped commits only by conventional type (`feat`/`fix`/...), so every domain-prefixed
commit this repo actually uses (`app:`, `render:`, `io:`, `agent:`, `bench:`,
`drc:`, ...) fell into a single undifferentiated "Other" bucket, making the generated
CHANGELOG read as a flat list rather than a product history.

## Decision

Remove the eight unreferenced offscreen assets with `git rm` while keeping the
offscreen harness that emits them (`xtask/src/media.rs`), so the media stays
reproducible on demand without committing the stale artifacts. Rewrite the
`cliff.toml` commit parsers to route both conventional types and domain prefixes into
meaningful, product-shaped sections (Editor and app, Rendering engine, Formats and
I/O, Agent MCP and tools, Benchmark suite, Verification, Generators, Tape-out, ...),
using zero-padded `<!-- NN -->` order tokens so sections past nine sort correctly as
strings, and skip pure bookkeeping commits (`merge:`, `wip:`) that are not
changelog-worthy.

## Consequences

The tree drops about 6 MB of unreferenced binaries and the README/book media set is
now exactly what ships; regenerating the old stills is still one `just capture-media`
away if ever needed. The next release's CHANGELOG (regenerated wholesale by git-cliff
over full history) gains readable per-subsystem sections and loses the merge/wip
noise. The trade-off is that the changelog no longer lists integration-merge commits;
they remain in `git log`. A future domain prefix just needs one more parser row; a
decision to resurrect a pruned asset is a `capture-media` run plus a README reference.

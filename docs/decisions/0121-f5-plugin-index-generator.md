# 0121, the F5 static plugin index generator

## Context

ADR 0105 froze the F5 contract: `Manifest`, the v0 host-function table, and the static
`Index { entries }` (deterministically sorted by `manifest.id`, a 64-char lowercase-hex
`wasm_sha256` per entry). Something has to actually walk a `plugins/` directory,
compute the real content hash, and write the committed `library/plugins/index.json`
the manager UI (`plugin-ui`) browses and the desktop host loads by source path. That
generator, and three small decisions it had to make, are this ADR.

Phase 4's plugin lanes are dispatched concurrently: this lane (`plugin-manifest-index`)
builds the generator while `plugin-sample` builds the first real plugin in parallel, in
a sibling worktree with no live coordination channel. At generation time, `plugins/`
does not exist on `main` (confirmed against the dispatch base commit 973cfc0): the
`lane/plugin-sample` branch exists but had not yet merged.

## Decision

**Location and reuse, not a mirror.** The generator is `xtask plugin-index
<plugins-dir> <out.json>`, matching the `verify-licenses` / `library-manifest`
precedent in the same crate. It depends on `reticle-plugin` directly (a path
dependency declared inside `xtask/Cargo.toml`, not routed through the root
`Cargo.toml`'s `[workspace.dependencies]`, so the addition never touches a file shared
across every lane) and builds `Index` / `IndexEntry` / `Manifest` values with the real
contract types. A hand-mirrored struct could silently drift from `manifest.rs`; reusing
the real type makes that class of bug impossible rather than merely tested-for. xtask
is a native-only dev tool never compiled for wasm32, so this adds no wasm bundle cost
and no new external crate for `cargo deny` to evaluate (`wasmi` was already resolved in
the graph via `reticle-plugin` itself, ADR 0116).

**Manifest file convention: `manifest.json`.** `plugin-sample`'s own brief left this
open ("manifest.json or .toml"). JSON was chosen because the whole F5 contract is
already JSON end to end (the fixture, `Index`, `Manifest`'s own derives) and
`reticle-plugin`'s existing dev-dependency is `serde_json`, not a TOML crate; picking
TOML here would have meant adding a new dependency to parse a format nothing else in
the contract uses. Each plugin subdirectory must carry exactly one `manifest.json` and
exactly one `*.wasm` file (scanned one level deep, so a local `target/` build directory
is never considered); zero or more than one of either is a hard, fail-closed error
naming the offending directory, never a guess.

**A missing or empty `plugins-dir` is valid, not an error.** `verify-licenses` and
`library-manifest` both treat a missing directory as an operator error (their staged
content is expected to already exist). The plugin index's domain is different: Phase 4
is mid-flight, so "zero plugins currently ship" is an honest, valid state, not a
pipeline failure. `plugin-index` therefore writes an empty (still `Index::validate`-
passing) index when `plugins-dir` does not exist, rather than failing the build. This
is a deliberate, narrow divergence from the sibling generators' stricter contract,
scoped to this one generator; it is not a general relaxation of the fail-closed rule
for malformed content once a plugin directory *does* exist.

## Consequences

`library/plugins/index.json` is committed as `{"entries": []}`: real and honest for
what exists on this branch today, never a fabricated entry for a plugin whose wasm
was not actually read and hashed. Once `plugin-sample` merges, re-running the exact
same command (`xtask plugin-index plugins library/plugins/index.json`) picks up the
new plugin automatically and re-commits a non-empty index; the gate is expected to do
this re-run once both lanes have landed. If `plugin-sample` ships a `.toml` manifest
instead of `.json`, that is a fail-closed error from this generator ("cannot read
manifest.json") until either the manifest is renamed or a follow-on patch teaches this
generator to also read `.toml`; this ADR flags the gap rather than silently guessing a
format. `crates/reticle-plugin/tests/committed_index.rs` pins that the committed file
parses and validates via the real `reticle_plugin::manifest::Index` at every entry
count, including today's zero, so this invariant is checked on every future
regeneration, not just this one.

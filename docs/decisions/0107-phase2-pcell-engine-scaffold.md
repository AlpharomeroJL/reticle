# 0107, Phase 2: the PCell engine scaffold and the script-to-gen edge

## Context

Phase 2 adds a user-defined parametric-cell (PCell) engine: a user authors a rhai script
whose top-level parameter bindings are described by a `ParamSchema`, and the app produces it
into geometry with a stable content identity, caching produced cells so an unchanged
parameter set is not re-run. F2 (ADR 0102) already froze the *provenance* half in
`reticle-gen` (`ProduceMeta` and the `SHA-256` param-hash recipe over
`canonical_params_json`). The remaining engine is fanned out across four parallel lanes with
a genuine runtime dependency chain: `pcell-params` (the definition, validation, and
registry), `pcell-produce` (the sandboxed rhai execution), `pcell-cache` (the produced-cell
cache), and `pcell-harness` (the exercising tests). A UI lane (`pcell-inspect`) renders a
PCell's parameters and provenance.

Fanning out a dependency chain safely needs a shared interface committed before the lanes
start, so each lane owns a disjoint file and compiles against fixed signatures rather than
racing on the same types. Two facts shape the interface: the produce metadata and param
identity live in `reticle-gen`, but rhai lives in `reticle-script`, and `reticle-script` does
not yet depend on `reticle-gen`.

## Decision

Commit a compiling scaffold before dispatch and split the engine across the two crates so the
lanes are file-disjoint.

**`reticle-gen` owns identity, params, and cache** (no rhai):
- `pcell::PCellDef { id, title, description, schema: ParamSchema, script, engine_version }`
  with `param_hash`, `produce_meta`, and `validate_params` (`def.rs`, `pcell-params` lane).
- `pcell::PCellRegistry` (`registry.rs`, `pcell-params` lane).
- `pcell::PCellCache` / `CacheStats` (`cache.rs`, `pcell-cache` lane).
- `pcell::param_hash` (`hash.rs`) is implemented and tested in the scaffold, **front-loaded**
  rather than left to `pcell-params` as ADR 0102 anticipated. Every PCell lane keys on this
  identity (produce stamps it, the cache keys on it, the harness asserts it), so one tested
  implementation the parallel lanes agree on is worth the small deviation. This adds a `sha2`
  dependency to `reticle-gen` (already vendored transitively, pure-Rust and wasm32-clean, so
  the browser build is unaffected).

**`reticle-script` owns the sandboxed producer**: `pcell::produce(def, params, tech, limits)
-> Result<(Cell, ProduceMeta), ProduceError>` plus `SandboxLimits` and `ProduceError`
(`pcell.rs`, `pcell-produce` lane). This is the first `reticle-script -> reticle-gen`
dependency edge; it is acyclic (`reticle-gen` does not depend on `reticle-script`). The
producer must build its own sandboxed `rhai::Engine` (create/edit API, no filesystem or
plugin-directory access, an `on_progress` operation-count bound, output-size caps) and must
not route through `ScriptEngine::run_plugin_dir`, whose filesystem loader is the opposite of
a sandbox. Enforcing `SandboxLimits` is the untrusted-input invariant applied to scripts: a
runaway or hostile PCell must fail cleanly, never hang the tab or exhaust memory.

The scaffold's lane-owned bodies are stubs that compile and fail honestly (`validate_params`
returns `Ok`, `produce` returns `ProduceError::NotImplemented`, the cache is unbounded); each
owning lane replaces its stub. `ParamSchema` gains `Default` (additive) for construction.

## Consequences

- The four PCell lanes and `pcell-inspect` build against a frozen interface and merge as
  disjoint files (append-only re-export blocks in `lib.rs`, like the Phase 1 app hooks).
- The `param_hash` identity is fixed and tested now; a future recipe change is a deliberate,
  ADR-gated event, and `pcell-harness` should pin a known-answer vector to guard it.
- `reticle-script` now depends on `reticle-gen`; the dependency graph stays acyclic and the
  wasm build stays green (both crates are wasm32-clean on this path).
- `pcell-produce` carries the security weight of the phase: the sandbox is the boundary
  between untrusted script input and the host, and its limits are a gate concern, not a
  nicety.

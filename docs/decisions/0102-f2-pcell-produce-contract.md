# 0102, F2: the PCell produce-metadata contract and the param-hash recipe

## Context

The v8.2 campaign adds user PCells (Phase 2): the Inspector edits a generator's parameters
and regenerates, and an instance cache keys on a parameter hash so an unchanged parameter
set is not regenerated. The Inspector (pcell-inspect) must build against a frozen shape
before the produce machinery (pcell-params, pcell-produce, pcell-cache) exists, and the
cache and the UI must agree on how an instance is identified.

The generator parameter form already ships: `reticle-gen` exposes each generator's
`ParamSchema` and a `Registry` that validates JSON parameters against it
(`Registry::validate`, `default_params`, `schema`). What F2 adds is the *provenance* a
produced instance carries and the canonical input to its hash.

## Decision

F2 lives in `reticle-gen` (`produce.rs`). `ProduceMeta { generator_id, engine_version,
script_ref, param_hash }` records which generator (or user PCell) produced an instance, the
engine version, the optional `.rhai` script reference (`None` for a built-in), and the
canonical parameter hash. The param hash is `SHA-256` over `generator_id + "\n" +
engine_version + "\n" + canonical_params_json(params)`, lowercase hex.

This crate ships the deterministic half of the recipe, `canonical_params_json`: a
sorted-key, compact JSON so two parameter sets that differ only in key order hash the same.
It stays independent of serde_json's `preserve_order` feature. The `pcell-params` lane
applies the SHA-256 itself, so `reticle-gen` gains no hash dependency before it needs one;
`pcell-cache` keys on the resulting hash and `pcell-inspect` shows the provenance.

The fixture is `crates/reticle-gen/tests/fixtures/contracts/f2_produce.json` (a built-in
generator's produce metadata). The cross-test (`tests/f2_produce.rs`) validates the fixture,
then uses the real `Registry`: the named generator's default params validate against its
own schema (the form the Inspector renders), and an out-of-range edit is rejected by the
generator's own validate rather than silently produced, exercising the edit-then-revalidate
flow the Inspector relies on.

## Consequences

pcell-inspect builds against the shipped `ParamSchema` form plus the `ProduceMeta`
provenance, entirely before the produce machinery lands. Because the hash input is
canonicalized here, the cache key is stable regardless of how a parameter set was
constructed, and identical (generator, engine, params) triples share a produce identity.
The claim shape stays honest: user PCells are DRC-checked on generate (pcell-produce);
clean-by-construction is claimed only for the shipped example generators, which this
contract does not change.

//! F2 produce-metadata contract cross-test.
//!
//! The producer (pcell-params, Phase 2) and the consumer (the Inspector's pcell-inspect
//! panel) agree on: the generator's [`ParamSchema`] is the form (already shipped), a
//! produced instance carries a [`ProduceMeta`] provenance, and the param hash is taken over
//! a canonical parameter JSON. This test pins the fixture and exercises the real generator
//! registry so the form contract is validated live, not against a hand-copied schema.

use reticle_gen::{ProduceMeta, Registry, canonical_params_json};

const FIXTURE: &str = include_str!("fixtures/contracts/f2_produce.json");

#[test]
fn f2_produce_meta_and_generator_form_contract() {
    let meta: ProduceMeta = serde_json::from_str(FIXTURE).expect("F2 fixture parses");
    assert!(
        meta.has_valid_hash(),
        "the param hash is 64-char lowercase hex"
    );
    assert!(
        meta.script_ref.is_none(),
        "a built-in generator carries no script ref"
    );
    assert_eq!(meta.engine_version, "8.2.0");

    // The generator the metadata names is a real registered generator; its default params
    // validate against its own schema (the exact form the Inspector renders and edits).
    let registry = Registry::with_builtins();
    let params = registry
        .default_params(&meta.generator_id)
        .expect("the produce metadata names a registered generator");
    registry
        .validate(&meta.generator_id, &params)
        .expect("a generator's default params validate against its schema");
    assert!(
        registry.schema(&meta.generator_id).is_some(),
        "the form schema exists"
    );

    // The hash input over those params is deterministic (pcell-params applies SHA-256).
    assert_eq!(
        canonical_params_json(&params),
        canonical_params_json(&params),
        "the canonical hash input is stable"
    );

    // ProduceMeta round-trips through serde unchanged.
    let re: ProduceMeta = serde_json::from_str(&serde_json::to_string(&meta).unwrap()).unwrap();
    assert_eq!(meta, re);
}

#[test]
fn f2_edit_then_revalidate_is_the_inspector_flow() {
    // pcell-inspect edits a field and revalidates before regenerate. An out-of-range edit
    // is rejected by the generator's own validate (a structured error, not a panic), which
    // the Inspector surfaces as a stale/error state rather than silently generating.
    let registry = Registry::with_builtins();
    let mut params = registry
        .default_params("guard_ring")
        .expect("guard_ring registered");
    registry
        .validate("guard_ring", &params)
        .expect("the unedited default validates");

    params["region_width"] = serde_json::json!(-1);
    assert!(
        registry.validate("guard_ring", &params).is_err(),
        "an out-of-range edit must be rejected, not silently produced"
    );
}

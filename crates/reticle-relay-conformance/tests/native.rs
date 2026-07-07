//! The always-on half of the conformance gate: every shared vector against the
//! in-process native `reticle_server` relay, plus the two-way (negative) proof
//! that the harness actually fails a relay that breaks the contract.
//!
//! This half needs no Node, no wrangler, no network: it is part of
//! `cargo nextest run -p reticle-relay-conformance` and runs in CI-shaped time.
//! The Durable Object half lives in `conformance_do.rs`, gated on an env flag.

use reticle_relay_conformance::vectors::broken_expects_view_frame_forwarded;
use reticle_relay_conformance::{Target, run_vector, vectors};

/// Every shared vector passes against the native relay. This is one half of the
/// "identical verdicts on both relays" claim; the Durable Object half asserts
/// the same table passes there.
#[tokio::test]
async fn every_vector_passes_against_the_native_relay() {
    let target = Target::native().await;
    for vector in vectors() {
        if let Err(failure) = run_vector(&target, &vector).await {
            panic!("{failure}");
        }
    }
}

/// The two-way test: a vector that asserts a view-mode frame is *forwarded* must
/// FAIL against the real relay (which drops it). This proves the runner's
/// assertions have teeth: a relay that wrongly forwarded view frames would be
/// caught, because the correct relay makes this deliberately-wrong expectation
/// time out. Without this, a vacuous suite could pass anything.
#[tokio::test]
async fn a_broken_expectation_fails_against_the_correct_relay() {
    let target = Target::native().await;
    let broken = broken_expects_view_frame_forwarded();
    let result = run_vector(&target, &broken).await;
    assert!(
        result.is_err(),
        "expecting a dropped view frame to be forwarded must fail against a correct relay, \
         but the vector passed (the harness would not detect a broken relay)"
    );
}

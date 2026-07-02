//! Property test for GPU stream compaction ([`CellCompactor`]).
//!
//! Over random visibility bitsets, the GPU compaction must produce exactly the set of
//! indices whose flag is set (order is unspecified, so the comparison is as a set),
//! and the indexed-indirect `instance_count` it fills must equal the number of
//! survivors. The reference is the trivial CPU filter.
//!
//! The bitset length spans a single partial workgroup up through several full
//! workgroups plus a partial tail, so the per-workgroup scan, the cross-workgroup
//! range reservation, and the out-of-range tail threads are all exercised.
//!
//! Skips (and passes) without a usable GPU adapter, so it is safe in CI. The GPU
//! context and pipeline are built once and shared across all generated cases.

use std::collections::BTreeSet;

use proptest::prelude::*;
use reticle_render::{CellCompactor, WgpuContext};

/// Largest generated bitset. `compact.wgsl` uses a 256-thread workgroup, so this spans
/// two full workgroups plus a partial third (512 < 650 < 768), covering the tail and
/// the multi-workgroup reservation path.
const MAX_LEN: usize = 650;

/// The CPU reference: the sorted set of indices whose flag is nonzero.
fn cpu_reference(flags: &[u32]) -> BTreeSet<u32> {
    flags
        .iter()
        .enumerate()
        .filter(|&(_, &f)| f != 0)
        .map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX))
        .collect()
}

#[test]
fn compaction_matches_cpu_reference() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let compactor = CellCompactor::new(&ctx);

    // A vector of 0/1 flags of a random length in 0..=MAX_LEN. Building the length and
    // the per-element bit separately keeps short and long bitsets both well sampled.
    let flags_strategy = (0usize..=MAX_LEN)
        .prop_flat_map(|len| proptest::collection::vec(prop::bool::ANY.prop_map(u32::from), len));

    // GPU work per case is real (dispatch + two readbacks), so cap the case count to
    // keep the test brisk while still covering a broad range of bitsets.
    let mut runner = proptest::test_runner::TestRunner::new(proptest::test_runner::Config {
        cases: 64,
        ..proptest::test_runner::Config::default()
    });

    runner
        .run(&flags_strategy, |flags| {
            let output = compactor.compact(&ctx, &flags);
            let (survivors, instance_count) = compactor.read_back(&ctx, &output);

            let expected = cpu_reference(&flags);
            let got: BTreeSet<u32> = survivors.iter().copied().collect();

            prop_assert_eq!(
                instance_count as usize,
                expected.len(),
                "instance_count {} must equal survivor count {}",
                instance_count,
                expected.len()
            );
            prop_assert_eq!(
                survivors.len(),
                expected.len(),
                "compacted output must have no duplicates or gaps"
            );
            prop_assert_eq!(got, expected, "compacted set must match the CPU reference");
            Ok(())
        })
        .expect("compaction property holds for all generated bitsets");
}

/// A couple of fixed edge cases the random search may under-sample: empty input, all
/// culled, and all kept across a workgroup boundary.
#[test]
fn compaction_edge_cases() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let compactor = CellCompactor::new(&ctx);

    // Empty input: no survivors, zero instance count.
    let (survivors, count) = compactor.read_back(&ctx, &compactor.compact(&ctx, &[]));
    assert!(survivors.is_empty());
    assert_eq!(count, 0);

    // All culled over more than one workgroup.
    let none = vec![0u32; 300];
    let (s, c) = compactor.read_back(&ctx, &compactor.compact(&ctx, &none));
    assert!(s.is_empty());
    assert_eq!(c, 0);

    // All kept over more than one workgroup: every index survives exactly once.
    let all = vec![1u32; 300];
    let (mut s, c) = compactor.read_back(&ctx, &compactor.compact(&ctx, &all));
    assert_eq!(c, 300);
    s.sort_unstable();
    let expected: Vec<u32> = (0..300).collect();
    assert_eq!(s, expected, "every index must survive exactly once");
}

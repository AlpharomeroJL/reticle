//! Adapter-gated tests for the fully GPU-resident arrayed hierarchy
//! ([`GpuHierarchy`]).
//!
//! The GPU path expands array placements, culls each element against a viewport, and
//! stream-compacts the survivors — all on the GPU. These tests check it against a
//! trivial CPU reference expansion of the same scene:
//!
//! * the compacted survivor SET equals the CPU reference (order is unspecified);
//! * splitting the element space into many small chunks (the mechanism that escapes the
//!   single-dispatch 65,535-workgroup / 128 MiB cap) yields the same union of
//!   survivors, so chunking is transparent;
//! * the per-frame GPU path performs zero CPU per-element expansions, so the
//!   `cpu_expand_ops` counter is flat across frames.
//!
//! Skips (and passes) without a usable GPU adapter, so it is safe in CI. The context
//! and pipeline are built once and reused.

use std::collections::BTreeMap;

use reticle_geometry::{Point, Rect};
use reticle_render::{
    ArrayPlacement, GpuHierarchy, InstanceTransform, RectInstance, RectInstanceT, cpu_expand_ops,
};

/// A base placement transform with unit magnification: orientation `code`, translation
/// `t`.
fn base(code: u32, t: [i32; 2]) -> InstanceTransform {
    InstanceTransform {
        orientation_code: code,
        magnification: 1.0,
        translate: t,
    }
}

/// A canonical, hashable key for an expanded instance. The GPU and CPU compute
/// byte-identical instances (integer translation, floats copied straight from the cell
/// and placement), so equality as a multiset is exact.
fn key(inst: &RectInstanceT) -> [u32; 12] {
    [
        inst.min_xy[0].to_bits(),
        inst.min_xy[1].to_bits(),
        inst.max_xy[0].to_bits(),
        inst.max_xy[1].to_bits(),
        inst.color[0].to_bits(),
        inst.color[1].to_bits(),
        inst.color[2].to_bits(),
        inst.color[3].to_bits(),
        inst.orientation_code,
        inst.magnification.to_bits(),
        inst.translate[0] as u32,
        inst.translate[1] as u32,
    ]
}

/// A multiset of expanded instances keyed canonically, for order-independent equality.
fn multiset(instances: &[RectInstanceT]) -> BTreeMap<[u32; 12], usize> {
    let mut m = BTreeMap::new();
    for inst in instances {
        *m.entry(key(inst)).or_insert(0) += 1;
    }
    m
}

/// A single white 10x10 leaf rect at the origin — the minimal arrayed cell.
fn leaf() -> RectInstance {
    RectInstance {
        min_xy: [0.0, 0.0],
        max_xy: [10.0, 10.0],
        color: [1.0, 1.0, 1.0, 1.0],
    }
}

#[test]
fn expansion_and_cull_match_cpu_reference() {
    let Some(ctx) = reticle_render::WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let cells = vec![leaf()];
    // Two placements of the same leaf: a 3x2 grid at the origin and a 2x2 grid offset
    // far to the right, plus a rotated single placement.
    let placements = vec![
        ArrayPlacement::new(0, base(0, [0, 0]), 3, 2, 100, 100),
        ArrayPlacement::new(0, base(0, [1000, 0]), 2, 2, 50, 50),
        ArrayPlacement::new(0, base(1, [-500, -500]), 1, 1, 0, 0),
    ];

    let mut hier = GpuHierarchy::new(&ctx);
    hier.upload(&ctx, &cells, &placements);

    // A viewport that keeps a real subset (some of grid 0, none of grid 1, the rotated
    // single one).
    let viewport = Rect::new(Point::new(-600, -600), Point::new(150, 150));
    hier.expand(&ctx, viewport);

    let gpu = hier.read_survivors(&ctx);
    let cpu = GpuHierarchy::cpu_reference(&cells, &placements, viewport);

    assert!(!cpu.is_empty(), "test viewport should keep some survivors");
    assert_eq!(
        multiset(&gpu),
        multiset(&cpu),
        "GPU survivors ({}) must equal the CPU reference ({}) as a set",
        gpu.len(),
        cpu.len()
    );
    assert_eq!(
        hier.read_survivor_count(&ctx),
        cpu.len() as u64,
        "indirect instance_count must equal the survivor count"
    );
}

#[test]
fn full_viewport_keeps_every_element() {
    let Some(ctx) = reticle_render::WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let cells = vec![leaf()];
    let placements = vec![ArrayPlacement::new(
        0,
        InstanceTransform::IDENTITY,
        20,
        20,
        100,
        100,
    )];

    let mut hier = GpuHierarchy::new(&ctx);
    hier.upload(&ctx, &cells, &placements);
    // A viewport covering the whole 20x20 grid (elements span 0..1910).
    let viewport = Rect::new(Point::new(-100, -100), Point::new(5000, 5000));
    hier.expand(&ctx, viewport);

    assert_eq!(
        hier.read_survivor_count(&ctx),
        hier.total_elements(),
        "a covering viewport keeps every element"
    );
}

#[test]
fn chunking_is_transparent_across_many_small_chunks() {
    let Some(ctx) = reticle_render::WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let cells = vec![leaf()];
    // A 40x40 grid = 1,600 elements, split into 256-element chunks (7 chunks, the last
    // partial). This exercises the cross-chunk path — the mechanism that escapes the
    // single-dispatch cap — in miniature.
    let placements = vec![ArrayPlacement::new(
        0,
        InstanceTransform::IDENTITY,
        40,
        40,
        100,
        100,
    )];

    let mut hier = GpuHierarchy::new(&ctx);
    hier.upload_with_chunk_size(&ctx, &cells, &placements, 256);
    assert!(
        hier.chunk_count() >= 2,
        "the tiny chunk size must force several chunks (got {})",
        hier.chunk_count()
    );

    // A viewport keeping a partial subset so culling is non-trivial across chunks.
    let viewport = Rect::new(Point::new(-50, -50), Point::new(1250, 2050));
    hier.expand(&ctx, viewport);

    let gpu = hier.read_survivors(&ctx);
    let cpu = GpuHierarchy::cpu_reference(&cells, &placements, viewport);
    assert!(!cpu.is_empty());
    assert_eq!(
        multiset(&gpu),
        multiset(&cpu),
        "the union of survivors across {} chunks must equal the CPU reference",
        hier.chunk_count()
    );
    assert_eq!(hier.read_survivor_count(&ctx), cpu.len() as u64);
}

#[test]
fn frame_path_does_no_cpu_per_element_work() {
    let Some(ctx) = reticle_render::WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let cells = vec![leaf()];
    let placements = vec![ArrayPlacement::new(
        0,
        InstanceTransform::IDENTITY,
        64,
        64,
        100,
        100,
    )];

    let mut hier = GpuHierarchy::new(&ctx);
    hier.upload(&ctx, &cells, &placements);
    let viewport = Rect::new(Point::new(0, 0), Point::new(3000, 3000));

    // Snapshot the CPU per-element expansion counter, then run several frames. The GPU
    // path must not perform any CPU per-element expansion, so the counter is flat.
    let before = cpu_expand_ops();
    for _ in 0..8 {
        hier.expand(&ctx, viewport);
    }
    let after = cpu_expand_ops();
    assert_eq!(
        after,
        before,
        "the per-frame GPU expand path must do zero CPU per-element work (delta {})",
        after - before
    );

    // Sanity: the CPU reference DOES bump the counter, so the assertion above is
    // meaningful (the counter is wired, not dead).
    let _ = GpuHierarchy::cpu_reference(&cells, &placements, viewport);
    assert!(
        cpu_expand_ops() > after,
        "the CPU reference must bump the counter, proving it is live"
    );
}

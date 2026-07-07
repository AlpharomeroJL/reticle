// GPU DRC check over binned rect instances, the second half of the compute-shader
// heatmap. One thread per instance:
//
//   * min-width: flag the instance if the smaller side of its world bounding box is
//     below the width rule (`min(w, h) < min_width`).
//   * min-spacing: walk the 3x3 neighbourhood of the instance's grid bin and, for each
//     other instance found there, measure the edge-to-edge gap of the two world boxes;
//     flag the instance if any neighbour sits at a strictly positive gap below the
//     spacing rule.
//
// A per-instance `violation_flags` entry is written (1 = in violation), and every
// flagged instance does one `atomicAdd` onto its bin's `heatmap` counter, so the
// heatmap holds the count of violating instances per bin - the coarse field the overlay
// draws.
//
// The geometry mirrors `reticle-drc`'s `geom.rs` exactly so the compute path agrees
// with the CPU `DrcEngine` oracle: overlapping or merely touching boxes have gap 0 and
// are never flagged; the diagonal corner gap is `floor(sqrt(dx^2 + dy^2))` computed
// with the same integer Newton `isqrt`. The 3x3 search is exhaustive because the CPU
// sizes bins to at least `max_instance_extent + min_spacing` (see `drc_heatmap.rs`).

struct RectInstanceT {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
    color: vec4<f32>,
    orientation_code: u32,
    magnification: f32,
    translate: vec2<i32>,
};

struct Params {
    world_min: vec2<f32>,
    grid: vec2<u32>,
    bin_size: vec2<u32>,
    count: u32,
    min_width: u32,
    min_spacing: u32,
    _pad0: u32,
    _pad1: vec2<u32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> instances: array<RectInstanceT>;
@group(0) @binding(2) var<storage, read> bin_count: array<u32>;
@group(0) @binding(3) var<storage, read> bin_offset: array<u32>;
@group(0) @binding(4) var<storage, read> binned_index: array<u32>;
@group(0) @binding(5) var<storage, read_write> violation_flags: array<u32>;
@group(0) @binding(6) var<storage, read_write> heatmap: array<atomic<u32>>;

fn orient(code: u32, p: vec2<f32>) -> vec2<f32> {
    let x = p.x;
    let y = p.y;
    switch code {
        case 0u: { return vec2<f32>(x, y); }
        case 1u: { return vec2<f32>(-y, x); }
        case 2u: { return vec2<f32>(-x, -y); }
        case 3u: { return vec2<f32>(y, -x); }
        case 4u: { return vec2<f32>(x, -y); }
        case 5u: { return vec2<f32>(y, x); }
        case 6u: { return vec2<f32>(-x, y); }
        default: { return vec2<f32>(-y, -x); }
    }
}

struct Aabb {
    lo: vec2<f32>,
    hi: vec2<f32>,
};

fn world_aabb(inst: RectInstanceT) -> Aabb {
    let tr = vec2<f32>(f32(inst.translate.x), f32(inst.translate.y));
    let m = inst.magnification;
    let c0 = orient(inst.orientation_code, inst.min_xy) * m + tr;
    let c1 = orient(inst.orientation_code, vec2<f32>(inst.max_xy.x, inst.min_xy.y)) * m + tr;
    let c2 = orient(inst.orientation_code, vec2<f32>(inst.min_xy.x, inst.max_xy.y)) * m + tr;
    let c3 = orient(inst.orientation_code, inst.max_xy) * m + tr;
    var out: Aabb;
    out.lo = min(min(c0, c1), min(c2, c3));
    out.hi = max(max(c0, c1), max(c2, c3));
    return out;
}

fn bin_coord(box: Aabb) -> vec2<u32> {
    let rel = box.lo - params.world_min;
    let xi = u32(max(0.0, floor(rel.x)));
    let yi = u32(max(0.0, floor(rel.y)));
    let bx = min(xi / params.bin_size.x, params.grid.x - 1u);
    let by = min(yi / params.bin_size.y, params.grid.y - 1u);
    return vec2<u32>(bx, by);
}

// Signed separation of two closed intervals: a positive gap when disjoint, 0 when they
// touch, and minus the overlap length when they overlap. Mirrors `interval_gap`.
fn interval_gap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if (b0 > a1) {
        return b0 - a1;
    } else if (a0 > b1) {
        return a0 - b1;
    } else {
        return -(min(a1, b1) - max(a0, b0));
    }
}

// Floor integer square root by Newton's method, matching `reticle-drc`'s `isqrt`, so
// the diagonal corner gap rounds down identically on GPU and CPU.
fn isqrt(n: u32) -> u32 {
    if (n < 2u) {
        return n;
    }
    var x: u32 = n;
    var y: u32 = (x + 1u) / 2u;
    for (var guard: u32 = 0u; guard < 64u; guard = guard + 1u) {
        if (y >= x) {
            break;
        }
        x = y;
        y = (x + n / x) / 2u;
    }
    return x;
}

// The edge-to-edge gap of two world boxes in DBU, floored on the diagonal, but capped
// at `min_spacing` because the caller only asks whether the gap is *below* the rule.
// Returns 0 when they overlap or touch. Mirrors `rect_gap` in `reticle-drc`, and the
// cap keeps `dx*dx + dy*dy` within `u32` even for distant neighbours in adjacent bins
// (a gap at or above the rule is never a violation, so its exact value is irrelevant).
fn rect_gap_capped(a: Aabb, b: Aabb) -> u32 {
    let gx = interval_gap(a.lo.x, a.hi.x, b.lo.x, b.hi.x);
    let gy = interval_gap(a.lo.y, a.hi.y, b.lo.y, b.hi.y);
    let dx = max(gx, 0.0);
    let dy = max(gy, 0.0);
    if (dx == 0.0 && dy == 0.0) {
        return 0u; // overlapping or touching
    }
    let rf = f32(params.min_spacing);
    if (dx >= rf || dy >= rf) {
        return params.min_spacing; // an axis gap alone already meets the rule
    }
    let ux = u32(dx);
    let uy = u32(dy);
    return isqrt(ux * ux + uy * uy);
}

@compute @workgroup_size(256)
fn check(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) {
        return;
    }

    let me = instances[i];
    let box = world_aabb(me);
    var flag: u32 = 0u;

    // min-width: the smaller side of the world box below the rule.
    let w = box.hi.x - box.lo.x;
    let h = box.hi.y - box.lo.y;
    if (min(w, h) < f32(params.min_width)) {
        flag = 1u;
    }

    // min-spacing: scan the 3x3 bin neighbourhood for a too-close neighbour.
    if (flag == 0u && params.min_spacing > 0u) {
        let bc = bin_coord(box);
        let bx = i32(bc.x);
        let by = i32(bc.y);
        for (var oy: i32 = -1; oy <= 1; oy = oy + 1) {
            for (var ox: i32 = -1; ox <= 1; ox = ox + 1) {
                let nx = bx + ox;
                let ny = by + oy;
                if (nx < 0 || ny < 0 || nx >= i32(params.grid.x) || ny >= i32(params.grid.y)) {
                    continue;
                }
                let nb = u32(ny) * params.grid.x + u32(nx);
                let start = bin_offset[nb];
                let end = start + bin_count[nb];
                for (var s: u32 = start; s < end; s = s + 1u) {
                    let j = binned_index[s];
                    if (j == i) {
                        continue;
                    }
                    let other = world_aabb(instances[j]);
                    let gap = rect_gap_capped(box, other);
                    if (gap > 0u && gap < params.min_spacing) {
                        flag = 1u;
                    }
                }
            }
        }
    }

    violation_flags[i] = flag;
    if (flag == 1u) {
        let bc = bin_coord(box);
        let b = bc.y * params.grid.x + bc.x;
        atomicAdd(&heatmap[b], 1u);
    }
}

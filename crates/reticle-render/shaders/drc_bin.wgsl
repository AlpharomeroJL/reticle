// GPU binning of retained rect instances into a uniform grid, the first half of the
// compute-shader DRC heatmap.
//
// Three entry points run as three passes over the same bindings, a textbook
// counting sort by grid cell:
//
//   1. `count`   - one thread per instance; compute the instance's grid cell from the
//                  min corner of its world-space bounding box and `atomicAdd` a per-bin
//                  counter.
//   2. `scan`    - a single 256-thread workgroup runs an exclusive prefix scan over the
//                  per-bin counts to produce the start offset of each bin's slice, and
//                  seeds a per-bin write cursor at that offset. This reuses the
//                  Hillis-Steele workgroup scan from `compact.wgsl` (the grid is capped
//                  at 256 bins so the whole scan fits one workgroup, as the DRC pipeline
//                  guarantees on the CPU side).
//   3. `scatter` - one thread per instance again; `atomicAdd(1)` on the instance's bin
//                  cursor reserves a slot and writes the instance index there, packing
//                  the per-bin instance lists densely into `binned_index`.
//
// The output (`bin_count`, `bin_offset`, `binned_index`) lets `drc_check.wgsl` walk the
// 3x3 neighbourhood of any instance's bin. The bin size is chosen on the CPU to be at
// least `max_instance_extent + min_spacing`, so two instances whose edge gap is below
// the spacing rule always land in bins at most one apart on each axis - the invariant
// that makes the 3x3 search exhaustive (see `drc_heatmap.rs`).

// A retained rect instance; must match `RectInstanceT` in `retained.rs`/`shapes.wgsl`.
struct RectInstanceT {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
    color: vec4<f32>,
    orientation_code: u32,
    magnification: f32,
    translate: vec2<i32>,
};

// Shared uniform parameters; must match `DrcParamsRaw` in `drc_heatmap.rs`.
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
@group(0) @binding(2) var<storage, read_write> bin_count: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> bin_offset: array<u32>;
@group(0) @binding(4) var<storage, read_write> bin_cursor: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> binned_index: array<u32>;

const WORKGROUP_SIZE: u32 = 256u;
const MAX_BINS: u32 = 256u;

// The image of `p` under dihedral orientation `code` (0..8). Columns match
// `orientation_matrix` in `shapes.wgsl` and `apply_instance` in `retained.rs`.
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

// World-space axis-aligned bounding box of an instance: its four local corners under
// orientation, uniform magnification, then integer translation. A rectangle stays
// axis-aligned under any dihedral orientation, so this box is exact.
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

// The linear bin index of an instance, from the min corner of its world AABB. Uses
// exact integer division (coordinates are exact integers below 2^24, see the crate
// docs), so there is no float-boundary rounding in the bin assignment.
fn bin_index_of(inst: RectInstanceT) -> u32 {
    let box = world_aabb(inst);
    let rel = box.lo - params.world_min;
    let xi = u32(max(0.0, floor(rel.x)));
    let yi = u32(max(0.0, floor(rel.y)));
    let bx = min(xi / params.bin_size.x, params.grid.x - 1u);
    let by = min(yi / params.bin_size.y, params.grid.y - 1u);
    return by * params.grid.x + bx;
}

@compute @workgroup_size(256)
fn count(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) {
        return;
    }
    let b = bin_index_of(instances[i]);
    atomicAdd(&bin_count[b], 1u);
}

// Scratch for the prefix scan: one slot per bin.
var<workgroup> scan_buf: array<u32, MAX_BINS>;

@compute @workgroup_size(256)
fn scan(@builtin(local_invocation_id) lid: vec3<u32>) {
    let i = lid.x;
    let n = params.grid.x * params.grid.y;

    var v: u32 = 0u;
    if (i < n) {
        v = atomicLoad(&bin_count[i]);
    }
    scan_buf[i] = v;
    workgroupBarrier();

    // Inclusive Hillis-Steele scan over the (<= 256) bins, mirroring `compact.wgsl`.
    var offset: u32 = 1u;
    for (; offset < MAX_BINS; offset = offset << 1u) {
        var add: u32 = 0u;
        if (i >= offset) {
            add = scan_buf[i - offset];
        }
        workgroupBarrier();
        scan_buf[i] = scan_buf[i] + add;
        workgroupBarrier();
    }

    // Exclusive prefix = inclusive minus own value: the start offset of bin i's slice.
    let excl = scan_buf[i] - v;
    if (i < n) {
        bin_offset[i] = excl;
        atomicStore(&bin_cursor[i], excl);
    }
}

@compute @workgroup_size(256)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) {
        return;
    }
    let b = bin_index_of(instances[i]);
    let slot = atomicAdd(&bin_cursor[b], 1u);
    binned_index[slot] = i;
}

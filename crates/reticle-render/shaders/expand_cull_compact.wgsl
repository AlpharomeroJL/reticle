// Fully GPU-resident hierarchy expansion, culling, and compaction.
//
// This fuses the three stages a GPU-driven arrayed hierarchy needs into one compute
// pass, so the CPU never touches the per-placement draw list in the steady state:
//
//   1. EXPAND. The scene is uploaded as a compact table of *array placements* (one
//      record per array reference, not per element) plus a table of leaf cells. One
//      thread is launched per array *element* over a bounded chunk of the global
//      element space; the thread binary-searches the placement table by a precomputed
//      cumulative element offset to find which array it belongs to and its (row, col).
//   2. CULL. The thread composes the element's placement transform (the array's base
//      transform plus the row/col pitch offset) exactly as the CPU retained path does,
//      transforms the leaf rect's bounding box, and tests it against the viewport with
//      the same half-open rule as `cull.wgsl`.
//   3. COMPACT. Survivors are stream-compacted with the same per-workgroup exclusive
//      prefix scan + single atomic range reservation as `compact.wgsl`, but the
//      scattered payload is the full expanded `RectInstanceT` (ready to draw) rather
//      than an index. Each surviving element also bumps the chunk's indirect
//      `instance_count`, so one `draw_indirect` per chunk draws exactly the survivors.
//
// The dispatch is bounded to one CHUNK of the element space at a time (see the Rust
// side): the single-dispatch cap (65,535 workgroups and the 128 MiB storage-binding
// limit) is escaped by chunk COUNT, not by a bigger dispatch. `chunk_base` shifts the
// element window; the compacted output and cursor are per-chunk.

// One array placement: a leaf cell reference, the array's base placement transform,
// and the array geometry (columns/rows and their pitches). `element_offset` is the
// exclusive prefix sum of `columns * rows` over all earlier placements, so the global
// element index space is contiguous and searchable.
struct ArrayPlacement {
    translate: vec2<i32>,      // base transform integer translation (DBU)
    cell_index: u32,           // index into `cells`
    orientation_code: u32,     // base dihedral orientation, 0..8
    magnification: f32,        // base uniform magnification
    columns: u32,              // array columns (>= 1)
    rows: u32,                 // array rows (>= 1)
    col_pitch: i32,            // per-column step in the arrayed cell's local frame
    row_pitch: i32,            // per-row step
    element_offset: u32,       // exclusive prefix of element counts
    element_count: u32,        // columns * rows (precomputed, avoids overflow retest)
    _pad: u32,
};

// One leaf cell's geometry: a single local-space rectangle. Matches `RectInstance`.
struct GpuCell {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
    color: vec4<f32>,
};

// The expanded, ready-to-draw instance. Byte-identical to Rust `RectInstanceT` and the
// retained vertex layout, so the compacted buffer feeds `draw_indirect` on the retained
// rect pipeline with no repack.
struct RectInstanceT {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
    color: vec4<f32>,
    orientation_code: u32,
    magnification: f32,
    translate: vec2<i32>,
};

// Non-indexed indirect draw arguments, matching `wgpu::util::DrawIndirectArgs`.
// `instance_count` is atomic so every workgroup adds its survivor total to it.
struct DrawIndirect {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
};

struct Params {
    view_min: vec2<f32>,   // viewport rectangle (DBU)
    view_max: vec2<f32>,
    chunk_base: u32,       // first global element index this dispatch covers
    chunk_count: u32,      // number of elements in this chunk (<= dispatch/binding cap)
    placement_count: u32,  // valid entries in `placements`
    _pad: u32,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var<storage, read> placements: array<ArrayPlacement>;

@group(0) @binding(2)
var<storage, read> cells: array<GpuCell>;

@group(0) @binding(3)
var<storage, read_write> compacted: array<RectInstanceT>;

@group(0) @binding(4)
var<storage, read_write> cursor: atomic<u32>;

@group(0) @binding(5)
var<storage, read_write> draw_args: DrawIndirect;

const WORKGROUP_SIZE: u32 = 256u;

var<workgroup> scan: array<u32, WORKGROUP_SIZE>;
var<workgroup> base: u32;

// Reconstructs the 2x2 linear part of a dihedral orientation from its 0..8 code,
// identical to `orientation_matrix` in `shapes.wgsl`.
fn orientation_matrix(code: u32) -> mat2x2<f32> {
    switch code {
        case 0u:  { return mat2x2<f32>( 1.0,  0.0,  0.0,  1.0); }
        case 1u:  { return mat2x2<f32>( 0.0,  1.0, -1.0,  0.0); }
        case 2u:  { return mat2x2<f32>(-1.0,  0.0,  0.0, -1.0); }
        case 3u:  { return mat2x2<f32>( 0.0, -1.0,  1.0,  0.0); }
        case 4u:  { return mat2x2<f32>( 1.0,  0.0,  0.0, -1.0); }
        case 5u:  { return mat2x2<f32>( 0.0,  1.0,  1.0,  0.0); }
        case 6u:  { return mat2x2<f32>(-1.0,  0.0,  0.0,  1.0); }
        default:  { return mat2x2<f32>( 0.0, -1.0, -1.0,  0.0); }
    }
}

// Largest placement index p with placements[p].element_offset <= e. The offsets are
// ascending, so this is an upper-bound - 1; runs in O(log placement_count).
fn find_placement(e: u32) -> u32 {
    var lo: u32 = 0u;
    var hi: u32 = params.placement_count; // exclusive
    // Invariant: answer in [lo, hi). Narrow to the last offset that is <= e.
    loop {
        if (hi - lo <= 1u) { break; }
        let mid = lo + (hi - lo) / 2u;
        if (placements[mid].element_offset <= e) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return lo;
}

// Builds the expanded instance for element (col, row) of `p`. The math mirrors the CPU
// retained expansion exactly: placing point x maps to base.apply(x) + O*mag*d, where
// O and mag are the base orientation/magnification and d = (col*col_pitch,
// row*row_pitch). So the emitted instance keeps the base orientation/magnification and
// shifts the base translation by the oriented, scaled pitch offset.
fn expand(p: ArrayPlacement, cell: GpuCell, col: u32, row: u32) -> RectInstanceT {
    let d = vec2<f32>(
        f32(i32(col) * p.col_pitch),
        f32(i32(row) * p.row_pitch),
    );
    let shifted = orientation_matrix(p.orientation_code) * (p.magnification * d);
    // Round to the nearest integer DBU (exact when magnification is integral, which is
    // the common case; the CPU reference rounds the same way).
    let delta = vec2<i32>(round(shifted));
    var inst: RectInstanceT;
    inst.min_xy = cell.min_xy;
    inst.max_xy = cell.max_xy;
    inst.color = cell.color;
    inst.orientation_code = p.orientation_code;
    inst.magnification = p.magnification;
    inst.translate = p.translate + delta;
    return inst;
}

// The world-space AABB of an expanded instance, applying its transform to the rect's
// corners the same way `vs_rect_retained` does.
fn instance_bbox(inst: RectInstanceT) -> vec4<f32> {
    let m = orientation_matrix(inst.orientation_code);
    let t = vec2<f32>(inst.translate);
    let c0 = m * inst.min_xy * inst.magnification + t;
    let c1 = m * inst.max_xy * inst.magnification + t;
    let lo = min(c0, c1);
    let hi = max(c0, c1);
    return vec4<f32>(lo, hi);
}

// Half-open overlap against the viewport, matching `Rect::intersects` and `cull.wgsl`.
fn visible(bbox: vec4<f32>) -> bool {
    return bbox.x < params.view_max.x
        && params.view_min.x < bbox.z
        && bbox.y < params.view_max.y
        && params.view_min.y < bbox.w;
}

@compute @workgroup_size(256)
fn expand_cull_compact(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let local = lid.x;
    let element = params.chunk_base + gid.x;

    // Expand + cull this element. Out-of-chunk and empty-scene threads contribute 0.
    var keep: u32 = 0u;
    var inst: RectInstanceT;
    if (gid.x < params.chunk_count && params.placement_count > 0u) {
        let p = placements[find_placement(element)];
        let localidx = element - p.element_offset;
        // Guard against a ragged final placement (element beyond its count): drop it.
        if (localidx < p.element_count && p.columns > 0u) {
            let col = localidx % p.columns;
            let row = localidx / p.columns;
            inst = expand(p, cells[p.cell_index], col, row);
            if (visible(instance_bbox(inst))) {
                keep = 1u;
            }
        }
    }

    // Per-workgroup inclusive Hillis-Steele scan of the keep flags.
    scan[local] = keep;
    workgroupBarrier();
    var offset: u32 = 1u;
    for (; offset < WORKGROUP_SIZE; offset = offset << 1u) {
        var add: u32 = 0u;
        if (local >= offset) {
            add = scan[local - offset];
        }
        workgroupBarrier();
        scan[local] = scan[local] + add;
        workgroupBarrier();
    }

    let total = scan[WORKGROUP_SIZE - 1u];
    let exclusive = scan[local] - keep;

    // One thread reserves the output range and bumps the draw's instance_count.
    if (local == 0u) {
        var reserved: u32 = 0u;
        if (total > 0u) {
            reserved = atomicAdd(&cursor, total);
            let _ignored = atomicAdd(&draw_args.instance_count, total);
        }
        base = reserved;
    }
    workgroupBarrier();

    // Each surviving thread scatters its expanded instance into the reserved block.
    if (keep == 1u) {
        let out = base + exclusive;
        if (out < arrayLength(&compacted)) {
            compacted[out] = inst;
        }
    }
}

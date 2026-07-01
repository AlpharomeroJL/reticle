// GPU-driven cell culling.
//
// One invocation per candidate cell bounding box. Each box is tested for overlap
// against the viewport rectangle (both axis-aligned, in database units stored as
// floats), and a per-box visibility flag (1 = keep, 0 = cull) is written out. This
// is the first stage of a GPU-driven draw pipeline: a later pass would compact the
// kept boxes into an indirect draw list. Keeping it a plain flag buffer makes the
// stage trivially verifiable from the CPU.

struct Aabb {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
};

struct Params {
    // Viewport rectangle (min, max) in DBU.
    view_min: vec2<f32>,
    view_max: vec2<f32>,
    // Number of valid entries in `boxes`.
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var<storage, read> boxes: array<Aabb>;

@group(0) @binding(2)
var<storage, read_write> visible: array<u32>;

// Half-open overlap test matching `Rect::intersects` on the CPU: touching edges do
// not count as visible.
fn overlaps(box: Aabb) -> bool {
    return box.min_xy.x < params.view_max.x
        && params.view_min.x < box.max_xy.x
        && box.min_xy.y < params.view_max.y
        && params.view_min.y < box.max_xy.y;
}

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if (index >= params.count) {
        return;
    }
    if (overlaps(boxes[index])) {
        visible[index] = 1u;
    } else {
        visible[index] = 0u;
    }
}

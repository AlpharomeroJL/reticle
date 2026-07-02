// Indirect instanced-rectangle draw for the GPU-driven path.
//
// This is the draw stage that consumes the output of `compact.wgsl`: a dense buffer
// of surviving instance indices plus a `DrawIndexedIndirectArgs` whose `instance_count`
// the compaction filled. Rather than the CPU issuing one instanced draw of a known
// count, the GPU decides the count, and this shader gathers each surviving rectangle
// from a storage array by its compacted index.
//
// Per-instance input is a single `u32`: the compacted index (from the instance-step
// vertex buffer). The full rectangle instances live in a storage array bound at
// group 1. The quad is drawn indexed (four corner vertices, six indices), so it can be
// issued with `draw_indexed_indirect`.
//
// The math mirrors `vs_rect_retained` in `shapes.wgsl` exactly (orient, then scale,
// then translate), so an indirect draw of a set of instances is pixel-equivalent to a
// direct draw of the same set.

struct View {
    clip_from_world: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

// One retained instanced rectangle: local-space corners, color, and the placement
// transform. Must match `RectInstanceT` in `shapes.wgsl` and Rust's `RectInstanceT`.
struct RectInstanceT {
    min_xy: vec2<f32>,
    max_xy: vec2<f32>,
    color: vec4<f32>,
    orientation_code: u32,
    magnification: f32,
    translate: vec2<i32>,
};

@group(1) @binding(0)
var<storage, read> instances: array<RectInstanceT>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Reconstructs the 2x2 linear part of a dihedral orientation from its 0..8 code. The
// columns are the images of (1,0) and (0,1); identical to `orientation_matrix` in
// `shapes.wgsl`.
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

// `idx` is the compacted instance index from the instance-step vertex buffer;
// `vertex_index` walks the quad's four corners via the six-index quad.
@vertex
fn vs_rect_indirect(
    @builtin(vertex_index) vertex_index: u32,
    @location(0) idx: u32,
) -> VsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let inst = instances[idx];
    let unit = corners[vertex_index];
    let local = mix(inst.min_xy, inst.max_xy, unit);
    let oriented = orientation_matrix(inst.orientation_code) * local;
    let placed = oriented * inst.magnification + vec2<f32>(inst.translate);
    var out: VsOut;
    out.clip_position = view.clip_from_world * vec4<f32>(placed, 0.0, 1.0);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}

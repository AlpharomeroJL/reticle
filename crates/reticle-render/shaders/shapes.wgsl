// Shaders for the Reticle offscreen renderer.
//
// Two entry-point pairs share one uniform (`view`): a world->clip orthographic
// matrix derived from the camera. `vs_rect`/`fs_solid` draw axis-aligned
// rectangles as instanced unit quads; `vs_mesh`/`fs_solid` draw pre-tessellated
// polygon and path geometry from a vertex/index buffer. Colors are linear RGBA
// carried per instance or per vertex, so no per-draw layer state is needed.

struct View {
    // Column-major world-space-DBU -> clip-space transform.
    clip_from_world: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> view: View;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Per-instance data for an axis-aligned rectangle: world-space min/max corners
// (in DBU, as floats) and a linear RGBA fill color.
struct RectInstance {
    @location(0) min_xy: vec2<f32>,
    @location(1) max_xy: vec2<f32>,
    @location(2) color: vec4<f32>,
};

// The unit quad expanded per instance. `vertex_index` walks a two-triangle strip
// of the corners (0,0)-(1,0)-(0,1)-(1,1).
@vertex
fn vs_rect(@builtin(vertex_index) vertex_index: u32, inst: RectInstance) -> VsOut {
    // Corners of the unit quad as a triangle strip.
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let unit = corners[vertex_index];
    let world = mix(inst.min_xy, inst.max_xy, unit);
    var out: VsOut;
    out.clip_position = view.clip_from_world * vec4<f32>(world, 0.0, 1.0);
    out.color = inst.color;
    return out;
}

// Per-vertex data for tessellated meshes: world-space position and linear RGBA.
struct MeshVertex {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_mesh(vert: MeshVertex) -> VsOut {
    var out: VsOut;
    out.clip_position = view.clip_from_world * vec4<f32>(vert.position, 0.0, 1.0);
    out.color = vert.color;
    return out;
}

@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}

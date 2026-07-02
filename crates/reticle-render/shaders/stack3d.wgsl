// Shaders for the 3D layer-stack (extruded prism) view.
//
// Two entry-point pairs. `vs_prism`/`fs_prism` draw the extruded scene mesh:
// vertices carry a world position (layout x/y in DBU, z from the technology layer
// stack), a face normal, and a linear RGBA color; the fragment stage applies a
// simple two-term Lambert shade so slab tops and side walls read distinctly.
// `vs_blit`/`fs_blit` copy a previously rendered 3D frame (a fullscreen triangle
// sampling `blit_texture`) into whatever pass is currently active; the app uses
// this to present the depth-tested offscreen 3D image inside egui's render pass,
// which has no depth attachment of its own.

struct View3d {
    // Column-major world -> clip transform (perspective orbit camera).
    view_proj: mat4x4<f32>,
    // Directional light, world space, w unused. Normalized in the shader.
    light_dir: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view3d: View3d;

struct PrismVertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct PrismOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_prism(v: PrismVertex) -> PrismOut {
    var out: PrismOut;
    out.clip_position = view3d.view_proj * vec4<f32>(v.position, 1.0);
    out.normal = v.normal;
    out.color = v.color;
    return out;
}

@fragment
fn fs_prism(in: PrismOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(view3d.light_dir.xyz);
    // Ambient floor of 0.4 plus 0.6 of the Lambert term: faces away from the
    // light stay readable instead of going black.
    let shade = 0.4 + 0.6 * max(dot(n, l), 0.0);
    return vec4<f32>(in.color.rgb * shade, in.color.a);
}

// --- Blit: present an already rendered 3D frame as a textured triangle. ---

@group(0) @binding(1)
var blit_texture: texture_2d<f32>;

@group(0) @binding(2)
var blit_sampler: sampler;

struct BlitOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the whole viewport; parts outside clip space are clipped.
@vertex
fn vs_blit(@builtin(vertex_index) vertex_index: u32) -> BlitOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = positions[vertex_index];
    var out: BlitOut;
    out.clip_position = vec4<f32>(p, 0.0, 1.0);
    // Clip y points up, texture v points down.
    out.uv = vec2<f32>(0.5 * (p.x + 1.0), 0.5 * (1.0 - p.y));
    return out;
}

@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(blit_texture, blit_sampler, in.uv);
}

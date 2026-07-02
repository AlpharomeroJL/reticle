//! The 3D layer-stack view: extrusion, orbit camera, and render pipeline.
//!
//! This module turns a cell's flattened 2D shapes into extruded prisms and draws
//! them with a perspective orbit camera and a depth buffer. Each layer becomes a
//! z slab: layers with a `stack` directive in the technology use their physical
//! `z_bottom`/`thickness` (nanometers, converted into DBU-equivalent world units),
//! and layers without stack data fall back to synthetic uniform slabs stacked in
//! layer-id order so any document renders sensibly.
//!
//! The CPU side ([`layer_spans`], [`Mesh3d`]) is window- and GPU-free and unit
//! tested; the GPU side compiles `stack3d.wgsl` into two pipelines:
//!
//! - [`StackRenderer`]: the depth-tested prism pass, rendering into any
//!   [`TARGET_FORMAT`] color view plus an internally managed depth buffer.
//! - [`BlitPipeline`]: presents an already rendered 3D frame into another pass
//!   (the app blits into egui's render pass, which has no depth attachment).
//!
//! [`render_stack_offscreen`] mirrors the 2D
//! [`WgpuRenderer::render_document_offscreen`](crate::WgpuRenderer::render_document_offscreen)
//! entry point: one call from a document to RGBA bytes, headless-testable.

use crate::context::WgpuContext;
use crate::palette::{Palette, Rgba};
use crate::target::{OffscreenTarget, TARGET_FORMAT};
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use lyon::math::{Point as LyonPoint, point};
use lyon::path::polygon::Polygon as LyonPolygon;
use lyon::path::{LineCap, Path as LyonPath};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use reticle_geometry::{Endcap, LayerId, Point, Rect, Shape};
use reticle_model::{Document, DrawShape, ShapeKind, Technology};
use std::collections::HashMap;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BlendState, BufferBindingType, BufferUsages, Color,
    ColorTargetState, ColorWrites, CommandEncoder, CompareFunction, DepthBiasState,
    DepthStencilState, Device, Extent3d, FilterMode, FragmentState, IndexFormat, LoadOp,
    MultisampleState, Operations, PipelineLayoutDescriptor, PrimitiveState, PrimitiveTopology,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, StencilState, StoreOp, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    VertexBufferLayout, VertexState, VertexStepMode,
};

/// The depth buffer format used by the 3D pass.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

/// The directional light for the prism shade, world space. `(0, -0.6, 0.8)` is
/// unit length, so the top-face Lambert term is exactly `0.8` and the shade
/// `0.4 + 0.6 * 0.8 = 0.88`; the golden test relies on that.
pub const LIGHT_DIR: [f32; 3] = [0.0, -0.6, 0.8];

/// Synthetic slabs are `max(extent / SYNTHETIC_DIVISOR, 1)` world units thick,
/// where `extent` is the larger horizontal side of the scene bounding box.
const SYNTHETIC_DIVISOR: f32 = 16.0;

/// The vertical field of view of the orbit camera, radians.
const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;

/// Per-vertex data for the extruded mesh: world position (layout x/y in DBU, z in
/// DBU-equivalent stack units), face normal, and linear RGBA color. Matches
/// `PrismVertex` in `stack3d.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Vertex3d {
    /// World-space position `(x, y, z)`.
    pub position: [f32; 3],
    /// Unit face normal.
    pub normal: [f32; 3],
    /// Linear RGBA color (alpha straight from the layer table).
    pub color: [f32; 4],
}

/// The z slab one layer occupies, in DBU-equivalent world units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerSpan {
    /// The layer this slab belongs to.
    pub layer: LayerId,
    /// Bottom of the slab.
    pub z_bottom: f32,
    /// Top of the slab; always greater than `z_bottom` for drawable spans.
    pub z_top: f32,
}

/// Computes the z slab for every distinct layer present in `shapes`.
///
/// Layers with a technology `stack` entry use their physical position: nanometers
/// are converted to DBU-equivalent units via `dbu_per_micron` (1000 nm = 1 um =
/// `dbu_per_micron` DBU), so a 1000-DBU-per-micron technology maps 1 nm to 1 unit.
/// Layers without stack data get synthetic uniform slabs stacked upward from z = 0
/// in layer-id order, each `max(extent / 16, 1)` thick where `extent` is the larger
/// side of the scene bounding box; that keeps the view proportioned for any
/// document. A non-positive `dbu_per_micron` falls back to 1000.
#[must_use]
pub fn layer_spans(tech: &Technology, shapes: &[DrawShape]) -> Vec<LayerSpan> {
    let mut layers: Vec<LayerId> = Vec::new();
    for shape in shapes {
        if !layers.contains(&shape.layer) {
            layers.push(shape.layer);
        }
    }
    layers.sort_unstable();

    let dpm = if tech.dbu_per_micron > 0 {
        tech.dbu_per_micron
    } else {
        1000
    };
    let nm_to_world = dpm as f32 / 1000.0;
    let thickness = synthetic_thickness(shapes);

    let mut out = Vec::with_capacity(layers.len());
    let mut next_bottom = 0.0f32;
    for layer in layers {
        if let Some(entry) = tech.stack_for(layer) {
            out.push(LayerSpan {
                layer,
                z_bottom: entry.z_bottom_nm as f32 * nm_to_world,
                z_top: entry.z_top_nm() as f32 * nm_to_world,
            });
        } else {
            out.push(LayerSpan {
                layer,
                z_bottom: next_bottom,
                z_top: next_bottom + thickness,
            });
            next_bottom += thickness;
        }
    }
    out
}

/// The synthetic slab thickness for `shapes`: the larger horizontal extent of the
/// scene divided by [`SYNTHETIC_DIVISOR`], at least 1 world unit.
fn synthetic_thickness(shapes: &[DrawShape]) -> f32 {
    let bbox = shapes
        .iter()
        .map(Shape::bounding_box)
        .reduce(|a, b| a.union(&b));
    let extent = bbox.map_or(0.0, |b| {
        let w = b.width() as f32;
        let h = b.height() as f32;
        w.max(h)
    });
    (extent / SYNTHETIC_DIVISOR).max(1.0)
}

/// A tessellated 2D face: the flat top/bottom cap of one prism, plus enough
/// structure to recover its outline for the side walls.
struct Face2d {
    /// Distinct 2D vertex positions.
    vertices: Vec<[f32; 2]>,
    /// Triangle-list indices into `vertices`, normalized to CCW winding.
    indices: Vec<u32>,
}

/// The CPU-built extruded scene: an indexed triangle mesh of layer prisms,
/// ordered bottom slab first so alpha blending composites correctly.
#[derive(Clone, Default, Debug)]
pub struct Mesh3d {
    /// Vertex data.
    pub vertices: Vec<Vertex3d>,
    /// Triangle-list indices into `vertices`.
    pub indices: Vec<u32>,
}

impl Mesh3d {
    /// Returns `true` if there is nothing to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The axis-aligned bounds of the mesh as `(min, max)`, or `None` if empty.
    #[must_use]
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = self.vertices.first()?;
        let mut min = first.position;
        let mut max = first.position;
        for v in &self.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(v.position[axis]);
                max[axis] = max[axis].max(v.position[axis]);
            }
        }
        Some((min, max))
    }

    /// Extrudes `shapes` into prisms using the z slab of each shape's layer.
    ///
    /// Shapes on invisible layers (per `palette`) or on layers absent from
    /// `spans` are skipped; degenerate spans (`z_top <= z_bottom`) and geometry
    /// `lyon` cannot tessellate are dropped rather than aborting. Shapes are
    /// emitted lower slab first so translucent layers blend bottom-up.
    #[must_use]
    pub fn build(shapes: &[DrawShape], spans: &[LayerSpan], palette: &Palette) -> Self {
        let by_layer: HashMap<LayerId, (f32, f32)> = spans
            .iter()
            .map(|s| (s.layer, (s.z_bottom, s.z_top)))
            .collect();

        let mut order: Vec<usize> = (0..shapes.len())
            .filter(|&i| {
                palette.is_visible(shapes[i].layer) && by_layer.contains_key(&shapes[i].layer)
            })
            .collect();
        // Stable sort keeps document order within a slab.
        order.sort_by(|&a, &b| {
            let za = by_layer[&shapes[a].layer].0;
            let zb = by_layer[&shapes[b].layer].0;
            za.total_cmp(&zb)
        });

        let mut fill = FillTessellator::new();
        let mut stroke = StrokeTessellator::new();
        let mut out = Self::default();
        for i in order {
            let shape = &shapes[i];
            let (z_bottom, z_top) = by_layer[&shape.layer];
            if z_top <= z_bottom {
                continue;
            }
            let color = palette.color(shape.layer).components;
            let face = match &shape.kind {
                ShapeKind::Rect(rect) => rect_face(*rect),
                ShapeKind::Polygon(poly) => polygon_face(poly.vertices(), &mut fill),
                ShapeKind::Path(path) => {
                    path_face(path.points(), path.width(), path.endcap(), &mut stroke)
                }
            };
            if let Some(face) = face {
                out.push_prism(&face, z_bottom, z_top, color);
            }
        }
        out
    }

    /// Emits one prism: the face as a top cap at `z_top` (CCW seen from above),
    /// the reversed face as a bottom cap at `z_bottom`, and a side quad per
    /// boundary edge of the face.
    fn push_prism(&mut self, face: &Face2d, z_bottom: f32, z_top: f32, color: [f32; 4]) {
        // Top cap.
        let top_base = self.vertices.len() as u32;
        for &[x, y] in &face.vertices {
            self.vertices.push(Vertex3d {
                position: [x, y, z_top],
                normal: [0.0, 0.0, 1.0],
                color,
            });
        }
        self.indices
            .extend(face.indices.iter().map(|i| top_base + i));

        // Bottom cap, winding reversed so it faces down.
        let bottom_base = self.vertices.len() as u32;
        for &[x, y] in &face.vertices {
            self.vertices.push(Vertex3d {
                position: [x, y, z_bottom],
                normal: [0.0, 0.0, -1.0],
                color,
            });
        }
        for tri in face.indices.chunks_exact(3) {
            self.indices.push(bottom_base + tri[0]);
            self.indices.push(bottom_base + tri[2]);
            self.indices.push(bottom_base + tri[1]);
        }

        // Side walls from the face outline.
        for (a, b) in boundary_edges(face) {
            self.push_side(a, b, z_bottom, z_top, color);
        }
    }

    /// Emits one side quad for the directed outline edge `a -> b`.
    ///
    /// Outline edges run CCW around the face (seen from above), so the outward
    /// normal is the edge direction rotated a quarter turn clockwise:
    /// `(dy, -dx)`. The quad is wound CCW as seen from outside.
    fn push_side(&mut self, a: [f32; 2], b: [f32; 2], z_bottom: f32, z_top: f32, color: [f32; 4]) {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return;
        }
        let normal = [dy / len, -dx / len, 0.0];
        let base = self.vertices.len() as u32;
        let corners = [
            [a[0], a[1], z_bottom],
            [b[0], b[1], z_bottom],
            [b[0], b[1], z_top],
            [a[0], a[1], z_top],
        ];
        for position in corners {
            self.vertices.push(Vertex3d {
                position,
                normal,
                color,
            });
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Builds the trivial two-triangle face of an axis-aligned rectangle (no
/// tessellator involved), or `None` for an empty rectangle.
fn rect_face(rect: Rect) -> Option<Face2d> {
    if rect.is_empty() {
        return None;
    }
    let x0 = rect.min.x as f32;
    let y0 = rect.min.y as f32;
    let x1 = rect.max.x as f32;
    let y1 = rect.max.y as f32;
    Some(Face2d {
        vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        indices: vec![0, 1, 2, 0, 2, 3],
    })
}

/// Tessellates a polygon ring into a face, or `None` if degenerate.
fn polygon_face(vertices: &[Point], fill: &mut FillTessellator) -> Option<Face2d> {
    if vertices.len() < 3 {
        return None;
    }
    let points: Vec<LyonPoint> = vertices
        .iter()
        .map(|p| point(p.x as f32, p.y as f32))
        .collect();
    let polygon = LyonPolygon {
        points: &points,
        closed: true,
    };
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(&mut buffers, |v: FillVertex| {
        let pos = v.position();
        [pos.x, pos.y]
    });
    fill.tessellate_polygon(polygon, &FillOptions::default(), &mut builder)
        .ok()?;
    Some(face_from_buffers(buffers))
}

/// Tessellates a stroked path (wire) into a face, or `None` if degenerate.
fn path_face(
    points: &[Point],
    width: i32,
    endcap: Endcap,
    stroke: &mut StrokeTessellator,
) -> Option<Face2d> {
    if points.len() < 2 || width <= 0 {
        return None;
    }
    let mut builder = LyonPath::builder();
    builder.begin(point(points[0].x as f32, points[0].y as f32));
    for p in &points[1..] {
        builder.line_to(point(p.x as f32, p.y as f32));
    }
    builder.end(false);
    let path = builder.build();

    let cap = match endcap {
        Endcap::Round => LineCap::Round,
        Endcap::Square | Endcap::Custom(_) => LineCap::Square,
        Endcap::Flat => LineCap::Butt,
    };
    let options = StrokeOptions::default()
        .with_line_width(width as f32)
        .with_line_cap(cap);

    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut out = BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
        let pos = v.position();
        [pos.x, pos.y]
    });
    stroke.tessellate_path(&path, &options, &mut out).ok()?;
    Some(face_from_buffers(buffers))
}

/// Converts tessellator output into a [`Face2d`], forcing every triangle to CCW
/// winding (positive 2D signed area) so caps and outline direction are uniform
/// regardless of the tessellator's conventions.
fn face_from_buffers(buffers: VertexBuffers<[f32; 2], u32>) -> Face2d {
    let mut face = Face2d {
        vertices: buffers.vertices,
        indices: buffers.indices,
    };
    for tri in face.indices.chunks_exact_mut(3) {
        let a = face.vertices[tri[0] as usize];
        let b = face.vertices[tri[1] as usize];
        let c = face.vertices[tri[2] as usize];
        let doubled_area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        if doubled_area < 0.0 {
            tri.swap(1, 2);
        }
    }
    face
}

/// Extracts the directed boundary edges of a face: edges used by exactly one
/// triangle, in the direction the triangle uses them. With CCW triangles the
/// outer outline runs CCW (and any hole runs CW, which flips its wall normals
/// outward too). Edges are keyed by exact vertex position so duplicated vertices
/// from tessellation still pair up. Order is deterministic (first appearance).
fn boundary_edges(face: &Face2d) -> Vec<([f32; 2], [f32; 2])> {
    /// A position key using the exact f32 bit patterns.
    fn key(p: [f32; 2]) -> u64 {
        (u64::from(p[0].to_bits()) << 32) | u64::from(p[1].to_bits())
    }

    // Slot per first-seen undirected edge; second sighting kills the slot.
    let mut slots: Vec<Option<([f32; 2], [f32; 2])>> = Vec::new();
    let mut index: HashMap<(u64, u64), usize> = HashMap::new();
    for tri in face.indices.chunks_exact(3) {
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let a = face.vertices[tri[i] as usize];
            let b = face.vertices[tri[j] as usize];
            let (ka, kb) = (key(a), key(b));
            if ka == kb {
                continue;
            }
            let undirected = (ka.min(kb), ka.max(kb));
            if let Some(&slot) = index.get(&undirected) {
                slots[slot] = None;
            } else {
                index.insert(undirected, slots.len());
                slots.push(Some((a, b)));
            }
        }
    }
    slots.into_iter().flatten().collect()
}

/// A perspective orbit camera: yaw/pitch/distance about a target point, with
/// world +z up (the stack axis).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OrbitCamera {
    /// Rotation about +z, radians. Zero looks along -x toward the target.
    pub yaw: f32,
    /// Elevation above the xy plane, radians, clamped to about +/- 86 degrees so
    /// the view never degenerates at the poles.
    pub pitch: f32,
    /// Distance from the target to the eye, world units; always positive.
    pub distance: f32,
    /// The point the camera orbits and looks at.
    pub target: [f32; 3],
}

impl OrbitCamera {
    /// The pitch clamp, radians (about 86 degrees).
    pub const MAX_PITCH: f32 = 1.5;
    /// The minimum orbit distance, world units.
    pub const MIN_DISTANCE: f32 = 0.001;

    /// A camera framing `bounds` (as returned by [`Mesh3d::bounds`]): targeted at
    /// the center, backed off proportionally to the bounding radius, at a
    /// pleasant three-quarter angle.
    #[must_use]
    pub fn framing(bounds: ([f32; 3], [f32; 3])) -> Self {
        let (min, max) = bounds;
        let target = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let dx = max[0] - min[0];
        let dy = max[1] - min[1];
        let dz = max[2] - min[2];
        let radius = (0.5 * (dx * dx + dy * dy + dz * dz).sqrt()).max(1.0);
        Self {
            yaw: -1.2,
            pitch: 0.7,
            distance: radius * 2.6,
            target,
        }
    }

    /// Rotates the view by `(dyaw, dpitch)` radians, clamping the pitch.
    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-Self::MAX_PITCH, Self::MAX_PITCH);
    }

    /// Scales the orbit distance by `factor` (values below 1 zoom in), clamping
    /// at [`OrbitCamera::MIN_DISTANCE`]. Non-positive factors are ignored.
    pub fn zoom(&mut self, factor: f32) {
        if factor > 0.0 {
            self.distance = (self.distance * factor).max(Self::MIN_DISTANCE);
        }
    }

    /// The eye position implied by yaw/pitch/distance around the target.
    #[must_use]
    pub fn eye(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [
            self.target[0] + self.distance * cp * cy,
            self.target[1] + self.distance * cp * sy,
            self.target[2] + self.distance * sp,
        ]
    }

    /// The column-major world -> clip transform for an output of the given
    /// aspect ratio (width over height), using wgpu clip conventions (z in
    /// `[0, 1]`).
    #[must_use]
    pub fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let eye = glam::Vec3::from(self.eye());
        let target = glam::Vec3::from(self.target);
        let view = glam::camera::rh::view::look_at_mat4(eye, target, glam::Vec3::Z);
        let near = (self.distance * 0.01).max(1e-4);
        let far = (self.distance * 40.0).max(near * 4.0);
        let proj = glam::camera::rh::proj::directx::perspective(FOV_Y, aspect.max(0.01), near, far);
        (proj * view).to_cols_array_2d()
    }
}

/// The uniform block for the 3D pass. Matches `View3d` in `stack3d.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct StackUniform {
    /// Column-major world -> clip transform.
    pub view_proj: [[f32; 4]; 4],
    /// Directional light (xyz; w unused).
    pub light_dir: [f32; 4],
}

impl StackUniform {
    /// Builds the uniform for `camera` rendering at `aspect` with [`LIGHT_DIR`].
    #[must_use]
    pub fn new(camera: &OrbitCamera, aspect: f32) -> Self {
        Self {
            view_proj: camera.view_proj(aspect),
            light_dir: [LIGHT_DIR[0], LIGHT_DIR[1], LIGHT_DIR[2], 0.0],
        }
    }
}

/// The color/depth target size and view the 3D pass renders into.
#[derive(Clone, Copy, Debug)]
pub struct RenderTarget3d<'a> {
    /// A [`TARGET_FORMAT`] color view.
    pub view: &'a TextureView,
    /// The target size in pixels (`width`, `height`).
    pub size: (u32, u32),
}

/// A cached depth buffer, recreated when the target size changes.
struct DepthBuffer {
    view: TextureView,
    size: (u32, u32),
}

/// The depth-tested prism pipeline plus its managed depth buffer.
pub struct StackRenderer {
    pipeline: RenderPipeline,
    uniform_layout: BindGroupLayout,
    depth: Option<DepthBuffer>,
}

impl core::fmt::Debug for StackRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StackRenderer").finish_non_exhaustive()
    }
}

impl StackRenderer {
    /// Compiles `stack3d.wgsl` and builds the prism pipeline on `device`,
    /// targeting [`TARGET_FORMAT`] color and [`DEPTH_FORMAT`] depth.
    #[must_use]
    pub fn new(device: &Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/stack3d.wgsl"));

        let uniform_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render 3d view layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render 3d layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });

        // Per-vertex attributes: position (loc 0), normal (loc 1), color (loc 2).
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let vertex_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex3d>() as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: &attrs,
        };

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render 3d prism pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_prism"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                // Translucent layers: both faces of every wall stay visible.
                cull_mode: None,
                ..PrimitiveState::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_prism"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            uniform_layout,
            depth: None,
        }
    }

    /// Ensures the cached depth buffer matches `size`.
    fn ensure_depth(&mut self, device: &Device, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.depth.as_ref().is_some_and(|d| d.size == size) {
            return;
        }
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("reticle-render 3d depth"),
            size: Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(DepthBuffer {
            view: texture.create_view(&TextureViewDescriptor::default()),
            size,
        });
    }

    /// Records one 3D frame into `target` on `encoder`: clears color and depth,
    /// then draws `mesh` with `camera`. An empty mesh still clears the target.
    pub fn render(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        target: RenderTarget3d<'_>,
        mesh: &Mesh3d,
        camera: &OrbitCamera,
        clear: Rgba,
    ) {
        self.ensure_depth(device, target.size);
        let depth_view = &self.depth.as_ref().expect("ensure_depth just ran").view;

        let aspect = target.size.0.max(1) as f32 / target.size.1.max(1) as f32;
        let uniform = StackUniform::new(camera, aspect);
        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render 3d uniform"),
            contents: bytes_of(&uniform),
            usage: BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render 3d bind group"),
            layout: &self.uniform_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render 3d vertices"),
            contents: cast_slice(&mesh.vertices),
            usage: BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render 3d indices"),
            contents: cast_slice(&mesh.indices),
            usage: BufferUsages::INDEX,
        });

        let clear_color = Color {
            r: f64::from(clear.components[0]),
            g: f64::from(clear.components[1]),
            b: f64::from(clear.components[2]),
            a: f64::from(clear.components[3]),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reticle-render 3d pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(clear_color),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        let index_count = u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX);
        if index_count > 0 {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..index_count, 0, 0..1);
        }
    }
}

/// A pipeline that copies a [`TARGET_FORMAT`] texture into the currently active
/// render pass as a fullscreen triangle (constrained by the pass viewport).
pub struct BlitPipeline {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: Sampler,
}

impl core::fmt::Debug for BlitPipeline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlitPipeline").finish_non_exhaustive()
    }
}

impl BlitPipeline {
    /// Builds the blit pipeline on `device`, targeting `dst_format` (the format
    /// of the pass it will draw inside, for example egui's surface format).
    #[must_use]
    pub fn new(device: &Device, dst_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/stack3d.wgsl"));
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render 3d blit layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render 3d blit pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render 3d blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_blit"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_blit"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: dst_format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("reticle-render 3d blit sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..SamplerDescriptor::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    /// Creates the bind group sampling `source`.
    #[must_use]
    pub fn bind(&self, device: &Device, source: &TextureView) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render 3d blit bind group"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Draws the bound texture as a fullscreen triangle into `pass`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bind: &BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A cached offscreen color target for [`StackView`].
struct ColorTarget {
    view: TextureView,
    size: (u32, u32),
}

/// The app-facing composite: renders the 3D scene into an internal color+depth
/// target and presents it into another render pass (egui's) via the blitter.
///
/// Call [`StackView::prepare`] once per frame with an encoder that runs before
/// the presenting pass, then [`StackView::paint`] inside that pass. This is the
/// shape `egui-wgpu` paint callbacks want: `prepare` from the callback's prepare
/// hook, `paint` from its paint hook.
pub struct StackView {
    renderer: StackRenderer,
    blit: BlitPipeline,
    color: Option<ColorTarget>,
    bind: Option<BindGroup>,
}

impl core::fmt::Debug for StackView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StackView").finish_non_exhaustive()
    }
}

impl StackView {
    /// Builds the 3D and blit pipelines on `device`; the presenting pass writes
    /// to `dst_format` targets.
    #[must_use]
    pub fn new(device: &Device, dst_format: TextureFormat) -> Self {
        Self {
            renderer: StackRenderer::new(device),
            blit: BlitPipeline::new(device, dst_format),
            color: None,
            bind: None,
        }
    }

    /// Ensures the internal color target matches `size`, refreshing the blit
    /// bind group when recreated.
    fn ensure_color(&mut self, device: &Device, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.color.as_ref().is_some_and(|c| c.size == size) {
            return;
        }
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("reticle-render 3d color"),
            size: Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.bind = Some(self.blit.bind(device, &view));
        self.color = Some(ColorTarget { view, size });
    }

    /// Renders one 3D frame into the internal target (recorded on `encoder`).
    pub fn prepare(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        size: (u32, u32),
        mesh: &Mesh3d,
        camera: &OrbitCamera,
        clear: Rgba,
    ) {
        self.ensure_color(device, size);
        let color = self.color.as_ref().expect("ensure_color just ran");
        self.renderer.render(
            device,
            encoder,
            RenderTarget3d {
                view: &color.view,
                size: color.size,
            },
            mesh,
            camera,
            clear,
        );
    }

    /// Draws the last prepared frame into `pass`. Does nothing if
    /// [`StackView::prepare`] has never run.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(bind) = &self.bind {
            self.blit.draw(pass, bind);
        }
    }
}

/// Renders the extruded 3D stack view of `top_cell` in `doc` through `camera`
/// into an offscreen RGBA8 target of `size` pixels, returning tightly packed
/// RGBA bytes (row 0 at the top). Mirrors the 2D
/// [`WgpuRenderer::render_document_offscreen`](crate::WgpuRenderer::render_document_offscreen).
///
/// The cell is flattened, layer z slabs are resolved with [`layer_spans`], the
/// prisms are built with [`Mesh3d::build`], and the frame is drawn over
/// [`DEFAULT_CLEAR`](crate::DEFAULT_CLEAR) with a fresh pipeline (one-shot use:
/// tests, thumbnails, export).
#[must_use]
pub fn render_stack_offscreen(
    ctx: &WgpuContext,
    doc: &Document,
    top_cell: &str,
    camera: &OrbitCamera,
    size: (u32, u32),
) -> Vec<u8> {
    let (width, height) = size;
    let target = OffscreenTarget::new(ctx, width, height);

    let palette = Palette::from_technology(doc.technology());
    let shapes = doc.flatten(top_cell);
    let spans = layer_spans(doc.technology(), &shapes);
    let mesh = Mesh3d::build(&shapes, &spans, &palette);

    let mut renderer = StackRenderer::new(ctx.device());
    let mut encoder = ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("reticle-render 3d offscreen encoder"),
        });
    renderer.render(
        ctx.device(),
        &mut encoder,
        RenderTarget3d {
            view: target.view(),
            size: (target.width(), target.height()),
        },
        &mesh,
        camera,
        crate::DEFAULT_CLEAR,
    );
    target.copy_to_buffer(&mut encoder);
    ctx.queue().submit(std::iter::once(encoder.finish()));
    target.read_pixels(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Polygon;
    use reticle_model::{LayerInfo, StackEntry};

    const LAYER_A: LayerId = LayerId::new(1, 0);
    const LAYER_B: LayerId = LayerId::new(2, 0);

    fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn tech_with_stack(stack: Vec<StackEntry>) -> Technology {
        Technology {
            name: "t".to_owned(),
            dbu_per_micron: 1000,
            layers: vec![
                LayerInfo {
                    id: LAYER_A,
                    name: "A".to_owned(),
                    color_rgba: 0xff00_00ff,
                    visible: true,
                },
                LayerInfo {
                    id: LAYER_B,
                    name: "B".to_owned(),
                    color_rgba: 0x00ff_00ff,
                    visible: true,
                },
            ],
            rules: Vec::new(),
            stack,
        }
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn spans_use_stack_entries_converted_to_world_units() {
        // 1000 DBU per micron makes 1 nm exactly 1 world unit.
        let tech = tech_with_stack(vec![StackEntry {
            layer: LAYER_A,
            z_bottom_nm: 500,
            thickness_nm: 200,
        }]);
        let shapes = [rect_shape(LAYER_A, 0, 0, 1600, 100)];
        let spans = layer_spans(&tech, &shapes);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].layer, LAYER_A);
        assert!(approx(spans[0].z_bottom, 500.0));
        assert!(approx(spans[0].z_top, 700.0));
    }

    #[test]
    fn spans_synthesize_uniform_slabs_in_layer_id_order() {
        // No stack entries: both layers get synthetic slabs. The scene is
        // 1600 DBU wide, so the slab thickness is 1600 / 16 = 100 exactly.
        let tech = tech_with_stack(Vec::new());
        // Shapes listed B first: slab order must still follow layer id (A, B).
        let shapes = [
            rect_shape(LAYER_B, 0, 0, 1600, 100),
            rect_shape(LAYER_A, 0, 0, 800, 100),
        ];
        let spans = layer_spans(&tech, &shapes);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].layer, LAYER_A);
        assert!(approx(spans[0].z_bottom, 0.0));
        assert!(approx(spans[0].z_top, 100.0));
        assert_eq!(spans[1].layer, LAYER_B);
        assert!(approx(spans[1].z_bottom, 100.0));
        assert!(approx(spans[1].z_top, 200.0));
    }

    #[test]
    fn spans_mix_declared_and_synthetic_layers() {
        // A declared, B synthetic: B still stacks from z = 0 (synthetic slabs
        // are independent of declared ones).
        let tech = tech_with_stack(vec![StackEntry {
            layer: LAYER_A,
            z_bottom_nm: 5000,
            thickness_nm: 1000,
        }]);
        let shapes = [
            rect_shape(LAYER_A, 0, 0, 1600, 100),
            rect_shape(LAYER_B, 0, 0, 1600, 100),
        ];
        let spans = layer_spans(&tech, &shapes);
        assert_eq!(spans.len(), 2);
        assert!(approx(spans[0].z_bottom, 5000.0));
        assert!(approx(spans[0].z_top, 6000.0));
        assert!(approx(spans[1].z_bottom, 0.0));
        assert!(approx(spans[1].z_top, 100.0));
    }

    /// The 2D signed doubled area of a mesh triangle projected onto xy.
    fn tri_doubled_area(mesh: &Mesh3d, tri: &[u32]) -> f32 {
        let a = mesh.vertices[tri[0] as usize].position;
        let b = mesh.vertices[tri[1] as usize].position;
        let c = mesh.vertices[tri[2] as usize].position;
        (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])
    }

    #[test]
    fn one_rect_extrudes_to_a_closed_prism() {
        let tech = tech_with_stack(Vec::new());
        let shapes = [rect_shape(LAYER_A, 0, 0, 100, 50)];
        let palette = Palette::from_technology(&tech);
        let spans = vec![LayerSpan {
            layer: LAYER_A,
            z_bottom: 10.0,
            z_top: 30.0,
        }];
        let mesh = Mesh3d::build(&shapes, &spans, &palette);

        // 4 top + 4 bottom + 4 sides x 4 = 24 vertices; 2 + 2 + 8 = 12 triangles.
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);

        // Every vertex sits on one of the two slab planes.
        for v in &mesh.vertices {
            let z = v.position[2];
            assert!(
                approx(z, 10.0) || approx(z, 30.0),
                "vertex z {z} not on a slab plane"
            );
        }

        // Classify triangles by their vertex z values and check winding.
        let mut top = 0;
        let mut bottom = 0;
        let mut side = 0;
        for tri in mesh.indices.chunks_exact(3) {
            let zs: Vec<f32> = tri
                .iter()
                .map(|&i| mesh.vertices[i as usize].position[2])
                .collect();
            if zs.iter().all(|&z| approx(z, 30.0)) {
                top += 1;
                // Top cap is CCW seen from above.
                assert!(tri_doubled_area(&mesh, tri) > 0.0, "top cap must be CCW");
            } else if zs.iter().all(|&z| approx(z, 10.0)) {
                bottom += 1;
                // Bottom cap is reversed (CW from above = faces down).
                assert!(tri_doubled_area(&mesh, tri) < 0.0, "bottom cap must be CW");
            } else {
                side += 1;
            }
        }
        assert_eq!((top, bottom, side), (2, 2, 8));

        // Cap normals point straight up/down; side normals are horizontal unit
        // vectors pointing away from the rect center (50, 25).
        for v in &mesh.vertices {
            let n = v.normal;
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(approx(len, 1.0), "normal must be unit length");
            if approx(n[2], 1.0) || approx(n[2], -1.0) {
                continue; // cap
            }
            assert!(approx(n[2], 0.0), "side normals are horizontal");
            let outward = n[0] * (v.position[0] - 50.0) + n[1] * (v.position[1] - 25.0);
            assert!(outward > 0.0, "side normal must point outward");
        }
    }

    #[test]
    fn l_polygon_walls_follow_the_outline() {
        // An L shape with 6 outline edges -> 12 side triangles.
        let l = Polygon::new(vec![
            Point::new(0, 0),
            Point::new(40, 0),
            Point::new(40, 10),
            Point::new(10, 10),
            Point::new(10, 30),
            Point::new(0, 30),
        ]);
        let shapes = [DrawShape::new(LAYER_A, ShapeKind::Polygon(l))];
        let tech = tech_with_stack(Vec::new());
        let palette = Palette::from_technology(&tech);
        let spans = vec![LayerSpan {
            layer: LAYER_A,
            z_bottom: 0.0,
            z_top: 5.0,
        }];
        let mesh = Mesh3d::build(&shapes, &spans, &palette);
        assert!(!mesh.is_empty());

        let mut side = 0;
        for tri in mesh.indices.chunks_exact(3) {
            let zs: Vec<f32> = tri
                .iter()
                .map(|&i| mesh.vertices[i as usize].position[2])
                .collect();
            let all_top = zs.iter().all(|&z| approx(z, 5.0));
            let all_bottom = zs.iter().all(|&z| approx(z, 0.0));
            if all_top {
                assert!(tri_doubled_area(&mesh, tri) > 0.0, "top cap must be CCW");
            }
            if !all_top && !all_bottom {
                side += 1;
            }
        }
        assert_eq!(side, 12, "6 outline edges -> 12 side triangles");
    }

    #[test]
    fn lower_slabs_are_emitted_first_and_hidden_layers_skipped() {
        let mut tech = tech_with_stack(Vec::new());
        let shapes = [
            rect_shape(LAYER_B, 0, 0, 10, 10), // upper slab, listed first
            rect_shape(LAYER_A, 0, 0, 10, 10), // lower slab
        ];
        let spans = layer_spans(&tech, &shapes);
        let palette = Palette::from_technology(&tech);
        let mesh = Mesh3d::build(&shapes, &spans, &palette);
        // The very first emitted vertex belongs to the lower (A) slab: its top
        // cap sits at A's z_top, below B's slab entirely.
        let first_z = mesh.vertices[0].position[2];
        let a_top = spans.iter().find(|s| s.layer == LAYER_A).unwrap().z_top;
        assert!(first_z <= a_top, "lower slab must be emitted first");

        // Hiding layer B removes its prism but keeps z assignments intact.
        tech.layers[1].visible = false;
        let hidden_palette = Palette::from_technology(&tech);
        let hidden = Mesh3d::build(&shapes, &spans, &hidden_palette);
        assert!(hidden.indices.len() < mesh.indices.len());
        assert_eq!(hidden.indices.len(), 36, "one rect prism remains");
    }

    #[test]
    fn empty_scene_builds_an_empty_mesh() {
        let tech = tech_with_stack(Vec::new());
        let palette = Palette::from_technology(&tech);
        let mesh = Mesh3d::build(&[], &[], &palette);
        assert!(mesh.is_empty());
        assert!(mesh.bounds().is_none());
    }

    #[test]
    fn orbit_camera_centers_its_target() {
        let camera = OrbitCamera {
            yaw: 0.8,
            pitch: 0.6,
            distance: 500.0,
            target: [100.0, -40.0, 25.0],
        };
        let m = glam::Mat4::from_cols_array_2d(&camera.view_proj(1.5));
        let clip = m * glam::Vec4::new(100.0, -40.0, 25.0, 1.0);
        assert!(clip.w > 0.0, "target must be in front of the camera");
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4, "target centered");
        assert!(ndc.z > 0.0 && ndc.z < 1.0, "target inside the depth range");
    }

    #[test]
    fn orbit_camera_clamps_pitch_and_distance() {
        let mut camera = OrbitCamera::framing(([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]));
        camera.orbit(0.0, 100.0);
        assert!(approx(camera.pitch, OrbitCamera::MAX_PITCH));
        camera.orbit(0.0, -200.0);
        assert!(approx(camera.pitch, -OrbitCamera::MAX_PITCH));
        camera.zoom(0.0); // ignored
        let d = camera.distance;
        assert!(approx(camera.distance, d));
        for _ in 0..200 {
            camera.zoom(0.5);
        }
        assert!(camera.distance >= OrbitCamera::MIN_DISTANCE);
    }

    #[test]
    fn framing_looks_at_the_scene_center() {
        let camera = OrbitCamera::framing(([0.0, 0.0, 0.0], [100.0, 200.0, 50.0]));
        assert!(approx(camera.target[0], 50.0));
        assert!(approx(camera.target[1], 100.0));
        assert!(approx(camera.target[2], 25.0));
        assert!(camera.distance > 100.0, "backed off beyond the radius");
    }
}

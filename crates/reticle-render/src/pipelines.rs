//! Render pipelines and the offscreen draw.
//!
//! [`Pipelines`] compiles `shapes.wgsl` and builds the instanced-rectangle and
//! tessellated-mesh render pipelines, which share one uniform bind group holding the
//! camera projection. [`Pipelines::render`] uploads per-frame buffers, records a
//! clearing render pass, draws both pipelines, and copies the result into the
//! target's staging buffer.

use crate::geometry::{MeshVertex, RectInstance, SceneGeometry};
use crate::retained::RectInstanceT;
use crate::target::{OffscreenTarget, TARGET_FORMAT};
use crate::view::ViewUniform;
use crate::{context::WgpuContext, palette::Rgba};
use bytemuck::{bytes_of, cast_slice};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BlendState, BufferBindingType, BufferUsages, Color,
    ColorTargetState, ColorWrites, CommandEncoderDescriptor, Device, FragmentState, IndexFormat,
    LoadOp, MultisampleState, Operations, PipelineLayoutDescriptor, PrimitiveState,
    PrimitiveTopology, RenderPipeline, RenderPipelineDescriptor, ShaderStages, StoreOp,
    TextureFormat, VertexBufferLayout, VertexState, VertexStepMode,
};

/// The compiled pipelines and shared bind group layout for the offscreen renderer.
pub struct Pipelines {
    uniform_layout: BindGroupLayout,
    rect_pipeline: RenderPipeline,
    mesh_pipeline: RenderPipeline,
    /// The retained instanced-rect pipeline (per-instance transform applied in the
    /// vertex shader). Shares the uniform bind group layout with the others.
    retained_rect_pipeline: RenderPipeline,
    /// The color format these pipelines were built for.
    format: TextureFormat,
    /// The multisample count the pipelines were built at. The offscreen path
    /// ([`Pipelines::new`]) uses [`crate::OFFSCREEN_SAMPLE_COUNT`] when the device
    /// supports it; the surface path ([`Pipelines::for_format`]) is always 1.
    sample_count: u32,
}

impl core::fmt::Debug for Pipelines {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipelines").finish_non_exhaustive()
    }
}

impl Pipelines {
    /// Compiles the shader and builds the render pipelines on `ctx`'s device for the
    /// offscreen [`TARGET_FORMAT`], multisampled at [`crate::OFFSCREEN_SAMPLE_COUNT`]
    /// when the device supports it.
    ///
    /// The sample count is negotiated against the adapter's texture-format features so
    /// a device without 4x MSAA falls back to single-sampled pipelines. Pair the
    /// result with an [`OffscreenTarget`] built on the same `ctx`, which negotiates the
    /// identical count.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let sample_count = if crate::target::supports_4x_msaa(ctx) {
            crate::target::OFFSCREEN_SAMPLE_COUNT
        } else {
            1
        };
        Self::build(ctx.device(), TARGET_FORMAT, sample_count)
    }

    /// Compiles the shader and builds single-sampled render pipelines on `device` for
    /// the given color `format`.
    ///
    /// This is the format-parameterized entry the windowed (surface) path uses with
    /// eframe's shared device and its surface `target_format`; the offscreen path goes
    /// through [`Pipelines::new`] with [`TARGET_FORMAT`] and MSAA. The surface path
    /// composites into egui's own single-sample pass, so these pipelines stay at
    /// sample count 1.
    #[must_use]
    pub fn for_format(device: &Device, format: TextureFormat) -> Self {
        Self::build(device, format, 1)
    }

    /// Compiles the shader and builds the render pipelines on `device` for the given
    /// color `format` at `sample_count` samples per pixel.
    #[must_use]
    fn build(device: &Device, format: TextureFormat, sample_count: u32) -> Self {
        let sample_count = sample_count.max(1);
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/shapes.wgsl"));

        let uniform_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render view layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });

        // Per-instance rectangle attributes: min_xy (loc 0), max_xy (loc 1), color (loc 2).
        let rect_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let rect_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &rect_attrs,
        };

        // Retained per-instance rect attributes: the RectInstance fields plus the
        // placement transform (orientation code loc 3, magnification loc 4,
        // translate loc 5). Matches `RectInstanceT` in `shapes.wgsl`.
        let retained_attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Uint32, 4 => Float32, 5 => Sint32x2
        ];
        let retained_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<RectInstanceT>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &retained_attrs,
        };

        // Per-vertex mesh attributes: position (loc 0), color (loc 1).
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let mesh_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        };

        let targets = [Some(ColorTargetState {
            format,
            blend: Some(BlendState::ALPHA_BLENDING),
            write_mask: ColorWrites::ALL,
        })];

        let make = |label: &str, entry: &str, layout: VertexBufferLayout, strip: bool| {
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[layout],
                },
                primitive: PrimitiveState {
                    topology: if strip {
                        PrimitiveTopology::TriangleStrip
                    } else {
                        PrimitiveTopology::TriangleList
                    },
                    ..PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: MultisampleState {
                    count: sample_count,
                    ..MultisampleState::default()
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: Some("fs_solid"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let rect_pipeline = make("reticle-render rect pipeline", "vs_rect", rect_layout, true);
        let mesh_pipeline = make(
            "reticle-render mesh pipeline",
            "vs_mesh",
            mesh_layout,
            false,
        );
        let retained_rect_pipeline = make(
            "reticle-render retained rect pipeline",
            "vs_rect_retained",
            retained_layout,
            true,
        );

        Self {
            uniform_layout,
            rect_pipeline,
            mesh_pipeline,
            retained_rect_pipeline,
            format,
            sample_count,
        }
    }

    /// The multisample count these pipelines were built at.
    #[must_use]
    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The uniform bind group layout (binding 0: the view matrix), shared by every
    /// pipeline. The retained renderer builds its camera bind group against this.
    #[must_use]
    pub fn uniform_layout(&self) -> &BindGroupLayout {
        &self.uniform_layout
    }

    /// The retained instanced-rect pipeline.
    #[must_use]
    pub fn retained_rect_pipeline(&self) -> &RenderPipeline {
        &self.retained_rect_pipeline
    }

    /// The tessellated-mesh pipeline.
    #[must_use]
    pub fn mesh_pipeline(&self) -> &RenderPipeline {
        &self.mesh_pipeline
    }

    /// The color format these pipelines target.
    #[must_use]
    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// Renders `geometry` into `target` with projection `view`, clearing to `clear`.
    ///
    /// Records and submits one command buffer: a render pass that clears the target,
    /// draws all rectangles, then draws the tessellated mesh, followed by a copy of
    /// the color texture into the target's staging buffer. Use
    /// [`OffscreenTarget::read_pixels`] afterwards to get the bytes back.
    pub fn render(
        &self,
        ctx: &WgpuContext,
        target: &OffscreenTarget,
        geometry: &SceneGeometry,
        view: &ViewUniform,
        clear: Rgba,
    ) {
        let device = ctx.device();

        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render view uniform"),
            contents: bytes_of(view),
            usage: BufferUsages::UNIFORM,
        });
        let uniform_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render view bind group"),
            layout: &self.uniform_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Per-frame geometry buffers. Empty slices still yield valid (zero-sized)
        // buffers via `create_buffer_init`, so the draw calls below are guarded by
        // their counts rather than by buffer presence.
        let rect_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render rect instances"),
            contents: cast_slice(&geometry.rects),
            usage: BufferUsages::VERTEX,
        });
        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render mesh vertices"),
            contents: cast_slice(&geometry.mesh_vertices),
            usage: BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render mesh indices"),
            contents: cast_slice(&geometry.mesh_indices),
            usage: BufferUsages::INDEX,
        });

        let clear_color = Color {
            r: f64::from(clear.components[0]),
            g: f64::from(clear.components[1]),
            b: f64::from(clear.components[2]),
            a: f64::from(clear.components[3]),
        };
        // When the target is multisampled, draw into its MSAA color texture and
        // resolve into the single-sample texture readback reads; otherwise draw
        // straight into that texture. Either way `target.view()` holds the final
        // (resolved) image.
        let (attachment_view, resolve_target) = match target.msaa_view() {
            Some(msaa) => (msaa, Some(target.view())),
            None => (target.view(), None),
        };
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reticle-render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment_view,
                    depth_slice: None,
                    resolve_target,
                    ops: Operations {
                        load: LoadOp::Clear(clear_color),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_bind_group(0, &uniform_bind_group, &[]);

            let rect_count = u32::try_from(geometry.rects.len()).unwrap_or(u32::MAX);
            if rect_count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(0, rect_buffer.slice(..));
                // Four vertices per instanced unit quad (triangle strip).
                pass.draw(0..4, 0..rect_count);
            }

            let index_count = u32::try_from(geometry.mesh_indices.len()).unwrap_or(u32::MAX);
            if index_count > 0 {
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..index_count, 0, 0..1);
            }
        }

        target.copy_to_buffer(&mut encoder);
        ctx.queue().submit(std::iter::once(encoder.finish()));
    }
}

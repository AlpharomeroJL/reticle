//! Render pipelines and the offscreen draw.
//!
//! [`Pipelines`] compiles `shapes.wgsl` and builds the instanced-rectangle and
//! tessellated-mesh render pipelines, which share one uniform bind group holding the
//! camera projection. [`Pipelines::render`] uploads per-frame buffers, records a
//! clearing render pass, draws both pipelines, and copies the result into the
//! target's staging buffer.

use crate::geometry::{MeshVertex, RectInstance, SceneGeometry};
use crate::target::{OffscreenTarget, TARGET_FORMAT};
use crate::view::ViewUniform;
use crate::{context::WgpuContext, palette::Rgba};
use bytemuck::{bytes_of, cast_slice};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BlendState, BufferBindingType, BufferUsages, Color,
    ColorTargetState, ColorWrites, CommandEncoderDescriptor, FragmentState, IndexFormat, LoadOp,
    MultisampleState, Operations, PipelineLayoutDescriptor, PrimitiveState, PrimitiveTopology,
    RenderPipeline, RenderPipelineDescriptor, ShaderStages, StoreOp, VertexBufferLayout,
    VertexState, VertexStepMode,
};

/// The compiled pipelines and shared bind group layout for the offscreen renderer.
pub struct Pipelines {
    uniform_layout: BindGroupLayout,
    rect_pipeline: RenderPipeline,
    mesh_pipeline: RenderPipeline,
}

impl core::fmt::Debug for Pipelines {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipelines").finish_non_exhaustive()
    }
}

impl Pipelines {
    /// Compiles the shader and builds both render pipelines on `ctx`'s device.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = ctx.device();
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

        // Per-vertex mesh attributes: position (loc 0), color (loc 1).
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let mesh_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        };

        let targets = [Some(ColorTargetState {
            format: TARGET_FORMAT,
            blend: Some(BlendState::ALPHA_BLENDING),
            write_mask: ColorWrites::ALL,
        })];

        let rect_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_rect"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[rect_layout],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_solid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        let mesh_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_mesh"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[mesh_layout],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_solid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            uniform_layout,
            rect_pipeline,
            mesh_pipeline,
        }
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
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reticle-render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view(),
                    depth_slice: None,
                    resolve_target: None,
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

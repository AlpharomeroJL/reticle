//! Indirect instanced-rectangle drawing, fed by the compaction stage.
//!
//! [`IndirectRects`] is the draw half of the GPU-driven path. Where the retained
//! renderer issues `draw(0..4, 0..count)` with a CPU-known instance count, this issues
//! [`draw_indexed_indirect`](wgpu::RenderPass::draw_indexed_indirect) against the
//! [`DrawIndexedIndirectArgs`](wgpu::util::DrawIndexedIndirectArgs) that
//! [`CellCompactor`](crate::CellCompactor) filled, so the GPU decides how many
//! instances to draw. Each surviving instance is a compacted index (an instance-step
//! vertex buffer) that the shader uses to gather the real rectangle from a storage
//! array.
//!
//! Not every target can execute indirect draws: WebGL2 lacks
//! [`DownlevelFlags::INDIRECT_EXECUTION`](wgpu::DownlevelFlags::INDIRECT_EXECUTION).
//! [`IndirectRects::supported`] is the runtime gate; when it returns `false` the caller
//! keeps the direct (CPU-count) draw path instead. On native adapters that report
//! [`Features::MULTI_DRAW_INDIRECT_COUNT`](wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
//! [`IndirectRects::supports_multi_draw`] is `true` and several buckets can be issued
//! in one [`multi_draw_indexed_indirect`](wgpu::RenderPass::multi_draw_indexed_indirect)
//! call; otherwise the baseline is one indirect call per bucket.

use crate::context::WgpuContext;
use crate::retained::RectInstanceT;
use crate::view::ViewUniform;
use bytemuck::bytes_of;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferAddress, BufferBindingType, BufferDescriptor,
    BufferUsages, Device, DownlevelFlags, Features, IndexFormat, MultisampleState,
    PipelineLayoutDescriptor, PrimitiveState, PrimitiveTopology, Queue, RenderPass, RenderPipeline,
    RenderPipelineDescriptor, ShaderStages, TextureFormat, VertexBufferLayout, VertexState,
    VertexStepMode,
};

/// The six indices of a unit quad drawn as two triangles over its four corners
/// `(0,0)-(1,0)-(0,1)-(1,1)`, matching the corner order in `indirect.wgsl`.
const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// A GPU-driven indirect renderer for retained rectangle instances.
pub struct IndirectRects {
    pipeline: RenderPipeline,
    /// The camera uniform, rewritten in place per frame (group 0).
    view_buffer: Buffer,
    view_bind_group: BindGroup,
    /// Bind group layout for the rectangle storage array (group 1), used to build a
    /// bind group per instances buffer via [`IndirectRects::bind_instances`].
    instances_layout: BindGroupLayout,
    /// The static unit-quad index buffer (six `u16` indices).
    quad_index_buffer: Buffer,
}

impl core::fmt::Debug for IndirectRects {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IndirectRects").finish_non_exhaustive()
    }
}

impl IndirectRects {
    /// Whether `ctx`'s adapter can execute indirect draws at all.
    ///
    /// This is the WebGL2 fallback gate: when it returns `false`, indirect drawing is
    /// unavailable and the caller must use the direct (CPU-count) draw path.
    #[must_use]
    pub fn supported(ctx: &WgpuContext) -> bool {
        ctx.adapter()
            .get_downlevel_capabilities()
            .flags
            .contains(DownlevelFlags::INDIRECT_EXECUTION)
    }

    /// Whether `device` supports native (non-emulated) multi-draw indirect.
    ///
    /// In wgpu 29 the non-count `multi_draw_indexed_indirect` is always callable but is
    /// emulated as a series of single indirect draws unless
    /// [`Features::MULTI_DRAW_INDIRECT_COUNT`] is present, which also guarantees native
    /// multi-draw. That feature is therefore the honest gate for the fast path; it is
    /// native-only (absent on WebGPU/WebGL2).
    #[must_use]
    pub fn supports_multi_draw(device: &Device) -> bool {
        device
            .features()
            .contains(Features::MULTI_DRAW_INDIRECT_COUNT)
    }

    /// Builds the indirect pipeline on `device` for the color `format` at
    /// `sample_count` samples per pixel (1 for single-sampled).
    #[must_use]
    pub fn new(device: &Device, format: TextureFormat, sample_count: u32) -> Self {
        let sample_count = sample_count.max(1);
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/indirect.wgsl"));

        // Group 0: the view uniform (vertex stage).
        let view_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render indirect view layout"),
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
        // Group 1: the rectangle instances, read-only storage (vertex stage).
        let instances_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render indirect instances layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render indirect pipeline layout"),
            bind_group_layouts: &[Some(&view_layout), Some(&instances_layout)],
            immediate_size: 0,
        });

        // Per-instance input: the compacted index, one u32 at instance step.
        let index_attrs = wgpu::vertex_attr_array![0 => Uint32];
        let index_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<u32>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &index_attrs,
        };

        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render indirect rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_rect_indirect"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[index_layout],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: MultisampleState {
                count: sample_count,
                ..MultisampleState::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_solid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });

        let view_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render indirect view uniform"),
            size: std::mem::size_of::<ViewUniform>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render indirect view bind group"),
            layout: &view_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        let quad_index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render indirect quad indices"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: BufferUsages::INDEX,
        });

        Self {
            pipeline,
            view_buffer,
            view_bind_group,
            instances_layout,
            quad_index_buffer,
        }
    }

    /// Rewrites the camera uniform in place (one `write_buffer`).
    pub fn set_camera(&self, queue: &Queue, view: &ViewUniform) {
        queue.write_buffer(&self.view_buffer, 0, bytes_of(view));
    }

    /// Builds the group-1 bind group for a rectangle-instance storage buffer.
    ///
    /// The buffer must hold [`RectInstanceT`] records and carry
    /// [`BufferUsages::STORAGE`]. The returned bind group is reused across frames until
    /// the instances change.
    #[must_use]
    pub fn bind_instances(&self, device: &Device, instances: &Buffer) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render indirect instances bind group"),
            layout: &self.instances_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: instances.as_entire_binding(),
            }],
        })
    }

    /// Records one indexed indirect draw of the compacted instances.
    ///
    /// `compacted` is the instance-step vertex buffer of surviving indices (from
    /// [`CompactionOutput::compacted_buffer`](crate::CompactionOutput::compacted_buffer),
    /// which carries [`BufferUsages::VERTEX`]); `draw_args` is the filled
    /// [`DrawIndexedIndirectArgs`](wgpu::util::DrawIndexedIndirectArgs) buffer (with
    /// [`BufferUsages::INDIRECT`]). The instance count comes from `draw_args`, not the
    /// CPU.
    pub fn paint(
        &self,
        pass: &mut RenderPass<'_>,
        instances: &BindGroup,
        compacted: &Buffer,
        draw_args: &Buffer,
    ) {
        self.begin(pass, instances, compacted);
        pass.draw_indexed_indirect(draw_args, 0);
    }

    /// Records several indexed indirect draws from one args buffer (see [`MultiDraw`]).
    ///
    /// The args buffer holds [`MultiDraw::draw_count`] tightly packed
    /// [`DrawIndexedIndirectArgs`](wgpu::util::DrawIndexedIndirectArgs) at
    /// [`MultiDraw::base_offset`]. When [`IndirectRects::supports_multi_draw`] is `true`
    /// this is one native
    /// [`multi_draw_indexed_indirect`](wgpu::RenderPass::multi_draw_indexed_indirect);
    /// otherwise it falls back to that many separate indirect draws (the same result,
    /// issued one bucket at a time), so the call is correct on every native backend.
    pub fn paint_multi(&self, pass: &mut RenderPass<'_>, device: &Device, draw: &MultiDraw<'_>) {
        self.begin(pass, draw.instances, draw.compacted);
        if Self::supports_multi_draw(device) {
            pass.multi_draw_indexed_indirect(draw.draw_args, draw.base_offset, draw.draw_count);
        } else {
            let stride =
                std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as BufferAddress;
            for i in 0..u64::from(draw.draw_count) {
                pass.draw_indexed_indirect(draw.draw_args, draw.base_offset + i * stride);
            }
        }
    }

    /// Sets the pipeline, bind groups, index buffer, and compacted instance buffer
    /// shared by every indirect draw variant.
    fn begin(&self, pass: &mut RenderPass<'_>, instances: &BindGroup, compacted: &Buffer) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.view_bind_group, &[]);
        pass.set_bind_group(1, instances, &[]);
        pass.set_index_buffer(self.quad_index_buffer.slice(..), IndexFormat::Uint16);
        pass.set_vertex_buffer(0, compacted.slice(..));
    }
}

/// The buffers and range for an [`IndirectRects::paint_multi`] call.
///
/// Groups the per-draw resources so the multi-draw entry point stays a single, small
/// call rather than a long positional argument list.
#[derive(Clone, Copy, Debug)]
pub struct MultiDraw<'a> {
    /// The group-1 instances bind group (from [`IndirectRects::bind_instances`]).
    pub instances: &'a BindGroup,
    /// The compacted instance-index vertex buffer (instance step).
    pub compacted: &'a Buffer,
    /// The indexed-indirect args buffer (carries [`BufferUsages::INDIRECT`]).
    pub draw_args: &'a Buffer,
    /// Byte offset of the first args record in `draw_args`.
    pub base_offset: BufferAddress,
    /// Number of tightly packed args records to draw.
    pub draw_count: u32,
}

/// Uploads `instances` into a fresh storage buffer suitable for
/// [`IndirectRects::bind_instances`]. A convenience for callers (and tests) that hold
/// a plain slice of expanded rectangles.
#[must_use]
pub fn upload_instances(device: &Device, instances: &[RectInstanceT]) -> Buffer {
    device.create_buffer_init(&BufferInitDescriptor {
        label: Some("reticle-render indirect instances"),
        contents: bytemuck::cast_slice(instances),
        usage: BufferUsages::STORAGE,
    })
}

//! GPU-driven cell culling (stretch feature).
//!
//! [`CellCuller`] runs `cull.wgsl`: it uploads a list of cell bounding boxes and the
//! viewport, dispatches one compute invocation per box to test overlap, and reads
//! back a per-cell visibility flag. This is the first stage of a GPU draw pipeline;
//! a production path would compact the survivors into an indirect draw buffer, which
//! is left as a follow-up. The visibility buffer form keeps the stage easy to verify
//! from the CPU (see the crate tests).

use crate::context::WgpuContext;
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use reticle_geometry::Rect;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BufferBindingType, BufferDescriptor, BufferUsages,
    CommandEncoderDescriptor, ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor,
    MapMode, PipelineLayoutDescriptor, PollType, ShaderStages,
};

/// One axis-aligned bounding box for a cull candidate, in DBU as floats. Matches
/// `Aabb` in `cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct CullAabb {
    /// Minimum corner `(x, y)`.
    pub min_xy: [f32; 2],
    /// Maximum corner `(x, y)`.
    pub max_xy: [f32; 2],
}

impl CullAabb {
    /// Builds a cull box from a geometry rectangle.
    #[must_use]
    pub fn from_rect(rect: Rect) -> Self {
        Self {
            min_xy: [rect.min.x as f32, rect.min.y as f32],
            max_xy: [rect.max.x as f32, rect.max.y as f32],
        }
    }
}

/// The uniform parameters for a cull dispatch. Matches `Params` in `cull.wgsl`;
/// padded to a 32-byte block so it satisfies uniform-buffer alignment rules.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
struct CullParams {
    view_min: [f32; 2],
    view_max: [f32; 2],
    count: u32,
    _pad: [u32; 3],
}

/// A compute-driven culler that flags which cell bounding boxes overlap a viewport.
pub struct CellCuller {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl core::fmt::Debug for CellCuller {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CellCuller").finish_non_exhaustive()
    }
}

/// The compute workgroup size; must match `@workgroup_size` in `cull.wgsl`.
const WORKGROUP_SIZE: u32 = 64;

impl CellCuller {
    /// Compiles the cull compute pipeline on `ctx`'s device.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = ctx.device();
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/cull.wgsl"));

        let storage = |read_only: bool| BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render cull layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: storage(true),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: storage(false),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render cull pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("reticle-render cull pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cull"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    /// Returns a visibility flag per input box: `1` if the box overlaps `viewport`,
    /// `0` otherwise. The result has the same length and order as `boxes`; an empty
    /// input returns an empty vector without touching the GPU.
    ///
    /// The overlap test is half-open, matching [`Rect::intersects`], so results agree
    /// with a CPU cull for validation.
    #[must_use]
    pub fn cull(&self, ctx: &WgpuContext, boxes: &[CullAabb], viewport: Rect) -> Vec<u32> {
        if boxes.is_empty() {
            return Vec::new();
        }
        let device = ctx.device();
        let count = u32::try_from(boxes.len()).unwrap_or(u32::MAX);

        let params = CullParams {
            view_min: [viewport.min.x as f32, viewport.min.y as f32],
            view_max: [viewport.max.x as f32, viewport.max.y as f32],
            count,
            _pad: [0; 3],
        };
        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render cull params"),
            contents: bytes_of(&params),
            usage: BufferUsages::UNIFORM,
        });
        let box_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render cull boxes"),
            contents: cast_slice(boxes),
            usage: BufferUsages::STORAGE,
        });

        let flags_size = (boxes.len() * std::mem::size_of::<u32>()) as u64;
        let flags_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render cull flags"),
            size: flags_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render cull readback"),
            size: flags_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render cull bind group"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: box_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: flags_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render cull encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("reticle-render cull pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let groups = count.div_ceil(WORKGROUP_SIZE);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&flags_buffer, 0, &readback, 0, flags_size);
        ctx.queue().submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(MapMode::Read, |_| {});
        let _ = ctx.device().poll(PollType::wait_indefinitely());

        let flags = {
            let data = slice.get_mapped_range();
            cast_slice::<u8, u32>(&data).to_vec()
        };
        readback.unmap();
        flags
    }
}

//! GPU-driven cell culling and compaction.
//!
//! [`CellCuller`] runs `cull.wgsl`: it uploads a list of cell bounding boxes and the
//! viewport, dispatches one compute invocation per box to test overlap, and reads
//! back a per-cell visibility flag. The visibility buffer form keeps that stage easy
//! to verify from the CPU (see the crate tests).
//!
//! [`CellCompactor`] runs `compact.wgsl`, the next stage: it takes a visibility flag
//! buffer and stream-compacts the indices of the kept cells into a dense buffer while
//! filling the `instance_count` of a [`DrawIndexedIndirectArgs`] so an indexed
//! indirect draw touches only the survivors. Each workgroup runs an exclusive prefix
//! scan in workgroup memory, reserves an output range with a single atomic add, and
//! writes its survivors there; the global order of the compacted output is
//! unspecified (which is all an instanced draw needs).

use crate::context::WgpuContext;
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use reticle_geometry::Rect;
use wgpu::util::{BufferInitDescriptor, DeviceExt, DrawIndexedIndirectArgs};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferDescriptor, BufferUsages,
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

/// The uniform parameters for a compaction dispatch. Matches `Params` in
/// `compact.wgsl`; padded to a 16-byte block for uniform-buffer alignment.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
struct CompactParams {
    count: u32,
    _pad: [u32; 3],
}

/// The number of indices an indexed unit-quad draw consumes: two triangles over the
/// four quad corners. Written into the indirect args' `index_count` so a later
/// [`RenderPass::draw_indexed_indirect`](wgpu::RenderPass::draw_indexed_indirect)
/// draws the compacted instances.
pub const QUAD_INDEX_COUNT: u32 = 6;

/// The workgroup size of `compact.wgsl`; the dispatch covers
/// `ceil(count / COMPACT_WORKGROUP_SIZE)` groups.
const COMPACT_WORKGROUP_SIZE: u32 = 256;

/// The result of a GPU stream compaction: the dense buffer of surviving instance
/// indices and the indexed-indirect draw arguments whose `instance_count` was filled
/// to match.
///
/// The buffers stay GPU-resident so the compacted output can feed
/// [`RenderPass::draw_indexed_indirect`](wgpu::RenderPass::draw_indexed_indirect)
/// with no CPU round-trip. [`CellCompactor::read_back`] copies them to the CPU for
/// tests and validation.
pub struct CompactionOutput {
    /// Dense buffer of surviving instance indices, `count` `u32`s long (only the first
    /// `instance_count` entries are meaningful). Carries `STORAGE` for the compute
    /// write and `VERTEX` so it can be bound as an instance buffer when drawing.
    compacted: Buffer,
    /// The indexed-indirect draw arguments (`DrawIndexedIndirectArgs`) with
    /// `instance_count` set to the number of survivors. Carries `INDIRECT`.
    draw_args: Buffer,
    /// The length of `compacted` in `u32`s (the input cell count).
    capacity: u32,
}

impl core::fmt::Debug for CompactionOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CompactionOutput")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl CompactionOutput {
    /// The compacted instance-index buffer (bind as an instance vertex buffer).
    #[must_use]
    pub fn compacted_buffer(&self) -> &Buffer {
        &self.compacted
    }

    /// The indexed-indirect draw-args buffer (pass to `draw_indexed_indirect`).
    #[must_use]
    pub fn draw_args_buffer(&self) -> &Buffer {
        &self.draw_args
    }

    /// The capacity of the compacted buffer in instances (the input cell count).
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// A compute-driven stream compactor: turns a visibility flag buffer into a dense list
/// of surviving indices plus indexed-indirect draw arguments.
pub struct CellCompactor {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl core::fmt::Debug for CellCompactor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CellCompactor").finish_non_exhaustive()
    }
}

impl CellCompactor {
    /// Compiles the compaction compute pipeline on `ctx`'s device.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = ctx.device();
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/compact.wgsl"));

        let storage = |read_only: bool| BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render compact layout"),
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
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: storage(false),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::COMPUTE,
                    ty: storage(false),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render compact pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("reticle-render compact pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("compact"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    /// Compacts a visibility flag buffer into surviving indices and indexed-indirect
    /// draw arguments, leaving the results GPU-resident in a [`CompactionOutput`].
    ///
    /// `flags` is one `u32` per cell (nonzero = keep), exactly the buffer
    /// [`CellCuller::cull`] returns. The output's compacted buffer holds the surviving
    /// indices densely packed (in an unspecified order); its draw-args buffer has
    /// `instance_count` equal to the number of survivors and `index_count` equal to
    /// [`QUAD_INDEX_COUNT`]. An empty input yields an output of capacity 0 and a zero
    /// instance count without dispatching.
    #[must_use]
    pub fn compact(&self, ctx: &WgpuContext, flags: &[u32]) -> CompactionOutput {
        let device = ctx.device();
        let capacity = u32::try_from(flags.len()).unwrap_or(u32::MAX);

        // The draw args start with instance_count = 0; the shader accumulates it.
        let initial_args = DrawIndexedIndirectArgs {
            index_count: QUAD_INDEX_COUNT,
            instance_count: 0,
            first_index: 0,
            base_vertex: 0,
            first_instance: 0,
        };
        let draw_args = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render compact draw args"),
            contents: initial_args.as_bytes(),
            usage: BufferUsages::STORAGE
                | BufferUsages::INDIRECT
                | BufferUsages::COPY_SRC
                | BufferUsages::COPY_DST,
        });

        if flags.is_empty() {
            // Nothing to dispatch: an empty (capacity-0) compacted buffer, and the
            // draw args already carry instance_count = 0.
            let compacted = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render compacted indices"),
                size: 0,
                usage: BufferUsages::STORAGE | BufferUsages::VERTEX | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            return CompactionOutput {
                compacted,
                draw_args,
                capacity,
            };
        }

        let params = CompactParams {
            count: capacity,
            _pad: [0; 3],
        };
        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render compact params"),
            contents: bytes_of(&params),
            usage: BufferUsages::UNIFORM,
        });
        let flags_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render compact flags"),
            contents: cast_slice(flags),
            usage: BufferUsages::STORAGE,
        });

        let compacted_size = std::mem::size_of_val(flags) as u64;
        let compacted = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render compacted indices"),
            size: compacted_size,
            usage: BufferUsages::STORAGE | BufferUsages::VERTEX | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // The global reservation cursor, zero-initialized each dispatch.
        let cursor = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render compact cursor"),
            size: std::mem::size_of::<u32>() as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.dispatch(
            ctx,
            capacity,
            &[
                params_buffer.as_entire_binding(),
                flags_buffer.as_entire_binding(),
                compacted.as_entire_binding(),
                cursor.as_entire_binding(),
                draw_args.as_entire_binding(),
            ],
            &cursor,
        );

        CompactionOutput {
            compacted,
            draw_args,
            capacity,
        }
    }

    /// Records and submits the compaction pass: binds the five resources in order,
    /// zeroes the reservation `cursor`, and dispatches `ceil(count / 256)` workgroups.
    fn dispatch(
        &self,
        ctx: &WgpuContext,
        count: u32,
        bindings: &[wgpu::BindingResource<'_>; 5],
        cursor: &Buffer,
    ) {
        let device = ctx.device();
        let entries: Vec<BindGroupEntry<'_>> = bindings
            .iter()
            .enumerate()
            .map(|(i, resource)| BindGroupEntry {
                binding: u32::try_from(i).unwrap_or(0),
                resource: resource.clone(),
            })
            .collect();
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render compact bind group"),
            layout: &self.layout,
            entries: &entries,
        });

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render compact encoder"),
        });
        // Zero the cursor before the pass (COPY_DST clear).
        encoder.clear_buffer(cursor, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("reticle-render compact pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let groups = count.div_ceil(COMPACT_WORKGROUP_SIZE);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        ctx.queue().submit(std::iter::once(encoder.finish()));
    }

    /// Reads a [`CompactionOutput`] back to the CPU as `(survivors, instance_count)`,
    /// where `survivors` is truncated to the meaningful `instance_count` entries.
    ///
    /// This blocks on the device to drive both readbacks to completion, so it is a
    /// test/validation helper rather than a per-frame path (the whole point of
    /// compaction is to keep the data GPU-resident and skip this round-trip).
    #[must_use]
    pub fn read_back(&self, ctx: &WgpuContext, output: &CompactionOutput) -> (Vec<u32>, u32) {
        let device = ctx.device();

        // Copy the compacted indices and the draw args into mappable readback buffers.
        let idx_size = u64::from(output.capacity) * std::mem::size_of::<u32>() as u64;
        let idx_readback = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render compact idx readback"),
            size: idx_size.max(std::mem::size_of::<u32>() as u64),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let args_size = std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
        let args_readback = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render compact args readback"),
            size: args_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render compact readback encoder"),
        });
        if idx_size > 0 {
            encoder.copy_buffer_to_buffer(&output.compacted, 0, &idx_readback, 0, idx_size);
        }
        encoder.copy_buffer_to_buffer(&output.draw_args, 0, &args_readback, 0, args_size);
        ctx.queue().submit(std::iter::once(encoder.finish()));

        // The instance_count lives at u32 index 1 of the draw args.
        let instance_count = {
            let slice = args_readback.slice(..);
            slice.map_async(MapMode::Read, |_| {});
            let _ = device.poll(PollType::wait_indefinitely());
            let data = slice.get_mapped_range();
            let args = cast_slice::<u8, u32>(&data);
            let n = args[1];
            drop(data);
            args_readback.unmap();
            n
        };

        let survivors = if idx_size == 0 {
            Vec::new()
        } else {
            let slice = idx_readback.slice(..idx_size);
            slice.map_async(MapMode::Read, |_| {});
            let _ = device.poll(PollType::wait_indefinitely());
            let all = {
                let data = slice.get_mapped_range();
                cast_slice::<u8, u32>(&data).to_vec()
            };
            idx_readback.unmap();
            let take = usize::try_from(instance_count.min(output.capacity)).unwrap_or(0);
            all[..take].to_vec()
        };

        (survivors, instance_count)
    }
}

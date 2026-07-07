//! A fully GPU-resident arrayed hierarchy: expansion, culling, and compaction on the
//! GPU, drawn with one indirect call per chunk and no per-frame CPU draw-list touch.
//!
//! The retained path ([`crate::RetainedScene`]) expands every instance and array
//! element into a per-placement transform buffer *on the CPU* once, then redraws it.
//! That is fine to tens of millions of placements, but the CPU expansion and the
//! materialized per-placement buffer both scale with the flat-equivalent shape count,
//! so a 100M-element arrayed design (a large via/fill/bit-cell array  -  routine in real
//! layout) pays a large one-time CPU cost and stores every placement.
//!
//! [`GpuHierarchy`] keeps the scene *compact and GPU-resident*: a small table of
//! [`ArrayPlacement`] records (one per array reference, not per element) and a table of
//! leaf [`crate::RectInstance`] cells are uploaded once. Every frame a single compute
//! pass  -  [`expand_cull_compact.wgsl`](../../shaders/expand_cull_compact.wgsl)  -
//! expands the arrays element-by-element, culls each against the viewport, and
//! stream-compacts the survivors into ready-to-draw [`RectInstanceT`] buffers, filling
//! an indirect `instance_count` so the GPU decides how many to draw.
//!
//! # Chunking past the single-dispatch cap
//!
//! One compute dispatch is bounded two ways: at most
//! `max_compute_workgroups_per_dimension` (65,535) workgroups of 256 threads
//! (~16.7M elements), and a storage binding at most `max_storage_buffer_binding_size`
//! (128 MiB, so ~2.79M 48-byte survivors). [`GpuHierarchy`] escapes both by splitting
//! the global element space into fixed-size **chunks** and issuing one dispatch and one
//! `draw_indirect` per chunk. The cap is beaten by chunk *count*, never by a larger
//! dispatch, so the design scales to arbitrarily many elements at fixed per-chunk cost.
//!
//! # Zero per-frame CPU draw-list work
//!
//! The per-frame path ([`GpuHierarchy::expand`] + [`GpuHierarchy::draw`]) iterates only
//! the chunk list (a handful of entries), never per-placement or per-element data. The
//! [`cpu_expand_ops`] counter  -  bumped only by the CPU reference expansion used in
//! tests  -  stays flat across frames, which the crate tests assert.

use crate::context::WgpuContext;
use crate::geometry::RectInstance;
use crate::retained::{InstanceTransform, RectInstanceT};
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use reticle_geometry::Rect;
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::util::{BufferInitDescriptor, DeviceExt, DrawIndirectArgs};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferDescriptor, BufferUsages,
    CommandEncoderDescriptor, ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor,
    MapMode, PipelineLayoutDescriptor, PollType, RenderPass, RenderPipeline, ShaderStages,
};

/// Count of CPU-side per-element expansions performed process-wide.
///
/// Only [`GpuHierarchy::cpu_reference`] (a test/validation helper) bumps this, once per
/// expanded element. The per-frame GPU path never does, so a steady-state frame loop
/// leaves it unchanged  -  the observable form of "zero per-frame CPU draw-list touch".
static CPU_EXPAND_OPS: AtomicU64 = AtomicU64::new(0);

/// The current value of the CPU per-element expansion counter (`CPU_EXPAND_OPS`).
#[must_use]
pub fn cpu_expand_ops() -> u64 {
    CPU_EXPAND_OPS.load(Ordering::Relaxed)
}

/// The compute workgroup size; must match `@workgroup_size` in
/// `expand_cull_compact.wgsl`.
const WORKGROUP_SIZE: u32 = 256;

/// Bytes per expanded instance ([`RectInstanceT`], 48 bytes): the compacted output
/// stride, which bounds a chunk against the storage-binding limit.
const RECT_T_STRIDE: u64 = std::mem::size_of::<RectInstanceT>() as u64;

/// One array placement uploaded to the GPU: a leaf cell, a base placement transform,
/// and the array's columns/rows and pitches. Matches `ArrayPlacement` in
/// `expand_cull_compact.wgsl` (48 bytes).
///
/// `element_offset` and `element_count` are filled by [`GpuHierarchy::upload`]; callers
/// build a placement with [`ArrayPlacement::new`] and leave them zero.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct ArrayPlacement {
    /// Base transform integer translation `(x, y)` in DBU.
    pub translate: [i32; 2],
    /// Index of the leaf cell in the cells table.
    pub cell_index: u32,
    /// Base dihedral orientation code in `0..8`.
    pub orientation_code: u32,
    /// Base uniform magnification.
    pub magnification: f32,
    /// Array columns (at least 1; a single instance is a 1x1 array).
    pub columns: u32,
    /// Array rows (at least 1).
    pub rows: u32,
    /// Per-column step in the arrayed cell's local frame (DBU).
    pub col_pitch: i32,
    /// Per-row step (DBU).
    pub row_pitch: i32,
    /// Exclusive prefix sum of element counts; set by [`GpuHierarchy::upload`].
    element_offset: u32,
    /// `columns * rows`; set by [`GpuHierarchy::upload`].
    element_count: u32,
    _pad: u32,
}

impl ArrayPlacement {
    /// Builds a placement of `cell_index` arrayed `columns` x `rows` with the given base
    /// transform (`base`) and per-column/row pitches. `columns`/`rows` are clamped to at
    /// least 1.
    #[must_use]
    pub fn new(
        cell_index: u32,
        base: InstanceTransform,
        columns: u32,
        rows: u32,
        col_pitch: i32,
        row_pitch: i32,
    ) -> Self {
        Self {
            translate: base.translate,
            cell_index,
            orientation_code: base.orientation_code,
            magnification: base.magnification,
            columns: columns.max(1),
            rows: rows.max(1),
            col_pitch,
            row_pitch,
            element_offset: 0,
            element_count: 0,
            _pad: 0,
        }
    }

    /// The number of elements (`columns * rows`) this placement expands to.
    #[must_use]
    pub fn element_span(&self) -> u64 {
        u64::from(self.columns.max(1)) * u64::from(self.rows.max(1))
    }
}

/// The uniform parameters for one chunk's expand dispatch. Matches `Params` in
/// `expand_cull_compact.wgsl` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
struct Params {
    view_min: [f32; 2],
    view_max: [f32; 2],
    chunk_base: u32,
    chunk_count: u32,
    placement_count: u32,
    _pad: u32,
}

/// One chunk of the global element space: its window, its GPU-resident output buffers,
/// and the bind group wiring them to the compute pass.
struct Chunk {
    base: u32,
    count: u32,
    /// Compacted survivors for this chunk (`STORAGE | VERTEX | COPY_SRC`), sized to hold
    /// every element in the chunk in the worst case (all visible).
    compacted: Buffer,
    /// The reservation cursor, zeroed each dispatch.
    cursor: Buffer,
    /// The non-indexed indirect draw args; `instance_count` is filled by the dispatch.
    draw_args: Buffer,
    /// Per-frame uniform (viewport + this chunk's window).
    params: Buffer,
    bind_group: BindGroup,
}

impl core::fmt::Debug for Chunk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Chunk")
            .field("base", &self.base)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

/// The initial indirect args for a chunk: a four-vertex triangle-strip unit quad, zero
/// instances (the dispatch accumulates the count). Rewritten each frame to reset the
/// count before the pass.
fn initial_draw_args() -> DrawIndirectArgs {
    DrawIndirectArgs {
        vertex_count: 4,
        instance_count: 0,
        first_vertex: 0,
        first_instance: 0,
    }
}

/// A fully GPU-resident arrayed hierarchy renderer.
///
/// Build once with [`GpuHierarchy::new`], upload a scene with [`GpuHierarchy::upload`],
/// then each frame call [`GpuHierarchy::expand`] (one compute dispatch per chunk) and
/// [`GpuHierarchy::draw`] (one indirect draw per chunk) on the retained rect pipeline.
pub struct GpuHierarchy {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    /// The per-chunk element cap derived from the device limits.
    max_chunk_elements: u32,
    /// The compact scene tables, uploaded once.
    placements: Buffer,
    cells: Buffer,
    placement_count: u32,
    total_elements: u64,
    /// The element window per chunk actually used (<= `max_chunk_elements`).
    chunk_elements: u32,
    chunks: Vec<Chunk>,
}

impl core::fmt::Debug for GpuHierarchy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuHierarchy")
            .field("placement_count", &self.placement_count)
            .field("total_elements", &self.total_elements)
            .field("chunk_elements", &self.chunk_elements)
            .field("chunks", &self.chunks.len())
            .finish_non_exhaustive()
    }
}

impl GpuHierarchy {
    /// Compiles the expansion compute pipeline and derives the per-chunk element cap
    /// from `ctx`'s device limits. The scene tables start empty; call
    /// [`GpuHierarchy::upload`] to load a scene.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = ctx.device();
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/expand_cull_compact.wgsl"));

        let storage = |read_only: bool| BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let entry = |binding: u32, ty: BindingType| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render gpu-hierarchy layout"),
            entries: &[
                entry(
                    0,
                    BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(1, storage(true)),  // placements
                entry(2, storage(true)),  // cells
                entry(3, storage(false)), // compacted
                entry(4, storage(false)), // cursor
                entry(5, storage(false)), // draw_args
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render gpu-hierarchy pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("reticle-render gpu-hierarchy pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("expand_cull_compact"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let max_chunk_elements = Self::derive_chunk_cap(ctx);

        // Empty scene: one-element placeholder buffers so bind groups are always valid.
        let placements = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render gpu-hierarchy placements"),
            contents: bytes_of(&ArrayPlacement::new(
                0,
                InstanceTransform::IDENTITY,
                1,
                1,
                0,
                0,
            )),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });
        let cells = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render gpu-hierarchy cells"),
            contents: bytes_of(&RectInstance {
                min_xy: [0.0, 0.0],
                max_xy: [0.0, 0.0],
                color: [0.0, 0.0, 0.0, 0.0],
            }),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });

        Self {
            layout,
            pipeline,
            max_chunk_elements,
            placements,
            cells,
            placement_count: 0,
            total_elements: 0,
            chunk_elements: max_chunk_elements,
            chunks: Vec::new(),
        }
    }

    /// The largest number of elements one chunk may cover on this device, bounded by
    /// the storage-binding limit (the 48-byte compacted output), the dispatch workgroup
    /// count, and the max buffer size, then floored to a multiple of the workgroup size.
    ///
    /// This is the single-dispatch cap the module chunks past; it is public so
    /// benchmarks can report it honestly.
    #[must_use]
    pub fn derive_chunk_cap(ctx: &WgpuContext) -> u32 {
        let limits = ctx.device().limits();
        let by_binding = limits.max_storage_buffer_binding_size / RECT_T_STRIDE;
        let by_buffer = limits.max_buffer_size / RECT_T_STRIDE;
        let by_dispatch =
            u64::from(limits.max_compute_workgroups_per_dimension) * u64::from(WORKGROUP_SIZE);
        let cap = by_binding.min(by_buffer).min(by_dispatch);
        // Floor to a whole workgroup and to u32.
        let cap = (cap / u64::from(WORKGROUP_SIZE)) * u64::from(WORKGROUP_SIZE);
        u32::try_from(cap).unwrap_or(u32::MAX).max(WORKGROUP_SIZE)
    }

    /// The per-chunk element cap derived from the device limits.
    #[must_use]
    pub fn max_chunk_elements(&self) -> u32 {
        self.max_chunk_elements
    }

    /// Uploads a scene: leaf `cells` and array `placements`, chunked at the device cap.
    ///
    /// Element offsets are assigned here (the exclusive prefix of each placement's
    /// `columns * rows`), so callers leave [`ArrayPlacement::new`]'s offset/count zero.
    /// Panics never; an empty scene yields zero chunks.
    pub fn upload(
        &mut self,
        ctx: &WgpuContext,
        cells: &[RectInstance],
        placements: &[ArrayPlacement],
    ) {
        let cap = self.max_chunk_elements;
        self.upload_with_chunk_size(ctx, cells, placements, cap);
    }

    /// Like [`GpuHierarchy::upload`] but with an explicit chunk size (clamped to the
    /// device cap). Used by tests to force multi-chunk splits well below the hardware
    /// cap, exercising the chunked path deterministically.
    pub fn upload_with_chunk_size(
        &mut self,
        ctx: &WgpuContext,
        cells: &[RectInstance],
        placements: &[ArrayPlacement],
        chunk_size: u32,
    ) {
        let device = ctx.device();
        self.chunk_elements = chunk_size.clamp(WORKGROUP_SIZE, self.max_chunk_elements);

        // Fill element offsets/counts and total. Saturate at u32 for the element space
        // (over 4.19G elements is beyond any real scene and beyond a single index).
        let mut filled = placements.to_vec();
        let mut offset: u64 = 0;
        for p in &mut filled {
            let span = p.element_span();
            p.element_offset = u32::try_from(offset).unwrap_or(u32::MAX);
            p.element_count = u32::try_from(span).unwrap_or(u32::MAX);
            offset = offset.saturating_add(span);
        }
        self.total_elements = offset;
        self.placement_count = u32::try_from(filled.len()).unwrap_or(u32::MAX);

        // Upload the tables (a one-element placeholder keeps buffers non-empty/bindable).
        self.placements = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render gpu-hierarchy placements"),
            contents: if filled.is_empty() {
                bytes_of(&ArrayPlacement::new(
                    0,
                    InstanceTransform::IDENTITY,
                    1,
                    1,
                    0,
                    0,
                ))
                .to_vec()
            } else {
                cast_slice(&filled).to_vec()
            }
            .as_slice(),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });
        self.cells = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render gpu-hierarchy cells"),
            contents: if cells.is_empty() {
                bytes_of(&RectInstance {
                    min_xy: [0.0, 0.0],
                    max_xy: [0.0, 0.0],
                    color: [0.0, 0.0, 0.0, 0.0],
                })
                .to_vec()
            } else {
                cast_slice(cells).to_vec()
            }
            .as_slice(),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        });

        self.rebuild_chunks(device);
    }

    /// (Re)builds the per-chunk output buffers and bind groups for the current scene.
    fn rebuild_chunks(&mut self, device: &wgpu::Device) {
        self.chunks.clear();
        if self.total_elements == 0 || self.placement_count == 0 {
            return;
        }
        let chunk = u64::from(self.chunk_elements);
        let mut base: u64 = 0;
        while base < self.total_elements {
            let count = (self.total_elements - base).min(chunk);
            let base_u = u32::try_from(base).unwrap_or(u32::MAX);
            let count_u = u32::try_from(count).unwrap_or(u32::MAX);

            let compacted = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render gpu-hierarchy compacted"),
                size: count * RECT_T_STRIDE,
                usage: BufferUsages::STORAGE | BufferUsages::VERTEX | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let cursor = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render gpu-hierarchy cursor"),
                size: std::mem::size_of::<u32>() as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let draw_args = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("reticle-render gpu-hierarchy draw args"),
                contents: initial_draw_args().as_bytes(),
                usage: BufferUsages::STORAGE
                    | BufferUsages::INDIRECT
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
            });
            let params = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render gpu-hierarchy params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("reticle-render gpu-hierarchy bind group"),
                layout: &self.layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: self.placements.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.cells.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: compacted.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: cursor.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: draw_args.as_entire_binding(),
                    },
                ],
            });

            self.chunks.push(Chunk {
                base: base_u,
                count: count_u,
                compacted,
                cursor,
                draw_args,
                params,
                bind_group,
            });
            base += count;
        }
    }

    /// The number of chunks the current scene occupies (one indirect draw each).
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// The total number of array elements (flat-equivalent shapes) in the scene.
    #[must_use]
    pub fn total_elements(&self) -> u64 {
        self.total_elements
    }

    /// Runs the per-frame expansion: one compute dispatch per chunk that expands,
    /// culls against `viewport`, and compacts survivors into that chunk's buffers,
    /// filling each chunk's indirect `instance_count`.
    ///
    /// This touches only the chunk list (a handful of entries), never per-element data,
    /// so it is O(chunks) on the CPU regardless of the element count.
    pub fn expand(&self, ctx: &WgpuContext, viewport: Rect) {
        if self.chunks.is_empty() {
            return;
        }
        let device = ctx.device();
        let queue = ctx.queue();
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render gpu-hierarchy expand encoder"),
        });
        for chunk in &self.chunks {
            let params = Params {
                view_min: [viewport.min.x as f32, viewport.min.y as f32],
                view_max: [viewport.max.x as f32, viewport.max.y as f32],
                chunk_base: chunk.base,
                chunk_count: chunk.count,
                placement_count: self.placement_count,
                _pad: 0,
            };
            queue.write_buffer(&chunk.params, 0, bytes_of(&params));
            // Reset the count each frame: zero the cursor and rewrite the args' zeroed
            // instance_count (leaving vertex_count = 4).
            encoder.clear_buffer(&chunk.cursor, 0, None);
            queue.write_buffer(&chunk.draw_args, 0, initial_draw_args().as_bytes());

            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("reticle-render gpu-hierarchy expand pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &chunk.bind_group, &[]);
            let groups = chunk.count.div_ceil(WORKGROUP_SIZE);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Records the per-chunk indirect draws into `pass` on the retained rect
    /// `pipeline`, with `view_bind_group` already holding the camera at group 0.
    ///
    /// Assumes [`GpuHierarchy::expand`] ran this frame. Each chunk is one
    /// `draw_indirect` whose instance count the compute pass decided; the CPU passes no
    /// count. Requires the adapter to support indirect execution (native/desktop; the
    /// retained CPU-count path is the WebGL2 fallback).
    pub fn draw(
        &self,
        pass: &mut RenderPass<'_>,
        pipeline: &RenderPipeline,
        view_bind_group: &BindGroup,
    ) {
        if self.chunks.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, view_bind_group, &[]);
        for chunk in &self.chunks {
            pass.set_vertex_buffer(0, chunk.compacted.slice(..));
            pass.draw_indirect(&chunk.draw_args, 0);
        }
    }

    /// Reads every chunk's survivor count back to the CPU and returns their sum.
    ///
    /// A blocking test/validation helper (the whole point of the design is to keep the
    /// data GPU-resident and skip this round-trip), not a per-frame path.
    #[must_use]
    pub fn read_survivor_count(&self, ctx: &WgpuContext) -> u64 {
        let device = ctx.device();
        let mut total: u64 = 0;
        for chunk in &self.chunks {
            let args_size = std::mem::size_of::<DrawIndirectArgs>() as u64;
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render gpu-hierarchy args readback"),
                size: args_size,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("reticle-render gpu-hierarchy args readback encoder"),
            });
            encoder.copy_buffer_to_buffer(&chunk.draw_args, 0, &readback, 0, args_size);
            ctx.queue().submit(std::iter::once(encoder.finish()));

            let slice = readback.slice(..);
            slice.map_async(MapMode::Read, |_| {});
            let _ = device.poll(PollType::wait_indefinitely());
            let count = {
                let data = slice.get_mapped_range();
                // instance_count is the second u32 of DrawIndirectArgs.
                cast_slice::<u8, u32>(&data)[1]
            };
            readback.unmap();
            total += u64::from(count);
        }
        total
    }

    /// Reads every chunk's compacted survivors back as expanded instances, in an
    /// unspecified order (each chunk's survivors are contiguous, but the global order is
    /// not meaningful). A blocking test/validation helper.
    #[must_use]
    pub fn read_survivors(&self, ctx: &WgpuContext) -> Vec<RectInstanceT> {
        let device = ctx.device();
        let mut out = Vec::new();
        for chunk in &self.chunks {
            // First read the count so only the meaningful prefix is copied back.
            let count = Self::read_chunk_count(ctx, chunk);
            if count == 0 {
                continue;
            }
            let bytes = u64::from(count) * RECT_T_STRIDE;
            let readback = device.create_buffer(&BufferDescriptor {
                label: Some("reticle-render gpu-hierarchy survivors readback"),
                size: bytes,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("reticle-render gpu-hierarchy survivors readback encoder"),
            });
            encoder.copy_buffer_to_buffer(&chunk.compacted, 0, &readback, 0, bytes);
            ctx.queue().submit(std::iter::once(encoder.finish()));

            let slice = readback.slice(..);
            slice.map_async(MapMode::Read, |_| {});
            let _ = device.poll(PollType::wait_indefinitely());
            {
                let data = slice.get_mapped_range();
                out.extend_from_slice(cast_slice::<u8, RectInstanceT>(&data));
            }
            readback.unmap();
        }
        out
    }

    /// Reads one chunk's survivor count (the indirect `instance_count`).
    fn read_chunk_count(ctx: &WgpuContext, chunk: &Chunk) -> u32 {
        let device = ctx.device();
        let args_size = std::mem::size_of::<DrawIndirectArgs>() as u64;
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render gpu-hierarchy chunk count readback"),
            size: args_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render gpu-hierarchy chunk count encoder"),
        });
        encoder.copy_buffer_to_buffer(&chunk.draw_args, 0, &readback, 0, args_size);
        ctx.queue().submit(std::iter::once(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(MapMode::Read, |_| {});
        let _ = device.poll(PollType::wait_indefinitely());
        let count = {
            let data = slice.get_mapped_range();
            cast_slice::<u8, u32>(&data)[1]
        };
        readback.unmap();
        count
    }

    /// The CPU reference expansion: every element of every placement, culled against
    /// `viewport`, as expanded instances. Mirrors the shader math exactly so a GPU
    /// readback can be compared against it as a set.
    ///
    /// This bumps [`cpu_expand_ops`] once per element, which is the counter the
    /// zero-per-frame assertion watches: the GPU frame path never calls this.
    #[must_use]
    pub fn cpu_reference(
        cells: &[RectInstance],
        placements: &[ArrayPlacement],
        viewport: Rect,
    ) -> Vec<RectInstanceT> {
        let mut out = Vec::new();
        for p in placements {
            let Some(cell) = cells.get(p.cell_index as usize) else {
                continue;
            };
            for row in 0..p.rows.max(1) {
                for col in 0..p.columns.max(1) {
                    CPU_EXPAND_OPS.fetch_add(1, Ordering::Relaxed);
                    let inst = expand_element(p, cell, col, row);
                    if instance_visible(&inst, viewport) {
                        out.push(inst);
                    }
                }
            }
        }
        out
    }
}

/// The image of `(x, y)` under the dihedral orientation `code`, matching
/// `orientation_matrix` in the shaders and `Orientation::apply` in reticle-geometry.
fn oriented(code: u32, x: f32, y: f32) -> [f32; 2] {
    match code {
        0 => [x, y],
        1 => [-y, x],
        2 => [-x, -y],
        3 => [y, -x],
        4 => [x, -y],
        5 => [y, x],
        6 => [-x, y],
        _ => [-y, -x],
    }
}

/// CPU mirror of the shader's `expand`: the ready-to-draw instance for element
/// `(col, row)` of `p`.
fn expand_element(p: &ArrayPlacement, cell: &RectInstance, col: u32, row: u32) -> RectInstanceT {
    let dx = (col as i32).wrapping_mul(p.col_pitch) as f32;
    let dy = (row as i32).wrapping_mul(p.row_pitch) as f32;
    let [sx, sy] = oriented(
        p.orientation_code,
        p.magnification * dx,
        p.magnification * dy,
    );
    RectInstanceT {
        min_xy: cell.min_xy,
        max_xy: cell.max_xy,
        color: cell.color,
        orientation_code: p.orientation_code,
        magnification: p.magnification,
        translate: [
            p.translate[0] + sx.round() as i32,
            p.translate[1] + sy.round() as i32,
        ],
    }
}

/// CPU mirror of the shader's `instance_bbox` + `visible`: half-open overlap of the
/// transformed rect against `viewport`.
fn instance_visible(inst: &RectInstanceT, viewport: Rect) -> bool {
    let t = [inst.translate[0] as f32, inst.translate[1] as f32];
    let corner = |x: f32, y: f32| {
        let [ox, oy] = oriented(inst.orientation_code, x, y);
        [
            ox * inst.magnification + t[0],
            oy * inst.magnification + t[1],
        ]
    };
    let c0 = corner(inst.min_xy[0], inst.min_xy[1]);
    let c1 = corner(inst.max_xy[0], inst.max_xy[1]);
    let lo = [c0[0].min(c1[0]), c0[1].min(c1[1])];
    let hi = [c0[0].max(c1[0]), c0[1].max(c1[1])];
    let vmin = [viewport.min.x as f32, viewport.min.y as f32];
    let vmax = [viewport.max.x as f32, viewport.max.y as f32];
    lo[0] < vmax[0] && vmin[0] < hi[0] && lo[1] < vmax[1] && vmin[1] < hi[1]
}

#[cfg(test)]
mod tests {
    use super::{ArrayPlacement, InstanceTransform, Params};
    use crate::retained::RectInstanceT;

    #[test]
    fn array_placement_is_48_bytes() {
        assert_eq!(std::mem::size_of::<ArrayPlacement>(), 48);
    }

    #[test]
    fn params_is_32_bytes() {
        assert_eq!(std::mem::size_of::<Params>(), 32);
    }

    #[test]
    fn rect_instance_t_stride_matches_shader() {
        // The compacted output is bound both as a storage array and as the retained
        // vertex buffer, so the stride must stay 48.
        assert_eq!(std::mem::size_of::<RectInstanceT>(), 48);
    }

    #[test]
    fn element_offsets_are_exclusive_prefixes() {
        // A 2x3 array and a 4x1 array: spans 6 and 4, offsets 0 and 6.
        let a = ArrayPlacement::new(0, InstanceTransform::IDENTITY, 3, 2, 10, 10);
        let b = ArrayPlacement::new(0, InstanceTransform::IDENTITY, 4, 1, 10, 10);
        assert_eq!(a.element_span(), 6);
        assert_eq!(b.element_span(), 4);
    }
}

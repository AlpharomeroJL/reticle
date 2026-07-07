//! Compute-shader DRC heatmap over retained rect instances.
//!
//! [`DrcHeatmap`] runs a two-stage GPU design-rule check that reuses the culling
//! crate's prefix-scan machinery for spatial binning:
//!
//! 1. **Binning** ([`drc_bin.wgsl`](../../shaders/drc_bin.wgsl)) sorts the visible
//!    [`RectInstanceT`]s into a uniform grid with a counting sort: an atomic count per
//!    bin, an exclusive prefix scan of those counts (the same 256-thread Hillis-Steele
//!    workgroup scan as `compact.wgsl`), then a scatter of instance indices into dense
//!    per-bin slices.
//! 2. **Checking** ([`drc_check.wgsl`](../../shaders/drc_check.wgsl)) runs one thread
//!    per instance: it flags a **min-width** violation when the smaller side of the
//!    instance's world box is below the width rule, and a **min-spacing** violation when
//!    any instance in its 3x3 bin neighbourhood sits at a positive edge gap below the
//!    spacing rule. It writes a per-instance flag buffer and accumulates a coarse
//!    per-bin heatmap of the violating-instance count.
//!
//! The bin size is chosen (on the CPU, in [`plan_grid`]) to be at least
//! `max_instance_extent + min_spacing`, which guarantees two instances whose edge gap is
//! below the rule always land in bins at most one apart on each axis. That is the
//! invariant that makes the 3x3 neighbourhood search *exhaustive*: the GPU never misses
//! a violation the CPU `DrcEngine` (in `reticle-drc`) would find. The grid is
//! capped at 256 bins so the whole prefix scan fits a single workgroup.
//!
//! # Coordinate constraint
//!
//! The geometry math mirrors `reticle-drc`'s exact integer helpers, but the compute
//! path works in `f32`. It is bit-exact against the CPU oracle only while every
//! coordinate, width, and height is an integer strictly below `2^24` (the largest range
//! over which `f32` represents consecutive integers exactly). The GPU-vs-CPU property
//! test generates layouts inside that range with identity placement transforms; see
//! `tests/drc_heatmap_gpu.rs`.
//!
//! [`RectInstanceT`]: crate::RectInstanceT

use crate::context::WgpuContext;
use crate::retained::RectInstanceT;
use crate::target::OffscreenTarget;
use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, BlendState, Buffer, BufferBindingType, BufferDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoderDescriptor, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, FragmentState, LoadOp, MapMode, MultisampleState,
    Operations, PipelineLayoutDescriptor, PollType, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderStages, StoreOp,
    TextureFormat, VertexState,
};

/// The compute workgroup size; must match `@workgroup_size` in both DRC shaders.
const WORKGROUP_SIZE: u32 = 256;

/// The maximum number of grid bins. The bin prefix scan runs in a single 256-thread
/// workgroup (mirroring `compact.wgsl`), so the grid is capped at `16 x 16` bins. This
/// is deliberately coarse - the heatmap is a coarse field and the bins only need to be
/// at least `max_instance_extent + min_spacing` wide for the 3x3 search to be exact.
const MAX_GRID_AXIS: u32 = 16;

/// The two DRC rules the compute path checks, in database units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrcRules {
    /// Minimum feature width: an instance whose smaller world side is below this is
    /// flagged (matches [`RuleKind::Width`](reticle_model::RuleKind::Width)).
    pub min_width: u32,
    /// Minimum edge-to-edge spacing: an instance with a neighbour at a positive gap
    /// below this is flagged (matches [`RuleKind::Spacing`](reticle_model::RuleKind::Spacing)).
    pub min_spacing: u32,
}

/// The shared uniform block for the binning and check passes. Matches `Params` in both
/// shaders; padded to 48 bytes for uniform-buffer alignment.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
struct DrcParamsRaw {
    world_min: [f32; 2],
    grid: [u32; 2],
    bin_size: [u32; 2],
    count: u32,
    min_width: u32,
    min_spacing: u32,
    _pad: [u32; 3],
}

/// A resolved grid plan: where the world starts, how many bins per axis, and how wide
/// each bin is (in DBU).
#[derive(Clone, Copy, PartialEq, Debug)]
struct GridPlan {
    world_min: [f32; 2],
    grid: [u32; 2],
    bin_size: [u32; 2],
}

impl GridPlan {
    /// The total number of bins in the grid.
    fn bins(&self) -> u32 {
        self.grid[0] * self.grid[1]
    }
}

/// The world-space axis-aligned bounding box of an instance, mirroring `world_aabb` in
/// the shaders: the four local corners under orientation, magnification, then integer
/// translation. Returns `(min, max)`.
fn world_aabb(inst: &RectInstanceT) -> ([f32; 2], [f32; 2]) {
    let orient = |code: u32, x: f32, y: f32| -> (f32, f32) {
        match code {
            0 => (x, y),
            1 => (-y, x),
            2 => (-x, -y),
            3 => (y, -x),
            4 => (x, -y),
            5 => (y, x),
            6 => (-x, y),
            _ => (-y, -x),
        }
    };
    let m = inst.magnification;
    let tx = inst.translate[0] as f32;
    let ty = inst.translate[1] as f32;
    let corners = [
        (inst.min_xy[0], inst.min_xy[1]),
        (inst.max_xy[0], inst.min_xy[1]),
        (inst.min_xy[0], inst.max_xy[1]),
        (inst.max_xy[0], inst.max_xy[1]),
    ];
    let mut lo = [f32::INFINITY, f32::INFINITY];
    let mut hi = [f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (x, y) in corners {
        let (ox, oy) = orient(inst.orientation_code, x, y);
        let wx = ox * m + tx;
        let wy = oy * m + ty;
        lo[0] = lo[0].min(wx);
        lo[1] = lo[1].min(wy);
        hi[0] = hi[0].max(wx);
        hi[1] = hi[1].max(wy);
    }
    (lo, hi)
}

/// Plans the binning grid for a set of instances and the spacing rule.
///
/// The bin size on each axis is at least `max_instance_extent + min_spacing`, so any
/// two instances whose edge gap is below `min_spacing` sit in bins at most one apart on
/// that axis. The bin count per axis is chosen to cover the world span at that size, and
/// capped at [`MAX_GRID_AXIS`] (making bins larger, never smaller, which keeps the 3x3
/// search exhaustive). An empty input yields a trivial `1 x 1` grid.
fn plan_grid(instances: &[RectInstanceT], min_spacing: u32) -> GridPlan {
    if instances.is_empty() {
        return GridPlan {
            world_min: [0.0, 0.0],
            grid: [1, 1],
            bin_size: [1, 1],
        };
    }

    let mut lo = [f32::INFINITY, f32::INFINITY];
    let mut hi = [f32::NEG_INFINITY, f32::NEG_INFINITY];
    let mut max_extent = 0.0f32;
    for inst in instances {
        let (blo, bhi) = world_aabb(inst);
        lo[0] = lo[0].min(blo[0]);
        lo[1] = lo[1].min(blo[1]);
        hi[0] = hi[0].max(bhi[0]);
        hi[1] = hi[1].max(bhi[1]);
        max_extent = max_extent.max(bhi[0] - blo[0]).max(bhi[1] - blo[1]);
    }

    // The minimum bin size for a correct 3x3 search, at least 1 DBU.
    let safe_bin = (max_extent.ceil() as u32)
        .saturating_add(min_spacing)
        .max(1);

    let axis = |span: f32| -> (u32, u32) {
        let span = span.max(0.0).ceil() as u32;
        // As many bins as fit at `safe_bin`, but at least one and at most MAX_GRID_AXIS.
        let fit = (span / safe_bin).clamp(1, MAX_GRID_AXIS);
        // Bin size that covers the span with `fit` bins, never below `safe_bin`.
        let bin = span.div_ceil(fit).max(safe_bin);
        (fit, bin)
    };

    let (gx, bx) = axis(hi[0] - lo[0]);
    let (gy, by) = axis(hi[1] - lo[1]);
    GridPlan {
        world_min: [lo[0], lo[1]],
        grid: [gx, gy],
        bin_size: [bx, by],
    }
}

/// The GPU-resident result of a DRC heatmap run: a per-instance violation flag buffer
/// and a per-bin heatmap count buffer, plus the grid the run used.
///
/// The buffers stay on the GPU so the heatmap can feed [`DrcHeatmapOverlay`] with no
/// round-trip; [`DrcHeatmap::read_flags`] and [`DrcHeatmap::read_heatmap`] copy them
/// back for tests and validation.
pub struct DrcOutput {
    flags: Buffer,
    heatmap: Buffer,
    grid: [u32; 2],
    count: u32,
}

impl core::fmt::Debug for DrcOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrcOutput")
            .field("grid", &self.grid)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl DrcOutput {
    /// The per-instance violation flag buffer (one `u32` per instance, 1 = violation).
    #[must_use]
    pub fn flags_buffer(&self) -> &Buffer {
        &self.flags
    }

    /// The per-bin heatmap buffer (one `u32` per bin: the count of violating instances).
    #[must_use]
    pub fn heatmap_buffer(&self) -> &Buffer {
        &self.heatmap
    }

    /// The grid dimensions `(bins_x, bins_y)` this run used.
    #[must_use]
    pub fn grid(&self) -> [u32; 2] {
        self.grid
    }

    /// The number of bins in the heatmap (`bins_x * bins_y`).
    #[must_use]
    pub fn bin_count(&self) -> u32 {
        self.grid[0] * self.grid[1]
    }
}

/// A read-only storage binding at `binding`, visible to the compute stage.
fn storage_entry(binding: u32, read_only: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A uniform binding at `binding`, visible to the compute stage.
fn uniform_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// The binning scratch and flag/heatmap output buffers allocated for one DRC run.
///
/// The three per-bin buffers are sized to the fixed 256-bin scan capacity so the
/// workgroup scan always has valid slots; `binned_index` and `flags` are one `u32` per
/// instance, and `heatmap` is one `u32` per live grid bin.
struct DrcBuffers {
    bin_count: Buffer,
    bin_offset: Buffer,
    bin_cursor: Buffer,
    binned_index: Buffer,
    flags: Buffer,
    heatmap: Buffer,
}

impl DrcBuffers {
    /// Allocates the scratch and output buffers for `count` instances over `bins` cells.
    fn new(device: &wgpu::Device, count: u32, bins: u32) -> Self {
        let u32_size = std::mem::size_of::<u32>() as u64;
        let bin_scratch = u64::from(WORKGROUP_SIZE) * u32_size;
        let per_instance = (u64::from(count) * u32_size).max(4);
        let heatmap_size = (u64::from(bins) * u32_size).max(4);
        let storage = |label: &str, size: u64, extra: BufferUsages| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        Self {
            bin_count: storage(
                "reticle-render drc bin count",
                bin_scratch,
                BufferUsages::COPY_DST,
            ),
            bin_offset: storage(
                "reticle-render drc bin offset",
                bin_scratch,
                BufferUsages::empty(),
            ),
            bin_cursor: storage(
                "reticle-render drc bin cursor",
                bin_scratch,
                BufferUsages::COPY_DST,
            ),
            binned_index: storage(
                "reticle-render drc binned index",
                per_instance,
                BufferUsages::empty(),
            ),
            flags: storage(
                "reticle-render drc flags",
                per_instance,
                BufferUsages::COPY_SRC,
            ),
            heatmap: storage(
                "reticle-render drc heatmap",
                heatmap_size,
                BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            ),
        }
    }
}

/// The compute-shader DRC heatmap pipeline: binning (count/scan/scatter) plus the check
/// pass. Build it once with [`DrcHeatmap::new`] and reuse it across runs.
pub struct DrcHeatmap {
    bin_layout: BindGroupLayout,
    check_layout: BindGroupLayout,
    count_pipeline: ComputePipeline,
    scan_pipeline: ComputePipeline,
    scatter_pipeline: ComputePipeline,
    check_pipeline: ComputePipeline,
}

impl core::fmt::Debug for DrcHeatmap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrcHeatmap").finish_non_exhaustive()
    }
}

impl DrcHeatmap {
    /// Compiles the binning and check compute pipelines on `ctx`'s device.
    #[must_use]
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = ctx.device();
        let bin_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/drc_bin.wgsl"));
        let check_shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/drc_check.wgsl"));

        // Binning bindings: params, instances, bin_count, bin_offset, bin_cursor,
        // binned_index. `bin_offset` is written by the scan and read by the scatter, so
        // it is a read_write storage binding here.
        let bin_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render drc bin layout"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
                storage_entry(4, false),
                storage_entry(5, false),
            ],
        });
        let bin_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render drc bin pipeline layout"),
            bind_group_layouts: &[Some(&bin_layout)],
            immediate_size: 0,
        });

        let make_bin = |entry: &str, label: &str| {
            device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&bin_pipeline_layout),
                module: &bin_shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let count_pipeline = make_bin("count", "reticle-render drc count");
        let scan_pipeline = make_bin("scan", "reticle-render drc scan");
        let scatter_pipeline = make_bin("scatter", "reticle-render drc scatter");

        // Check bindings: params, instances, bin_count (read), bin_offset (read),
        // binned_index (read), violation_flags (write), heatmap (atomic write).
        let check_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render drc check layout"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, true),
                storage_entry(5, false),
                storage_entry(6, false),
            ],
        });
        let check_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render drc check pipeline layout"),
            bind_group_layouts: &[Some(&check_layout)],
            immediate_size: 0,
        });
        let check_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("reticle-render drc check"),
            layout: Some(&check_pipeline_layout),
            module: &check_shader,
            entry_point: Some("check"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            bin_layout,
            check_layout,
            count_pipeline,
            scan_pipeline,
            scatter_pipeline,
            check_pipeline,
        }
    }

    /// Runs the DRC heatmap over `instances` with `rules`, leaving the per-instance flag
    /// buffer and per-bin heatmap GPU-resident in a [`DrcOutput`].
    ///
    /// An empty input returns an output over a trivial `1 x 1` grid with a zero-length
    /// flag buffer, without dispatching.
    ///
    /// The input is capped to [`device_instance_cap`] instances (what a single compute
    /// dispatch and one storage binding hold on this device); a larger slice is processed
    /// over its leading `cap` instances rather than tripping wgpu validation. The overlay
    /// targets the visible (culled) instance set, which sits far below the cap in
    /// practice; the CPU `DrcEngine` remains the unbounded, authoritative oracle.
    #[must_use]
    pub fn run(
        &self,
        ctx: &WgpuContext,
        instances: &[RectInstanceT],
        rules: DrcRules,
    ) -> DrcOutput {
        self.run_with_cap(ctx, instances, rules, device_instance_cap(ctx.device()))
    }

    /// [`DrcHeatmap::run`] with an explicit instance `cap`, so a test can exercise the
    /// capping path without allocating a device-limit-sized input.
    #[must_use]
    fn run_with_cap(
        &self,
        ctx: &WgpuContext,
        instances: &[RectInstanceT],
        rules: DrcRules,
        cap: usize,
    ) -> DrcOutput {
        let device = ctx.device();
        let instances = &instances[..instances.len().min(cap)];
        let plan = plan_grid(instances, rules.min_spacing);
        let count = u32::try_from(instances.len()).unwrap_or(u32::MAX);
        let bins = plan.bins();

        let params = DrcParamsRaw {
            world_min: plan.world_min,
            grid: plan.grid,
            bin_size: plan.bin_size,
            count,
            min_width: rules.min_width,
            min_spacing: rules.min_spacing,
            _pad: [0; 3],
        };
        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render drc params"),
            contents: bytes_of(&params),
            usage: BufferUsages::UNIFORM,
        });

        let bufs = DrcBuffers::new(device, count, bins);
        if instances.is_empty() {
            return DrcOutput {
                flags: bufs.flags,
                heatmap: bufs.heatmap,
                grid: plan.grid,
                count,
            };
        }

        let instance_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render drc instances"),
            contents: cast_slice(instances),
            usage: BufferUsages::STORAGE,
        });
        let (bin_bind, check_bind) =
            self.bind_groups(device, &bufs, &params_buffer, &instance_buffer);

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render drc encoder"),
        });
        // Zero the accumulators the passes add into.
        encoder.clear_buffer(&bufs.bin_count, 0, None);
        encoder.clear_buffer(&bufs.heatmap, 0, None);

        let groups = count.div_ceil(WORKGROUP_SIZE);
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("reticle-render drc bin pass"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &bin_bind, &[]);
            pass.set_pipeline(&self.count_pipeline);
            pass.dispatch_workgroups(groups, 1, 1);
            pass.set_pipeline(&self.scan_pipeline);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.scatter_pipeline);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("reticle-render drc check pass"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &check_bind, &[]);
            pass.set_pipeline(&self.check_pipeline);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        ctx.queue().submit(std::iter::once(encoder.finish()));

        DrcOutput {
            flags: bufs.flags,
            heatmap: bufs.heatmap,
            grid: plan.grid,
            count,
        }
    }

    /// Builds the binning and check bind groups over one run's buffers, wiring each
    /// buffer to the binding its shader expects (see the layouts in [`DrcHeatmap::new`]).
    fn bind_groups(
        &self,
        device: &wgpu::Device,
        bufs: &DrcBuffers,
        params: &Buffer,
        instances: &Buffer,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let entry = |binding, resource| BindGroupEntry { binding, resource };
        let bin_bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render drc bin bind group"),
            layout: &self.bin_layout,
            entries: &[
                entry(0, params.as_entire_binding()),
                entry(1, instances.as_entire_binding()),
                entry(2, bufs.bin_count.as_entire_binding()),
                entry(3, bufs.bin_offset.as_entire_binding()),
                entry(4, bufs.bin_cursor.as_entire_binding()),
                entry(5, bufs.binned_index.as_entire_binding()),
            ],
        });
        let check_bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render drc check bind group"),
            layout: &self.check_layout,
            entries: &[
                entry(0, params.as_entire_binding()),
                entry(1, instances.as_entire_binding()),
                entry(2, bufs.bin_count.as_entire_binding()),
                entry(3, bufs.bin_offset.as_entire_binding()),
                entry(4, bufs.binned_index.as_entire_binding()),
                entry(5, bufs.flags.as_entire_binding()),
                entry(6, bufs.heatmap.as_entire_binding()),
            ],
        });
        (bin_bind, check_bind)
    }

    /// Reads the per-instance violation flags back to the CPU (one `u32` per instance).
    /// Blocks on the device, so it is a test/validation helper, not a per-frame path.
    #[must_use]
    pub fn read_flags(&self, ctx: &WgpuContext, out: &DrcOutput) -> Vec<u32> {
        if out.count == 0 {
            return Vec::new();
        }
        let size = u64::from(out.count) * std::mem::size_of::<u32>() as u64;
        read_u32_buffer(ctx, &out.flags, size)
    }

    /// Reads the per-bin heatmap back to the CPU (one `u32` per bin, row-major). Blocks
    /// on the device, so it is a test/validation helper.
    #[must_use]
    pub fn read_heatmap(&self, ctx: &WgpuContext, out: &DrcOutput) -> Vec<u32> {
        let bins = out.bin_count();
        if bins == 0 {
            return Vec::new();
        }
        let size = u64::from(bins) * std::mem::size_of::<u32>() as u64;
        read_u32_buffer(ctx, &out.heatmap, size)
    }
}

/// Copies `size` bytes of a storage buffer into a mappable readback buffer and returns
/// them as `u32`s, blocking on the device to drive the map to completion.
fn read_u32_buffer(ctx: &WgpuContext, src: &Buffer, size: u64) -> Vec<u32> {
    let device = ctx.device();
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("reticle-render drc readback"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("reticle-render drc readback encoder"),
    });
    encoder.copy_buffer_to_buffer(src, 0, &readback, 0, size);
    ctx.queue().submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(MapMode::Read, |_| {});
    let _ = device.poll(PollType::wait_indefinitely());
    let out = {
        let data = slice.get_mapped_range();
        cast_slice::<u8, u32>(&data).to_vec()
    };
    readback.unmap();
    out
}

/// The uniform block for the heatmap overlay. Matches `Overlay` in the inline overlay
/// shader; padded to 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
struct OverlayUniform {
    grid: [u32; 2],
    inv_max: f32,
    alpha: f32,
}

/// The WGSL for the heatmap overlay, inlined so the module keeps to its two named
/// shader files. Draws one alpha-blended quad per grid bin, colored by a heat ramp
/// scaled by the per-bin violation count; empty bins are discarded so the underlying
/// image shows through.
const OVERLAY_WGSL: &str = r"
struct Overlay { grid: vec2<u32>, inv_max: f32, alpha: f32 };
@group(0) @binding(0) var<uniform> ov: Overlay;
@group(0) @binding(1) var<storage, read> heat: array<u32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) intensity: f32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let bx = f32(inst % ov.grid.x);
    let by = f32(inst / ov.grid.x);
    let u = (bx + c.x) / f32(ov.grid.x);
    let v = (by + c.y) / f32(ov.grid.y);
    var o: VsOut;
    o.pos = vec4<f32>(u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0);
    o.intensity = f32(heat[inst]) * ov.inv_max;
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    if (in.intensity <= 0.0) {
        discard;
    }
    let t = clamp(in.intensity, 0.0, 1.0);
    let col = vec3<f32>(1.0, t * 0.6, 0.0);
    return vec4<f32>(col, ov.alpha);
}
";

/// A render pipeline that draws a [`DrcOutput`]'s coarse heatmap as an alpha-blended
/// overlay: one colored quad per grid bin, its intensity the bin's violation count
/// normalized by a caller-supplied maximum.
///
/// A UI rule-value slider drives the heatmap by re-running [`DrcHeatmap::run`] with a
/// new [`DrcRules`] and redrawing; the overlay itself just visualizes the current
/// heatmap buffer (kept GPU-resident, no readback). Wiring the slider into the
/// interactive app is a follow-up (the app crate is out of this module's scope).
pub struct DrcHeatmapOverlay {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
}

impl core::fmt::Debug for DrcHeatmapOverlay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrcHeatmapOverlay").finish_non_exhaustive()
    }
}

impl DrcHeatmapOverlay {
    /// Compiles the overlay render pipeline for a color `format` at `sample_count`.
    #[must_use]
    pub fn new(ctx: &WgpuContext, format: TextureFormat, sample_count: u32) -> Self {
        let device = ctx.device();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reticle-render drc overlay shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_WGSL.into()),
        });

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("reticle-render drc overlay layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("reticle-render drc overlay pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("reticle-render drc overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self { layout, pipeline }
    }

    /// Draws the heatmap over `target`, loading (not clearing) its existing contents so
    /// the overlay composites on top. `max_count` normalizes the heat ramp (the busiest
    /// bin's violation count is a natural choice); a zero or empty heatmap draws nothing.
    pub fn draw(
        &self,
        ctx: &WgpuContext,
        target: &OffscreenTarget,
        out: &DrcOutput,
        max_count: u32,
    ) {
        let bins = out.bin_count();
        if bins == 0 || max_count == 0 {
            return;
        }
        let device = ctx.device();
        let uniform = OverlayUniform {
            grid: out.grid,
            inv_max: 1.0 / max_count as f32,
            alpha: 0.6,
        };
        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("reticle-render drc overlay uniform"),
            contents: bytes_of(&uniform),
            usage: BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render drc overlay bind group"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: out.heatmap.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("reticle-render drc overlay encoder"),
        });
        {
            // Draw single-sampled into the resolved view; the overlay is a coarse grid
            // of axis-aligned quads that needs no MSAA.
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("reticle-render drc overlay pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..bins);
        }
        ctx.queue().submit(std::iter::once(encoder.finish()));
    }
}

/// The largest instance count [`DrcHeatmap::run`] processes on `device` in one dispatch.
///
/// Bounded by the storage-binding limit and max buffer size (the instance buffer is one
/// `size_of::<RectInstanceT>()`-stride storage binding) and by the compute dispatch count
/// (`max_compute_workgroups_per_dimension * WORKGROUP_SIZE` threads). Mirrors
/// `GpuHierarchy::derive_chunk_cap`; a larger input is capped, never panicking wgpu.
fn device_instance_cap(device: &wgpu::Device) -> usize {
    let limits = device.limits();
    let stride = core::mem::size_of::<RectInstanceT>() as u64;
    let by_binding = limits.max_storage_buffer_binding_size / stride;
    let by_buffer = limits.max_buffer_size / stride;
    let by_dispatch =
        u64::from(limits.max_compute_workgroups_per_dimension) * u64::from(WORKGROUP_SIZE);
    usize::try_from(by_binding.min(by_buffer).min(by_dispatch)).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(min: [f32; 2], max: [f32; 2]) -> RectInstanceT {
        RectInstanceT {
            min_xy: min,
            max_xy: max,
            color: [1.0, 1.0, 1.0, 1.0],
            orientation_code: 0,
            magnification: 1.0,
            translate: [0, 0],
        }
    }

    #[test]
    fn plan_grid_bin_size_covers_max_extent_plus_spacing() {
        // One big and one small instance spread over a wide world.
        let insts = [
            ident([0.0, 0.0], [40.0, 40.0]),
            ident([1000.0, 1000.0], [1010.0, 1010.0]),
        ];
        let plan = plan_grid(&insts, 10);
        // Bin size must be at least max_extent (40) + spacing (10) = 50 on each axis.
        assert!(plan.bin_size[0] >= 50, "bin {:?}", plan.bin_size);
        assert!(plan.bin_size[1] >= 50);
        assert!(plan.grid[0] >= 1 && plan.grid[0] <= MAX_GRID_AXIS);
        assert!(plan.grid[1] >= 1 && plan.grid[1] <= MAX_GRID_AXIS);
    }

    #[test]
    fn plan_grid_empty_is_trivial() {
        let plan = plan_grid(&[], 5);
        assert_eq!(plan.grid, [1, 1]);
    }

    #[test]
    fn run_caps_oversized_input_without_panicking() {
        let Some(ctx) = WgpuContext::new_blocking() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let heatmap = DrcHeatmap::new(&ctx);
        let insts: Vec<RectInstanceT> = (0..6)
            .map(|i| {
                let x = i as f32 * 100.0;
                ident([x, 0.0], [x + 10.0, 10.0])
            })
            .collect();
        let rules = DrcRules {
            min_width: 5,
            min_spacing: 20,
        };

        // A tiny artificial cap forces the clamp path: only the leading two instances are
        // processed, and the dispatch runs without tripping wgpu validation.
        let capped = heatmap.run_with_cap(&ctx, &insts, rules, 2);
        assert_eq!(capped.count, 2, "run_with_cap must clamp to `cap`");

        // The real device cap is positive and does not clamp a normal, small input.
        assert!(device_instance_cap(ctx.device()) >= insts.len());
        let full = heatmap.run(&ctx, &insts, rules);
        assert_eq!(full.count, insts.len() as u32);
    }

    /// The corners are exact integers below `2^24`, so `f32` holds them precisely;
    /// comparing as `i32` keeps the assertion exact without tripping the float-cmp lint.
    fn as_ints(v: [f32; 2]) -> [i32; 2] {
        [v[0] as i32, v[1] as i32]
    }

    #[test]
    fn world_aabb_identity_is_local_box() {
        let inst = ident([3.0, 4.0], [9.0, 20.0]);
        let (lo, hi) = world_aabb(&inst);
        assert_eq!(as_ints(lo), [3, 4]);
        assert_eq!(as_ints(hi), [9, 20]);
    }

    #[test]
    fn world_aabb_translate_shifts_box() {
        let mut inst = ident([0.0, 0.0], [10.0, 5.0]);
        inst.translate = [100, -50];
        let (lo, hi) = world_aabb(&inst);
        assert_eq!(as_ints(lo), [100, -50]);
        assert_eq!(as_ints(hi), [110, -45]);
    }
}

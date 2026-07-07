//! Fully GPU-resident hierarchy throughput at 30M and 100M flat-equivalent shapes.
//!
//! Builds a large arrayed scene  -  one leaf rect referenced by a single huge array (a
//! via/fill/bit-cell array, routine in real IC layout)  -  uploads it once as a compact
//! [`GpuHierarchy`] (one array record plus one cell, a few dozen bytes), and then each
//! frame runs the GPU expand + cull + compact pass and one indirect draw per chunk. The
//! CPU never expands, culls, or stores a per-element draw list.
//!
//! Two viewport regimes are timed per N, because they stress different things:
//!
//! - **zoomed-out (whole design):** every element is expanded, most survive the cull,
//!   and all are drawn  -  so this includes the full rasterization/overdraw cost of N
//!   sub-pixel quads. This is the worst case for the draw.
//! - **zoomed-in (a small window):** every element is still expanded and culled on the
//!   GPU each frame, but only a handful survive to be drawn. This isolates the
//!   expand+cull throughput  -  the honest "pan/zoom over a 100M design" number.
//!
//! A third figure, **expand-only**, times just the compute pass (forced to GPU
//! completion) with no draw or readback, so it reports the raw expand+cull rate in
//! elements/second.
//!
//! Offscreen frames read the pixels back to the CPU (a cost a surface-presenting loop
//! skips), so an on-screen interactive loop runs at or above the full-frame numbers.
//!
//! Run with `cargo run -p reticle-render --example gpu_hierarchy_bench --release`.

use std::time::{Duration, Instant};

use reticle_geometry::{Point, Rect};
use reticle_model::Camera;
use reticle_render::{
    ArrayPlacement, GpuHierarchy, IndirectRects, InstanceTransform, OffscreenTarget, Pipelines,
    RectInstance, Rgba, TARGET_FORMAT, ViewUniform, WgpuContext,
};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BufferDescriptor, BufferUsages, PollType,
};

/// The output resolution: 1080p, a realistic full-window frame (matches `fps_bench`).
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// Leaf rect edge (DBU) and the array pitch (DBU). Pitch > edge leaves gaps, so the
/// grid is a real field of distinct rects rather than one solid block.
const RECT_EDGE: f32 = 10.0;
const PITCH: i32 = 20;

/// Warmup frames rendered (and discarded) before timing.
const WARMUP: u32 = 3;

/// A camera framing `bbox` into `width` x `height` with a small margin (a local copy of
/// the CLI's `framing_camera`, matching `fps_bench`).
fn framing_camera(bbox: Rect, width: u32, height: u32) -> Camera {
    const MARGIN: f32 = 0.02;
    let cx = i64::midpoint(i64::from(bbox.min.x), i64::from(bbox.max.x)) as i32;
    let cy = i64::midpoint(i64::from(bbox.min.y), i64::from(bbox.max.y)) as i32;
    let center = Point::new(cx, cy);
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let span_x = bbox.width().max(1) as f32;
    let span_y = bbox.height().max(1) as f32;
    let ppd = ((w / span_x).min(h / span_y) * (1.0 - MARGIN)).max(f32::MIN_POSITIVE);
    let half_w = w / (2.0 * ppd);
    let half_h = h / (2.0 * ppd);
    let viewport = Rect::new(
        Point::new((cx as f32 - half_w) as i32, (cy as f32 - half_h) as i32),
        Point::new((cx as f32 + half_w) as i32, (cy as f32 + half_h) as i32),
    );
    Camera {
        center,
        pixels_per_dbu: ppd,
        viewport,
    }
}

/// Counts pixels differing from the top-left background (a non-blank check).
fn non_background_pixels(rgba: &[u8]) -> usize {
    if rgba.len() < 4 {
        return 0;
    }
    let bg = &rgba[0..4];
    rgba.chunks_exact(4).filter(|px| *px != bg).count()
}

/// fps and ms/frame for a timed loop.
fn report(label: &str, frames: u32, elapsed: Duration) {
    let secs = elapsed.as_secs_f64();
    let fps = if secs > 0.0 {
        f64::from(frames) / secs
    } else {
        f64::INFINITY
    };
    let ms = if frames > 0 {
        secs * 1000.0 / f64::from(frames)
    } else {
        0.0
    };
    println!("  {label:<34} {ms:>9.2} ms/frame  {fps:>9.2} fps  ({frames} frames)");
}

/// A square-ish grid `cols` x `rows` whose product is as close to `target` as possible.
fn grid_dims(target: u64) -> (u32, u32) {
    let side = (target as f64).sqrt() as u64;
    let cols = side.max(1);
    let rows = target.div_ceil(cols).max(1);
    (cols as u32, rows as u32)
}

/// Records one offscreen frame: the (already-run) GPU-resident scene drawn with one
/// indirect draw per chunk into `target`, then read back. Returns the read-back pixels.
fn draw_frame(
    ctx: &WgpuContext,
    hier: &GpuHierarchy,
    pipelines: &Pipelines,
    view_bg: &BindGroup,
    target: &OffscreenTarget,
    clear: Rgba,
) -> Vec<u8> {
    let device = ctx.device();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gpu-hierarchy bench encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gpu-hierarchy bench pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear.components[0]),
                        g: f64::from(clear.components[1]),
                        b: f64::from(clear.components[2]),
                        a: f64::from(clear.components[3]),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        hier.draw(&mut pass, pipelines.retained_rect_pipeline(), view_bg);
    }
    target.copy_to_buffer(&mut encoder);
    ctx.queue().submit(std::iter::once(encoder.finish()));
    target.read_pixels(ctx)
}

/// Benchmarks the GPU-resident hierarchy for a scene of `target` flat-equivalent shapes.
fn bench(ctx: &WgpuContext, target_count: u64) {
    let (cols, rows) = grid_dims(target_count);
    let elements = u64::from(cols) * u64::from(rows);
    println!(
        "\n== N = {elements} flat-equivalent shapes ({cols} x {rows} array of 1 leaf rect) =="
    );

    // The compact scene: one leaf cell, one giant array. Uploaded once.
    let cells = vec![RectInstance {
        min_xy: [0.0, 0.0],
        max_xy: [RECT_EDGE, RECT_EDGE],
        color: [0.2, 0.6, 1.0, 1.0],
    }];
    let placements = vec![ArrayPlacement::new(
        0,
        InstanceTransform::IDENTITY,
        cols,
        rows,
        PITCH,
        PITCH,
    )];

    let mut hier = GpuHierarchy::new(ctx);
    let upload_start = Instant::now();
    hier.upload(ctx, &cells, &placements);
    let upload_time = upload_start.elapsed();
    println!(
        "  uploaded compact scene ({} placement, {} cell) in {:?}; {} chunk(s), cap {} elements/chunk",
        placements.len(),
        cells.len(),
        upload_time,
        hier.chunk_count(),
        hier.max_chunk_elements()
    );

    // World bbox: the grid spans [0, cols*PITCH) x [0, rows*PITCH), plus the rect edge.
    let world = Rect::new(
        Point::new(0, 0),
        Point::new(
            cols as i32 * PITCH + RECT_EDGE as i32,
            rows as i32 * PITCH + RECT_EDGE as i32,
        ),
    );

    let pipelines = Pipelines::for_format(ctx.device(), TARGET_FORMAT);
    let target = OffscreenTarget::new(ctx, WIDTH, HEIGHT);
    let clear = Rgba {
        components: [0.0, 0.0, 0.0, 1.0],
    };

    // A persistent view uniform + bind group against the pipelines' shared layout.
    let view_buffer = ctx.device().create_buffer(&BufferDescriptor {
        label: Some("gpu-hierarchy bench view"),
        size: std::mem::size_of::<ViewUniform>() as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let view_bg = ctx.device().create_bind_group(&BindGroupDescriptor {
        label: Some("gpu-hierarchy bench view bg"),
        layout: pipelines.uniform_layout(),
        entries: &[BindGroupEntry {
            binding: 0,
            resource: view_buffer.as_entire_binding(),
        }],
    });

    // Frame counts scale down as N grows so wall-clock stays reasonable.
    let frames = if elements >= 100_000_000 {
        20
    } else if elements >= 30_000_000 {
        30
    } else {
        60
    };

    let b = Bencher {
        ctx,
        hier: &hier,
        pipelines: &pipelines,
        view_bg: &view_bg,
        view_buffer: &view_buffer,
        target: &target,
        clear,
    };

    // expand-only: pure GPU expand + cull of all N elements, forced to completion.
    b.expand_only(world, elements, frames);

    if !IndirectRects::supported(ctx) {
        println!("  note: adapter lacks INDIRECT_EXECUTION; drawing skipped");
        return;
    }

    // Zoomed-out: expand all, draw all, read back. Worst-case draw.
    b.full_frame(
        "zoomed-out (expand+draw+readback)",
        world,
        world,
        frames,
        true,
    );

    // Zoomed-in: a small window near the center; culls to a subset.
    let cx = world.max.x / 2;
    let cy = world.max.y / 2;
    let win = Rect::new(Point::new(cx, cy), Point::new(cx + 2000, cy + 2000));
    hier.expand(ctx, win);
    let kept = hier.read_survivor_count(ctx);
    b.full_frame("zoomed-in (expand+cull+draw few)", win, win, frames, false);
    println!("  survivors (zoomed-in window): {kept} of {elements} drawn");
}

/// The reusable per-scene state a timed regime needs.
struct Bencher<'a> {
    ctx: &'a WgpuContext,
    hier: &'a GpuHierarchy,
    pipelines: &'a Pipelines,
    view_bg: &'a BindGroup,
    view_buffer: &'a wgpu::Buffer,
    target: &'a OffscreenTarget,
    clear: Rgba,
}

impl Bencher<'_> {
    /// Times just the compute expand+cull pass over `viewport`, forced to GPU
    /// completion (no draw, no readback), and reports elements/second.
    fn expand_only(&self, viewport: Rect, elements: u64, frames: u32) {
        let run = || {
            self.hier.expand(self.ctx, viewport);
            let _ = self.ctx.device().poll(PollType::wait_indefinitely());
        };
        for _ in 0..WARMUP {
            run();
        }
        let start = Instant::now();
        for _ in 0..frames {
            run();
        }
        let elapsed = start.elapsed();
        report("expand+cull only (all visible)", frames, elapsed);
        let per_s = f64::from(frames) * elements as f64 / elapsed.as_secs_f64();
        println!(
            "  -> {:.1} M elements/s expanded + culled on the GPU",
            per_s / 1e6
        );
        println!(
            "  survivors (whole design): {} of {elements}",
            self.hier.read_survivor_count(self.ctx)
        );
    }

    /// Times a full offscreen frame (expand over `cull`, draw framed on `frame_bbox`,
    /// read back) under `label`. `report_lit` prints the first frame's lit-pixel count.
    fn full_frame(&self, label: &str, cull: Rect, frame_bbox: Rect, frames: u32, report_lit: bool) {
        let camera = framing_camera(frame_bbox, WIDTH, HEIGHT);
        let view = ViewUniform::from_camera(&camera, WIDTH, HEIGHT);
        self.ctx
            .queue()
            .write_buffer(self.view_buffer, 0, bytemuck::bytes_of(&view));
        let run = || {
            self.hier.expand(self.ctx, cull);
            draw_frame(
                self.ctx,
                self.hier,
                self.pipelines,
                self.view_bg,
                self.target,
                self.clear,
            )
        };
        let first = run();
        if report_lit {
            println!(
                "  first frame: {} of {} pixels lit (non-blank)",
                non_background_pixels(&first),
                (WIDTH * HEIGHT) as usize
            );
        }
        for _ in 1..WARMUP {
            let _ = run();
        }
        let start = Instant::now();
        for _ in 0..frames {
            std::hint::black_box(run().len());
        }
        report(label, frames, start.elapsed());
    }
}

fn main() {
    println!("== Reticle GPU-resident hierarchy benchmark ==");
    println!("resolution : {WIDTH}x{HEIGHT}");
    let Some(ctx) = WgpuContext::new_blocking() else {
        println!("no GPU adapter available, skipping");
        return;
    };
    let info = ctx.adapter().get_info();
    println!(
        "adapter    : {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );
    let limits = ctx.device().limits();
    println!(
        "limits     : max_storage_buffer_binding_size = {} MiB, max_compute_workgroups_per_dimension = {}",
        limits.max_storage_buffer_binding_size / (1024 * 1024),
        limits.max_compute_workgroups_per_dimension
    );
    println!(
        "chunk cap  : {} elements/chunk",
        GpuHierarchy::derive_chunk_cap(&ctx)
    );

    bench(&ctx, 30_000_000);
    bench(&ctx, 100_000_000);

    println!("\nnote: expand+cull runs every element on the GPU each frame; the CPU touches only");
    println!("the chunk list (a handful of entries), never a per-element draw list. Full-frame");
    println!("numbers include a blocking CPU readback a surface-presenting loop would skip.");
}

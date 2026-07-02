//! Headless offscreen render throughput (fps) benchmark.
//!
//! Builds a FLAT document of N leaf rectangles (a single top cell holding N axis
//! aligned rects on a few layers, positioned by a deterministic xorshift PRNG so the
//! run is reproducible without a `rand` dependency), frames a camera to fit them, and
//! times the offscreen GPU render path at 1920x1080 for N = 1,000,000 and
//! N = 10,000,000. It prints honest frames-per-second and ms/frame against the 60fps
//! (1M) and 30fps (10M) targets.
//!
//! Two numbers are reported per N, because they measure different things:
//!
//! - "full per-call" is [`WgpuRenderer::render_document_offscreen`], which rebuilds
//!   the pipelines, the offscreen target, the palette, and the whole `SceneGeometry`
//!   on every call (see that method's docs). So this number includes per-frame scene
//!   build and pipeline/target setup, not just the GPU draw.
//! - "reuse (draw+readback)" builds the `Pipelines`, `OffscreenTarget`, and
//!   `SceneGeometry` once and then re-times only `Pipelines::render` plus
//!   `read_pixels`. This is the steady-state interactive-loop cost: upload per-frame
//!   buffers, draw, copy, and read back.
//!
//! If no GPU adapter is available the benchmark prints a notice and exits 0 (a
//! graceful skip, not a failure), matching the CLI render path.
//!
//! Run with `cargo run -p reticle-render --example fps_bench --release`.

use std::time::Instant;

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Camera, Cell, Document, DrawShape, ShapeKind};
use reticle_render::{
    CellCompactor, CellCuller, CullAabb, ExpandedScene, OffscreenTarget, Palette, Pipelines,
    RetainedRenderer, RetainedScene, Rgba, SceneGeometry, TARGET_FORMAT, ViewUniform, WgpuContext,
    WgpuRenderer,
};

/// A tiny deterministic xorshift PRNG so the benchmark is reproducible without a
/// `rand` dependency (matching the crate's benches and the streaming demo).
struct XorShift(u64);

impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// The world spans `[-HALF, HALF)` in each axis (DBU). Leaf rects are scattered over
/// it and the camera is framed to fit the whole world.
const HALF: i32 = 5_000_000;

/// Bytes per instanced rectangle on the GPU (`RectInstance`: two `[f32; 2]` corners
/// plus one `[f32; 4]` color = 32 bytes). Used to size N against the device's
/// `max_buffer_size` so the single instance buffer the renderer builds is legal.
const RECT_INSTANCE_BYTES: u64 = 32;

/// The output resolution: 1080p, a realistic full-window frame.
const WIDTH: u32 = 1920;
/// The output resolution height.
const HEIGHT: u32 = 1080;

/// How many distinct layers the leaf rects are spread across. A handful keeps the
/// fallback palette varied without changing the shape count.
const LAYERS: u16 = 4;

/// Builds a FLAT document: one top cell named `top` holding `count` axis-aligned
/// rectangles scattered across the world on a few layers, positions and sizes from a
/// deterministic PRNG seeded by `seed`. Nothing is instanced or arrayed, so `count`
/// is the exact leaf shape count the renderer flattens and draws. Distinct seeds give
/// distinct (but still deterministic) shape sets, which is how a large N is split into
/// chunks that each fit a single GPU instance buffer.
fn build_flat_document(count: usize, seed: u64) -> Document {
    let mut rng = XorShift(seed);
    let mut cell = Cell::new("top");
    cell.shapes.reserve(count);

    let span = 2u64 * HALF as u64;
    for _ in 0..count {
        let x = ((rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        let y = ((rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        // Rects large enough (relative to world size) that many cover pixels at a
        // world-framed camera, so the frame is genuinely non-blank.
        let w = (rng.next_u64() % 4_000 + 500) as i32;
        let h = (rng.next_u64() % 4_000 + 500) as i32;
        let layer = (rng.next_u64() % u64::from(LAYERS)) as u16;
        cell.shapes.push(DrawShape::new(
            LayerId::new(layer, 0),
            ShapeKind::Rect(Rect::new(Point::new(x, y), Point::new(x + w, y + h))),
        ));
    }

    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_owned()]);
    doc
}

/// A camera that frames `bbox` into a `width` x `height` target with a small margin,
/// so the whole design is visible and centered. This mirrors the CLI's
/// `framing_camera` (kept local so the example does not depend on the CLI crate).
fn framing_camera(bbox: Rect, width: u32, height: u32) -> Camera {
    /// Fraction of the viewport left as empty margin around the design.
    const MARGIN: f32 = 0.05;

    let cx = i64::midpoint(i64::from(bbox.min.x), i64::from(bbox.max.x)) as i32;
    let cy = i64::midpoint(i64::from(bbox.min.y), i64::from(bbox.max.y)) as i32;
    let center = Point::new(cx, cy);

    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let span_x = bbox.width().max(1) as f32;
    let span_y = bbox.height().max(1) as f32;

    let fit_x = w / span_x;
    let fit_y = h / span_y;
    let ppd = (fit_x.min(fit_y) * (1.0 - MARGIN)).max(f32::MIN_POSITIVE);

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

/// Counts pixels in `rgba` whose color differs from the top-left pixel (taken as the
/// background). A non-blank frame has at least one, so the caller can assert the
/// render actually drew geometry rather than a cleared image.
fn non_background_pixels(rgba: &[u8]) -> usize {
    if rgba.len() < 4 {
        return 0;
    }
    let bg = &rgba[0..4];
    rgba.chunks_exact(4).filter(|px| *px != bg).count()
}

/// The timed result for one measurement path: the frame count and elapsed time turned
/// into fps and ms/frame.
struct Timing {
    frames: u32,
    fps: f64,
    ms_per_frame: f64,
}

impl Timing {
    fn new(frames: u32, elapsed: std::time::Duration) -> Self {
        let secs = elapsed.as_secs_f64();
        let fps = if secs > 0.0 {
            f64::from(frames) / secs
        } else {
            f64::INFINITY
        };
        let ms_per_frame = if frames > 0 {
            elapsed.as_secs_f64() * 1000.0 / f64::from(frames)
        } else {
            0.0
        };
        Self {
            frames,
            fps,
            ms_per_frame,
        }
    }
}

/// How many timed frames to render. 10M shapes are heavier, so fewer frames keep the
/// wall-clock reasonable while staying above the 20-frame floor asked for.
fn timed_frames(count: usize) -> u32 {
    if count >= 10_000_000 { 20 } else { 60 }
}

/// Number of warmup frames rendered (and discarded) before timing, to amortize
/// one-time costs (shader compile, allocation, first submit) and let the GPU settle.
const WARMUP_FRAMES: u32 = 3;

/// Splits `count` shapes into chunk sizes that each fit a single GPU instance buffer
/// under `max_rects_per_buffer`. Returns one entry per chunk (they sum to `count`).
/// For N at or below the cap this is a single chunk, so the common 1M case renders
/// exactly as the real one-shot path does.
fn chunk_sizes(count: usize, max_rects_per_buffer: usize) -> Vec<usize> {
    let cap = max_rects_per_buffer.max(1);
    let mut sizes = Vec::new();
    let mut remaining = count;
    while remaining > 0 {
        let this = remaining.min(cap);
        sizes.push(this);
        remaining -= this;
    }
    sizes
}

/// One rendered chunk's reusable state: its scene geometry and framing camera. The
/// pipelines and target are shared across chunks (they do not depend on shape data).
struct Chunk {
    geometry: SceneGeometry,
    camera: Camera,
}

/// Builds the per-chunk documents and scene geometry for a run of `count` shapes,
/// split so each chunk's instance buffer fits `max_rects_per_buffer`. Every chunk is
/// framed to the same whole-world camera so the chunks composite into one coherent
/// image. Returns the chunks plus the total build/flatten/tessellate wall time.
fn build_chunks(
    count: usize,
    max_rects_per_buffer: usize,
    palette: &Palette,
) -> (Vec<Chunk>, std::time::Duration) {
    let sizes = chunk_sizes(count, max_rects_per_buffer);
    let build_start = Instant::now();
    let mut chunks = Vec::with_capacity(sizes.len());
    for (i, &size) in sizes.iter().enumerate() {
        // Distinct seed per chunk so chunks hold distinct shapes; all share the same
        // world, so framing is identical.
        let seed = 0x9E37_79B9_7F4A_7C15 ^ (u64::from(i as u32).wrapping_mul(0x1000_0001B3));
        let doc = build_flat_document(size, seed);
        let bbox = doc
            .cell_bbox("top")
            .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)));
        let camera = framing_camera(bbox, WIDTH, HEIGHT);
        let shapes = doc.flatten("top");
        let geometry = SceneGeometry::build(&shapes, palette);
        chunks.push(Chunk { geometry, camera });
    }
    (chunks, build_start.elapsed())
}

/// Runs both measurement paths for a document of `count` leaf shapes and prints the
/// summary. `max_rects_per_buffer` is the largest instanced-rect count that fits one
/// GPU buffer on this device; N above it is rendered as several chunks that composite
/// onto the same target, and the reported per-frame time is the sum over all chunks
/// (the honest cost to push all N shapes through the renderer at this resolution).
fn bench_count(ctx: &WgpuContext, count: usize, target_fps: f64, max_rects_per_buffer: usize) {
    println!("\n== N = {count} leaf shapes ==");

    // An empty default technology: leaf layers resolve through the fallback palette
    // (all visible, opaque), which is all this synthetic geometry needs.
    let empty_doc = Document::new();
    let palette = Palette::from_technology(empty_doc.technology());
    let (chunks, build_time) = build_chunks(count, max_rects_per_buffer, &palette);
    let chunk_note = if chunks.len() == 1 {
        "single buffer".to_owned()
    } else {
        format!(
            "{} chunks (device max_buffer_size caps one instance buffer at {} rects)",
            chunks.len(),
            max_rects_per_buffer
        )
    };
    println!("built + tessellated flat geometry in {build_time:?}: {chunk_note}");

    let frames = timed_frames(count);
    let clear = Rgba {
        components: [0.0, 0.0, 0.0, 1.0],
    };

    // Shared pipelines and target: independent of the shape data, reused across every
    // chunk and every frame.
    let pipelines = Pipelines::new(ctx);
    let target = OffscreenTarget::new(ctx, WIDTH, HEIGHT);

    // ---- Path 1: full per-call render_document_offscreen (only when N fits one
    // buffer). This rebuilds pipelines, the target, the palette, and the whole
    // SceneGeometry every call, so the number includes per-frame scene build and
    // pipeline/target setup, not just the GPU draw. It cannot render a chunked N (its
    // single instance buffer would exceed max_buffer_size), so it is skipped there and
    // labeled as such.
    let full = if chunks.len() == 1 {
        let doc = build_flat_document(count, 0x9E37_79B9_7F4A_7C15);
        let camera = chunks[0].camera;
        let mut renderer = WgpuRenderer::new();

        // Warmup, and grab the first frame to cross-check it is non-blank.
        let first = renderer.render_document_offscreen(ctx, &doc, "top", &camera, (WIDTH, HEIGHT));
        let lit = non_background_pixels(&first);
        assert!(
            lit > 0,
            "first frame must be non-blank (found {lit} non-background pixels)"
        );
        println!(
            "first frame: {lit} of {} pixels differ from the top-left background (non-blank)",
            (WIDTH * HEIGHT) as usize
        );
        for _ in 1..WARMUP_FRAMES {
            let _ = renderer.render_document_offscreen(ctx, &doc, "top", &camera, (WIDTH, HEIGHT));
        }
        let start = Instant::now();
        for _ in 0..frames {
            let px = renderer.render_document_offscreen(ctx, &doc, "top", &camera, (WIDTH, HEIGHT));
            std::hint::black_box(px.len());
        }
        Some(Timing::new(frames, start.elapsed()))
    } else {
        None
    };

    // ---- Path 2: reuse pipelines + target + scene; time only draw + readback ----
    // Scene geometry, pipelines, and target are built once (above); this loop re-times
    // only Pipelines::render (per-frame buffer upload + draw + copy) plus read_pixels.
    // read_pixels blocks on the device (poll wait_indefinitely) and copies the frame
    // back to the CPU, which forces GPU completion each frame, so this is a true
    // wall-clock fps for the OFFSCREEN path. A surface-presenting interactive loop would
    // not pay the CPU readback, so it would run faster than this number; this is the
    // honest cost of the offscreen render-plus-readback path the crate actually exposes.
    //
    // For a chunked N this renders every chunk per frame. The public API always clears,
    // so a multi-chunk image is not composited (only the last chunk survives); the
    // TIMING, however, faithfully covers drawing and reading back all N shapes.
    let render_frame = || {
        for chunk in &chunks {
            let view = ViewUniform::from_camera(&chunk.camera, target.width(), target.height());
            pipelines.render(ctx, &target, &chunk.geometry, &view, clear);
            std::hint::black_box(target.read_pixels(ctx).len());
        }
    };

    for _ in 0..WARMUP_FRAMES {
        render_frame();
    }
    let start = Instant::now();
    for _ in 0..frames {
        render_frame();
    }
    let reuse = Timing::new(frames, start.elapsed());

    // For a single-chunk N, cross-check the reuse path is also non-blank.
    if chunks.len() == 1 {
        let view = ViewUniform::from_camera(&chunks[0].camera, target.width(), target.height());
        pipelines.render(ctx, &target, &chunks[0].geometry, &view, clear);
        let lit = non_background_pixels(&target.read_pixels(ctx));
        assert!(lit > 0, "reuse-path frame must be non-blank");
    }

    print_summary(
        count,
        target_fps,
        max_rects_per_buffer,
        chunks.len(),
        full.as_ref(),
        &reuse,
    );
}

/// Prints the per-N results block: the full per-call number (or why it was skipped)
/// and the reuse (draw + readback) number, each with its fps, ms/frame, and whether it
/// meets `target_fps`.
fn print_summary(
    count: usize,
    target_fps: f64,
    max_rects_per_buffer: usize,
    chunk_count: usize,
    full: Option<&Timing>,
    reuse: &Timing,
) {
    let meets = |fps: f64| if fps >= target_fps { "yes" } else { "no" };
    println!("---- results (N = {count}, {WIDTH}x{HEIGHT}) ----");
    match full {
        Some(full) => println!(
            "full per-call (rebuilds scene + pipelines + target each frame):\n\
             \x20  {:>7.2} ms/frame  {:>8.2} fps  over {} frames  | meets {:.0}fps target: {}",
            full.ms_per_frame,
            full.fps,
            full.frames,
            target_fps,
            meets(full.fps)
        ),
        None => println!(
            "full per-call: skipped. N exceeds the device max_buffer_size for a single\n\
             \x20  instance buffer ({max_rects_per_buffer} rects), which the one-shot API needs."
        ),
    }
    let chunk_label = if chunk_count == 1 {
        "draw + readback only, scene + pipelines built once".to_owned()
    } else {
        format!("draw + readback of all {count} shapes across {chunk_count} chunks per frame")
    };
    println!(
        "reuse ({}):\n\
         \x20  {:>7.2} ms/frame  {:>8.2} fps  over {} frames  | meets {:.0}fps target: {}",
        chunk_label,
        reuse.ms_per_frame,
        reuse.fps,
        reuse.frames,
        target_fps,
        meets(reuse.fps)
    );
    if chunk_count > 1 {
        println!(
            "  (multi-chunk: the composited image keeps only the last chunk because the\n\
             \x20  public render API always clears; the TIMING still covers all {count} shapes.)"
        );
    }
}

/// Records one offscreen frame drawing `renderer`'s retained scene into `target`,
/// updating only the camera uniform, then copies and reads it back. This is the
/// steady-state retained cost: no per-frame flatten, tessellate, or buffer creation.
fn draw_retained_frame(
    ctx: &WgpuContext,
    renderer: &RetainedRenderer,
    target: &OffscreenTarget,
    view: &ViewUniform,
    clear: Rgba,
) {
    let device = ctx.device();
    renderer.set_camera(ctx.queue(), view);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("retained bench encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("retained bench pass"),
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
        renderer.paint(&mut pass);
    }
    target.copy_to_buffer(&mut encoder);
    ctx.queue().submit(std::iter::once(encoder.finish()));
    std::hint::black_box(target.read_pixels(ctx).len());
}

/// Benchmarks the retained render path for `count` flat leaf shapes: build the
/// retained scene and upload it once, then time only the per-frame draw plus CPU
/// readback with a uniform-only camera update. This is the path the windowed surface
/// uses (minus the readback a real surface skips), so it is the honest steady-state
/// number for the new renderer.
fn bench_retained(ctx: &WgpuContext, count: usize, target_fps: f64) {
    println!("\n== retained path, N = {count} leaf shapes ==");
    let empty_doc = Document::new();
    let palette = Palette::from_technology(empty_doc.technology());

    // Build the flat document and its retained scene, then expand once (this is the
    // one-time cost the retained path pays on an edit, not per frame).
    let build_start = Instant::now();
    let doc = build_flat_document(count, 0x9E37_79B9_7F4A_7C15);
    let bbox = doc
        .cell_bbox("top")
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)));
    let camera = framing_camera(bbox, WIDTH, HEIGHT);
    let scene = RetainedScene::new(&doc, "top", &palette);
    let expanded: ExpandedScene = scene.expand();
    let build_time = build_start.elapsed();

    let mut renderer = RetainedRenderer::new(ctx.device(), TARGET_FORMAT);
    renderer.upload_expanded(ctx.device(), ctx.queue(), &expanded);
    println!(
        "built retained scene + expanded {} rect instances in {build_time:?}; {} page-sized draw(s)",
        expanded.rect_count(),
        renderer.rect_chunk_count()
    );

    let target = OffscreenTarget::new(ctx, WIDTH, HEIGHT);
    let view = ViewUniform::from_camera(&camera, target.width(), target.height());
    let clear = Rgba {
        components: [0.0, 0.0, 0.0, 1.0],
    };

    // Warmup, and cross-check the first frame is non-blank.
    draw_retained_frame(ctx, &renderer, &target, &view, clear);
    let first = target.read_pixels(ctx);
    let lit = non_background_pixels(&first);
    assert!(lit > 0, "retained frame must be non-blank (lit {lit})");
    for _ in 1..WARMUP_FRAMES {
        draw_retained_frame(ctx, &renderer, &target, &view, clear);
    }

    let frames = timed_frames(count);
    let start = Instant::now();
    for _ in 0..frames {
        draw_retained_frame(ctx, &renderer, &target, &view, clear);
    }
    let timing = Timing::new(frames, start.elapsed());
    let meets = if timing.fps >= target_fps {
        "yes"
    } else {
        "no"
    };
    println!("---- retained results (N = {count}, {WIDTH}x{HEIGHT}) ----");
    println!(
        "retained (upload once, then per-frame draw + readback):\n\
         \x20  {:>7.2} ms/frame  {:>8.2} fps  over {} frames  | meets {:.0}fps target: {}",
        timing.ms_per_frame, timing.fps, timing.frames, target_fps, meets
    );
}

/// Builds `count` axis-aligned cull boxes scattered across the world with the same
/// deterministic PRNG the render benches use, so the set is reproducible.
fn build_cull_boxes(count: usize, seed: u64) -> Vec<CullAabb> {
    let mut rng = XorShift(seed);
    let span = 2u64 * HALF as u64;
    let mut boxes = Vec::with_capacity(count);
    for _ in 0..count {
        let x = ((rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        let y = ((rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        let w = (rng.next_u64() % 4_000 + 500) as i32;
        let h = (rng.next_u64() % 4_000 + 500) as i32;
        boxes.push(CullAabb {
            min_xy: [x as f32, y as f32],
            max_xy: [(x + w) as f32, (y + h) as f32],
        });
    }
    boxes
}

/// Compares the two GPU cull stages head to head at `count` boxes: the flags-only
/// stage against the flags-plus-compaction stage.
///
/// * "flags only" is [`CellCuller::cull`]: it dispatches the overlap test, writes one
///   visibility flag per box, and reads all `count` flags back to the CPU. A
///   CPU-driven draw list then has to walk those flags itself.
/// * "cull + compact" runs the same cull, then [`CellCompactor::compact`] scans the
///   flags on the GPU into a dense survivor list and fills a `DrawIndexedIndirectArgs`;
///   only the tiny instance count is read back. This is what the GPU-driven draw list
///   uses, so its per-frame CPU cost is O(1) rather than O(count).
///
/// Both paths block to GPU completion each iteration (the flags path via its readback,
/// the compaction path via its count readback), so the milliseconds are honest
/// wall-clock. The viewport keeps roughly half the boxes, a realistic partial cull.
fn bench_flags_vs_compacted(ctx: &WgpuContext, requested: usize) {
    // The single-dispatch cull stage bounds N two ways, so clamp to the smaller rather
    // than tripping a validation error (chunking the cull is a separate follow-up):
    //   * it binds the box array as one storage buffer, capped by
    //     `max_storage_buffer_binding_size` (`CullAabb` is 16 bytes);
    //   * it dispatches one workgroup of 64 per 64 boxes, and the group count in a
    //     dimension is capped by `max_compute_workgroups_per_dimension`.
    // The compacted index buffer (4 bytes/entry) and the 256-wide compaction dispatch
    // are both looser, so the cull is the limit.
    /// The cull compute workgroup size; matches `WORKGROUP_SIZE` in `cull.wgsl`.
    const CULL_WORKGROUP_SIZE: usize = 64;
    let limits = ctx.device().limits();
    let cull_box_bytes = std::mem::size_of::<CullAabb>() as u64;
    let binding_cap = usize::try_from(limits.max_storage_buffer_binding_size / cull_box_bytes)
        .unwrap_or(usize::MAX);
    let dispatch_cap =
        (limits.max_compute_workgroups_per_dimension as usize).saturating_mul(CULL_WORKGROUP_SIZE);
    let cap = binding_cap.min(dispatch_cap);
    let count = requested.min(cap);
    if count < requested {
        println!(
            "\n== flags vs compacted, N = {count} cull boxes (capped from {requested} by the \
             single-dispatch cull limits: binding {binding_cap}, dispatch {dispatch_cap}) =="
        );
    } else {
        println!("\n== flags vs compacted, N = {count} cull boxes ==");
    }
    let boxes = build_cull_boxes(count, 0x5EED_1234_ABCD_0001);
    // A viewport over one quadrant keeps roughly a quarter to a half of the boxes.
    let viewport = Rect::new(Point::new(-HALF, -HALF), Point::new(0, HALF));

    let culler = CellCuller::new(ctx);
    let compactor = CellCompactor::new(ctx);

    // Warm up and record how many survive (a cross-check that the cull is non-trivial).
    let flags = culler.cull(ctx, &boxes, viewport);
    let survivors = flags.iter().filter(|&&f| f != 0).count();
    let (_dense, instance_count) = compactor.read_back(ctx, &compactor.compact(ctx, &flags));
    assert_eq!(
        instance_count as usize, survivors,
        "compacted instance_count must equal the survivor flags"
    );
    println!("survivors: {survivors} of {count} boxes kept by the viewport");

    // A handful of iterations; each blocks to GPU completion, so the wall-clock is real.
    let iters = if count >= 10_000_000 { 10 } else { 30 };
    for _ in 0..WARMUP_FRAMES {
        let f = culler.cull(ctx, &boxes, viewport);
        let _ = compactor.read_back(ctx, &compactor.compact(ctx, &f));
    }

    // Flags-only path.
    let start = Instant::now();
    for _ in 0..iters {
        let f = culler.cull(ctx, &boxes, viewport);
        std::hint::black_box(f.len());
    }
    let flags_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);

    // Cull + compaction path (compaction on precomputed GPU-shaped flags, tiny readback).
    let flags_fixed = culler.cull(ctx, &boxes, viewport);
    let start = Instant::now();
    for _ in 0..iters {
        let out = compactor.compact(ctx, &flags_fixed);
        let (_dense, n) = compactor.read_back(ctx, &out);
        std::hint::black_box(n);
    }
    let compact_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);

    println!("---- flags vs compacted results (N = {count}) ----");
    println!("flags only  (cull + read back {count} flags)   : {flags_ms:>8.3} ms/op");
    println!("compaction  (GPU scan -> draw args, count read): {compact_ms:>8.3} ms/op");
}

fn main() {
    println!("== Reticle offscreen render fps benchmark ==");
    println!("resolution       : {WIDTH}x{HEIGHT}");
    println!(
        "world            : [{}, {}) in each axis (DBU)",
        -HALF, HALF
    );
    println!("warmup frames    : {WARMUP_FRAMES}");

    let Some(ctx) = WgpuContext::new_blocking() else {
        println!("no GPU adapter available, skipping fps benchmark");
        return;
    };
    let info = ctx.adapter().get_info();
    println!(
        "adapter          : {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );

    // The renderer builds one instance buffer holding every rect, so N is bounded by
    // the device's max_buffer_size. Read the real limit and derive the per-buffer rect
    // cap; N above it is rendered in chunks (see bench_count).
    let max_buffer_size = ctx.device().limits().max_buffer_size;
    let max_rects_per_buffer = usize::try_from(max_buffer_size / RECT_INSTANCE_BYTES).unwrap_or(0);
    println!(
        "max_buffer_size  : {max_buffer_size} bytes ({max_rects_per_buffer} rects per instance buffer)"
    );

    bench_count(&ctx, 1_000_000, 60.0, max_rects_per_buffer);
    bench_count(&ctx, 10_000_000, 30.0, max_rects_per_buffer);

    // The retained path: the new renderer that caches tessellation and uploads once,
    // then redraws with only a uniform update. This is what the windowed surface runs.
    println!("\n=== retained (RetainedRenderer) path ===");
    bench_retained(&ctx, 1_000_000, 60.0);
    bench_retained(&ctx, 10_000_000, 30.0);

    // The GPU-driven draw list's cull stages: flags-only against flags-plus-compaction.
    // The larger N is bounded by the single-dispatch cull's storage-binding and
    // workgroup-count limits (see bench_flags_vs_compacted), so it uses 4,000,000.
    println!("\n=== GPU-driven cull: flags vs compacted ===");
    bench_flags_vs_compacted(&ctx, 1_000_000);
    bench_flags_vs_compacted(&ctx, 4_000_000);

    println!("\nnote: 'full per-call' includes per-frame scene build (flatten + tessellate) and");
    println!("pipeline/target setup; 'reuse' isolates the steady-state draw + CPU readback; the");
    println!("'retained' path uploads geometry once and then only redraws (a camera move is a");
    println!("uniform write). All read the frame back to the CPU (an offscreen cost a surface-");
    println!("presenting loop skips), so a real interactive path runs at or above these numbers.");
}

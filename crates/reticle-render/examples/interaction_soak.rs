//! Interaction soak: open a design once, then pan / zoom / redraw it many times and
//! assert the run is stable, no unbounded growth and no frame-time creep.
//!
//! This is the headless stand-in for a long editing session. The retained renderer
//! uploads the scene geometry to the GPU once; every subsequent frame is a
//! camera-uniform write plus a draw plus a CPU readback, so a correct implementation
//! holds its GPU buffers and per-frame cost flat no matter how long the user pans.
//! The soak drives that path in a tight loop and checks the invariants that would
//! break if a per-frame leak or rebuild crept in:
//!
//! * **Zero geometry-buffer growth.** The retained renderer's rect-instance count,
//!   page-sized chunk count (draw calls), and mesh-index count are sampled before the
//!   loop and after every frame; they must never change, because a pan or zoom must
//!   not re-upload or re-expand geometry. A regression that rebuilt the scene per
//!   frame (or leaked a page) would trip this immediately.
//! * **Bounded frame time.** No single frame may exceed `--frame-ceiling-ms`, and the
//!   mean of the last tenth of the run must not exceed the mean of the first tenth by
//!   more than `--drift-tolerance` (default 50%). Together these catch both a hard
//!   stall and a slow upward creep.
//!
//! Heap bytes are not sampled in-process (there is no allocator hook wired here); the
//! zero-growth proxy above is the retained buffer inventory, and the operator gets the
//! true peak working set by running this under `scripts/measure-run.ps1` (the soak
//! prints its own RSS-proxy note). A full multi-minute browser soak of the live wasm
//! pan path is an e2e/operator step, called out in `docs/PERF.md`.
//!
//! Usage:
//!
//! ```text
//! cargo run -p reticle-render --example interaction_soak --release -- \
//!     [--iters N] [--frame-ceiling-ms MS] [--drift-tolerance F] FILE
//! ```
//!
//! Exit code is 0 on a clean soak and non-zero (with a printed reason) if any
//! invariant is violated, so it can gate in a script.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use reticle_geometry::{Point, Rect};
use reticle_io::Gds;
use reticle_model::{Camera, Document, Importer};
use reticle_render::{
    ExpandedScene, OffscreenTarget, Palette, RetainedRenderer, RetainedScene, Rgba, TARGET_FORMAT,
    ViewUniform, WgpuContext,
};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const WARMUP_FRAMES: u32 = 3;
const CLEAR: Rgba = Rgba {
    components: [0.0, 0.0, 0.0, 1.0],
};

/// A camera framing `bbox` into `width` x `height` with a small margin.
fn framing_camera(bbox: Rect, width: u32, height: u32) -> Camera {
    const MARGIN: f32 = 0.05;
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

/// Derives a per-iteration camera that both pans (sweeping back and forth) and zooms
/// (breathing in and out), so the soak exercises the uniform-update path under varied
/// projections rather than redrawing one fixed frame.
fn soak_camera(base: &Camera, i: u32) -> Camera {
    let pan_phase = (i % 40) as f32 / 40.0; // 0..1
    let pan_frac = (pan_phase - 0.5) * 0.5; // -0.25..+0.25 of a viewport
    let vp_w = base.viewport.width().max(1) as f32;
    let vp_h = base.viewport.height().max(1) as f32;
    let dx = (vp_w * pan_frac) as i32;
    let dy = (vp_h * pan_frac * 0.5) as i32;

    // Zoom breathes between 0.6x and 1.4x of the base scale on a slower cycle.
    let zoom_phase = (i % 73) as f32 / 73.0;
    let zoom = 1.0 + 0.4 * (zoom_phase * std::f32::consts::TAU).sin();
    let ppd = (base.pixels_per_dbu * zoom).max(f32::MIN_POSITIVE);

    Camera {
        center: Point::new(
            base.center.x.wrapping_add(dx),
            base.center.y.wrapping_add(dy),
        ),
        pixels_per_dbu: ppd,
        viewport: Rect::new(
            Point::new(base.viewport.min.x + dx, base.viewport.min.y + dy),
            Point::new(base.viewport.max.x + dx, base.viewport.max.y + dy),
        ),
    }
}

/// One retained offscreen frame: camera-uniform write, draw, copy, readback.
fn draw_frame(
    ctx: &WgpuContext,
    renderer: &RetainedRenderer,
    target: &OffscreenTarget,
    view: &ViewUniform,
) {
    renderer.set_camera(ctx.queue(), view);
    let mut encoder = ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("interaction_soak encoder"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("interaction_soak pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(CLEAR.components[0]),
                        g: f64::from(CLEAR.components[1]),
                        b: f64::from(CLEAR.components[2]),
                        a: f64::from(CLEAR.components[3]),
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

/// Parsed CLI arguments.
struct Args {
    file: String,
    iters: u32,
    frame_ceiling_ms: f64,
    drift_tolerance: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut file: Option<String> = None;
    let mut iters = 2000u32;
    let mut frame_ceiling_ms = 100.0f64;
    let mut drift_tolerance = 0.5f64;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--iters" => {
                iters = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--iters needs a number")?;
            }
            "--frame-ceiling-ms" => {
                frame_ceiling_ms = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--frame-ceiling-ms needs a number")?;
            }
            "--drift-tolerance" => {
                drift_tolerance = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--drift-tolerance needs a number")?;
            }
            other => file = Some(other.to_owned()),
        }
    }
    Ok(Args {
        file: file.ok_or("a FILE argument is required")?,
        iters,
        frame_ceiling_ms,
        drift_tolerance,
    })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Drives the timed pan/zoom loop and checks the invariants, printing the results.
///
/// Returns `Ok(())` on a clean soak or `Err(reason)` on the first violation (a
/// geometry-buffer change, a frame over the ceiling, or upward frame-time drift).
fn run_soak(
    ctx: &WgpuContext,
    renderer: &RetainedRenderer,
    target: &OffscreenTarget,
    base: &Camera,
    args: &Args,
) -> Result<(), String> {
    let base_rects = renderer.rect_count();
    let base_chunks = renderer.rect_chunk_count();
    let base_indices = renderer.index_count();

    for i in 0..WARMUP_FRAMES {
        let view = ViewUniform::from_camera(&soak_camera(base, i), target.width(), target.height());
        draw_frame(ctx, renderer, target, &view);
    }

    let mut frame_ms: Vec<f64> = Vec::with_capacity(args.iters as usize);
    let soak_start = Instant::now();
    for i in 0..args.iters {
        let view = ViewUniform::from_camera(&soak_camera(base, i), target.width(), target.height());
        let t = Instant::now();
        draw_frame(ctx, renderer, target, &view);
        let dt = ms(t.elapsed());
        frame_ms.push(dt);

        // Zero-growth check every frame: the retained buffers must not change.
        if renderer.rect_count() != base_rects
            || renderer.rect_chunk_count() != base_chunks
            || renderer.index_count() != base_indices
        {
            return Err(format!(
                "geometry buffers grew at frame {i}: rects {}->{}, chunks {}->{}, indices {}->{}",
                base_rects,
                renderer.rect_count(),
                base_chunks,
                renderer.rect_chunk_count(),
                base_indices,
                renderer.index_count(),
            ));
        }
        if dt > args.frame_ceiling_ms {
            return Err(format!(
                "frame {i} took {dt:.3} ms, over the {:.1} ms ceiling",
                args.frame_ceiling_ms
            ));
        }
    }
    let soak_wall = soak_start.elapsed();

    // Drift check: mean of the last tenth vs the first tenth.
    let n = frame_ms.len();
    let decile = (n / 10).max(1);
    let mean = |slice: &[f64]| slice.iter().sum::<f64>() / slice.len() as f64;
    let first = mean(&frame_ms[..decile]);
    let last = mean(&frame_ms[n - decile..]);
    let mut sorted = frame_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = sorted[n / 2];
    let p99 = sorted[(n * 99 / 100).min(n - 1)];
    let max = sorted[n - 1];

    println!("---- soak results ----");
    println!("frames         : {n}");
    println!("wall           : {:.2} s", soak_wall.as_secs_f64());
    println!("frame p50      : {p50:.3} ms");
    println!("frame p99      : {p99:.3} ms");
    println!(
        "frame max      : {max:.3} ms  (ceiling {:.1} ms)",
        args.frame_ceiling_ms
    );
    println!("first-decile mean: {first:.3} ms");
    println!("last-decile  mean: {last:.3} ms");
    println!(
        "geometry buffers: rects {base_rects}, chunks {base_chunks}, indices {base_indices} (unchanged across all {n} frames)"
    );
    println!(
        "note: peak process working set (heap proxy) is reported when this runs under scripts/measure-run.ps1."
    );

    let drift_ceiling = first * (1.0 + args.drift_tolerance);
    if last > drift_ceiling {
        return Err(format!(
            "last-decile mean {last:.3} ms exceeds first-decile mean {first:.3} ms by more than {:.0}% (ceiling {drift_ceiling:.3} ms): frame time is creeping upward",
            args.drift_tolerance * 100.0
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: cargo run -p reticle-render --example interaction_soak --release -- \
                 [--iters N] [--frame-ceiling-ms MS] [--drift-tolerance F] FILE"
            );
            return ExitCode::FAILURE;
        }
    };

    println!("== Reticle interaction soak ==");
    println!("file           : {}", args.file);
    println!("iters          : {}", args.iters);
    println!("frame ceiling  : {:.1} ms", args.frame_ceiling_ms);
    println!("drift tolerance: {:.0}%", args.drift_tolerance * 100.0);

    let bytes = match std::fs::read(&args.file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: read {}: {e}", args.file);
            return ExitCode::FAILURE;
        }
    };
    let doc = match Gds.import(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: parse {}: {e}", args.file);
            return ExitCode::FAILURE;
        }
    };
    let top = doc
        .top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().map(|c| c.name.clone()).min())
        .unwrap_or_else(|| "top".to_owned());
    let bbox = doc
        .cell_bbox(&top)
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)));
    let base = framing_camera(bbox, WIDTH, HEIGHT);

    let Some(ctx) = WgpuContext::new_blocking() else {
        println!("no GPU adapter available; soak needs a GPU. Skipping (exit 0).");
        return ExitCode::SUCCESS;
    };
    let info = ctx.adapter().get_info();
    println!("adapter        : {} ({:?})", info.name, info.backend);

    // Build and upload the retained scene ONCE. This is the only geometry upload the
    // whole soak performs; every frame after is uniform-only.
    let empty = Document::new();
    let palette = Palette::from_technology(empty.technology());
    let scene = RetainedScene::new(&doc, &top, &palette);
    let expanded: ExpandedScene = scene.expand();
    let mut renderer = RetainedRenderer::new(ctx.device(), TARGET_FORMAT);
    renderer.upload_expanded(ctx.device(), ctx.queue(), &expanded);

    // The zero-growth baseline: the retained buffer inventory after the single upload.
    let base_rects = renderer.rect_count();
    let base_chunks = renderer.rect_chunk_count();
    let base_indices = renderer.index_count();
    println!(
        "uploaded once  : {base_rects} rect instances, {base_chunks} page chunk(s), {base_indices} mesh indices",
    );

    let target = OffscreenTarget::new(&ctx, WIDTH, HEIGHT);

    match run_soak(&ctx, &renderer, &target, &base, &args) {
        Ok(()) => {
            println!("SOAK OK: no buffer growth, no frame over ceiling, no upward drift.");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("SOAK FAILED: {reason}");
            ExitCode::FAILURE
        }
    }
}

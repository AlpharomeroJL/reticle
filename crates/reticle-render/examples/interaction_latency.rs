//! Interaction-latency harness: open a real design, render its first frame, and
//! pan/redraw under load, timing each phase separately on this host.
//!
//! Where `fps_bench` measures synthetic FLAT documents at steady state, this harness
//! measures the *product interaction path* on real, hierarchical designs: the cost a
//! user actually pays to open a file and start moving around in it. It is headless
//! and deterministic (no window, offscreen render plus CPU readback), so the numbers
//! reproduce and can be recorded in `docs/PERF.md`.
//!
//! For each input design it reports, as wall-clock milliseconds:
//!
//! * **open**: read the file, parse it into a [`Document`], compute the framing
//!   bounding box, and flatten the top cell to leaf shapes. This is the pure-CPU
//!   "open the document" cost, split into its parse / bbox / flatten parts so a
//!   redundant traversal is visible.
//! * **first frame (one-shot)**: [`WgpuRenderer::render_document_offscreen`], which
//!   is the CLI `render` path: it rebuilds pipelines, target, palette, and the whole
//!   [`SceneGeometry`] and reads the frame back. This is what "open then see the
//!   first picture" costs through the one-shot API.
//! * **scene build + upload (retained)**: build the [`RetainedScene`], expand it, and
//!   upload it to the GPU once. This is the one-time cost the interactive editor pays
//!   on open (and on an edit), after which pans are uniform-only.
//! * **pan/redraw (retained)**: with geometry resident, shift the camera and redraw
//!   `--iters` times, each a camera-uniform write plus draw plus CPU readback (the
//!   readback is an offscreen cost a live surface skips). This is the steady-state
//!   interaction latency per frame.
//!
//! Usage:
//!
//! ```text
//! cargo run -p reticle-render --example interaction_latency --release -- \
//!     [--iters N] FILE [FILE ...]
//! ```
//!
//! With no FILE arguments it prints usage and exits 0. If no GPU adapter is available
//! the GPU phases are skipped (the CPU-open numbers still print), matching the CLI.

use std::path::Path;
use std::time::{Duration, Instant};

use reticle_geometry::{Point, Rect};
use reticle_io::Gds;
use reticle_model::{Camera, Document, Importer};
use reticle_render::{
    ExpandedScene, OffscreenTarget, Palette, RetainedRenderer, RetainedScene, Rgba, SceneGeometry,
    TARGET_FORMAT, ViewUniform, WgpuContext, WgpuRenderer,
};

/// The offscreen resolution: a realistic full-window 1080p frame.
const WIDTH: u32 = 1920;
/// The offscreen resolution height.
const HEIGHT: u32 = 1080;

/// Frames rendered and discarded before timing, to amortize one-time GPU costs
/// (shader compile, first submit) and let the device settle.
const WARMUP_FRAMES: u32 = 3;

/// The opaque-black clear color for offscreen frames.
const CLEAR: Rgba = Rgba {
    components: [0.0, 0.0, 0.0, 1.0],
};

/// A camera that frames `bbox` into `width` x `height` with a small margin, so the
/// whole design is visible and centered. Mirrors the CLI's `framing_camera`.
fn framing_camera(bbox: Rect, width: u32, height: u32) -> Camera {
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

/// The bounding box a document's top cell frames into, or a unit box if empty.
fn frame_bbox(doc: &Document, top: &str) -> Rect {
    doc.cell_bbox(top)
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)))
}

/// Picks the top cell the way the CLI does: the first declared top, else the
/// lexicographically first cell, else `"top"` for an empty document.
fn pick_top(doc: &Document) -> String {
    if let Some(t) = doc.top_cells().first() {
        return t.clone();
    }
    doc.cells()
        .map(|c| c.name.clone())
        .min()
        .unwrap_or_else(|| "top".to_owned())
}

/// The CPU-side "open" phases and their timings, plus the parsed document, the top
/// cell name, the framing camera, and the flattened shape count.
struct Opened {
    doc: Document,
    top: String,
    camera: Camera,
    leaf_count: usize,
    parse: Duration,
    bbox: Duration,
    flatten: Duration,
}

/// Reads and parses `path`, then times the bbox and flatten traversals. GDS only
/// (every committed corpus/example design and the generator output is GDSII).
fn open_design(path: &Path) -> Result<Opened, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;

    let t = Instant::now();
    let doc = Gds
        .import(&bytes)
        .map_err(|e| format!("parse {}: {e}", path.display()))?;
    let parse = t.elapsed();

    let top = pick_top(&doc);

    let t = Instant::now();
    let bbox = frame_bbox(&doc, &top);
    let bbox_time = t.elapsed();
    let camera = framing_camera(bbox, WIDTH, HEIGHT);

    let t = Instant::now();
    let shapes = doc.flatten(&top);
    let flatten = t.elapsed();
    let leaf_count = shapes.len();

    Ok(Opened {
        doc,
        top,
        camera,
        leaf_count,
        parse,
        bbox: bbox_time,
        flatten,
    })
}

/// Times `render_document_offscreen` (the CLI one-shot path) after a warmup, and
/// asserts the first frame is non-blank. Returns the median-ish steady per-call time
/// (mean over a few timed calls).
fn time_first_frame_oneshot(ctx: &WgpuContext, opened: &Opened) -> Duration {
    let mut renderer = WgpuRenderer::new();
    // First frame (also warmup).
    let first = renderer.render_document_offscreen(
        ctx,
        &opened.doc,
        &opened.top,
        &opened.camera,
        (WIDTH, HEIGHT),
    );
    let lit = non_background_pixels(&first);
    assert!(
        lit > 0,
        "one-shot first frame must be non-blank (lit {lit})"
    );
    for _ in 1..WARMUP_FRAMES {
        let _ = renderer.render_document_offscreen(
            ctx,
            &opened.doc,
            &opened.top,
            &opened.camera,
            (WIDTH, HEIGHT),
        );
    }
    let frames = 5u32;
    let start = Instant::now();
    for _ in 0..frames {
        let px = renderer.render_document_offscreen(
            ctx,
            &opened.doc,
            &opened.top,
            &opened.camera,
            (WIDTH, HEIGHT),
        );
        std::hint::black_box(px.len());
    }
    start.elapsed() / frames
}

/// Counts pixels differing from the top-left background, so callers can assert a
/// frame actually drew geometry.
fn non_background_pixels(rgba: &[u8]) -> usize {
    if rgba.len() < 4 {
        return 0;
    }
    let bg = &rgba[0..4];
    rgba.chunks_exact(4).filter(|px| *px != bg).count()
}

/// The retained-path measurement: the one-time scene build + upload cost, and the
/// steady per-frame pan/redraw cost with geometry resident.
struct Retained {
    build_upload: Duration,
    rect_instances: usize,
    chunk_count: usize,
    pan_frame_avg: Duration,
    pan_frame_max: Duration,
}

/// Records one offscreen retained frame at `view`: rewrite the camera uniform, draw,
/// copy, and read back (forcing GPU completion so the timing is honest wall-clock).
fn draw_retained_frame(
    ctx: &WgpuContext,
    renderer: &RetainedRenderer,
    target: &OffscreenTarget,
    view: &ViewUniform,
) {
    renderer.set_camera(ctx.queue(), view);
    let mut encoder = ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("interaction_latency retained encoder"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("interaction_latency retained pass"),
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

/// A camera panned by `frac` of its viewport width to the right and `frac/2` up, so
/// each pan iteration shows a genuinely different view (not a repeated draw of the
/// same framing that a driver might elide).
fn panned(camera: &Camera, frac: f32) -> Camera {
    let vp_w = camera.viewport.width().max(1) as f32;
    let vp_h = camera.viewport.height().max(1) as f32;
    let dx = (vp_w * frac) as i32;
    let dy = (vp_h * frac * 0.5) as i32;
    let center = Point::new(
        camera.center.x.wrapping_add(dx),
        camera.center.y.wrapping_add(dy),
    );
    Camera {
        center,
        pixels_per_dbu: camera.pixels_per_dbu,
        viewport: Rect::new(
            Point::new(camera.viewport.min.x + dx, camera.viewport.min.y + dy),
            Point::new(camera.viewport.max.x + dx, camera.viewport.max.y + dy),
        ),
    }
}

/// Builds the retained scene, uploads once, then pans/redraws `iters` times, timing
/// the per-frame cost.
fn measure_retained(ctx: &WgpuContext, opened: &Opened, iters: u32) -> Retained {
    let empty = Document::new();
    let palette = Palette::from_technology(empty.technology());

    let start = Instant::now();
    let scene = RetainedScene::new(&opened.doc, &opened.top, &palette);
    let expanded: ExpandedScene = scene.expand();
    let mut renderer = RetainedRenderer::new(ctx.device(), TARGET_FORMAT);
    renderer.upload_expanded(ctx.device(), ctx.queue(), &expanded);
    let build_upload = start.elapsed();

    let rect_instances = expanded.rect_count();
    let chunk_count = renderer.rect_chunk_count();

    let target = OffscreenTarget::new(ctx, WIDTH, HEIGHT);

    // Warmup at the base framing.
    let base_view = ViewUniform::from_camera(&opened.camera, target.width(), target.height());
    draw_retained_frame(ctx, &renderer, &target, &base_view);
    let first = target.read_pixels(ctx);
    // A design with only rects that all fall off-frame is unlikely for a framed
    // camera; assert non-blank so a broken upload is caught. Allow blank only when
    // there is genuinely nothing to draw.
    if rect_instances > 0 || !expanded_mesh_empty(&expanded) {
        let lit = non_background_pixels(&first);
        assert!(
            lit > 0,
            "retained first frame must be non-blank (lit {lit})"
        );
    }
    for _ in 1..WARMUP_FRAMES {
        draw_retained_frame(ctx, &renderer, &target, &base_view);
    }

    // Timed pan loop: each frame shifts the camera a little, so the sequence sweeps
    // across the design rather than redrawing one fixed view.
    let mut max = Duration::ZERO;
    let start = Instant::now();
    for i in 0..iters {
        // Oscillate the pan fraction so the camera sweeps back and forth over the
        // design and never wanders arbitrarily far from the framed geometry.
        let phase = (i % 40) as f32 / 40.0; // 0..1
        let frac = (phase - 0.5) * 0.5; // -0.25 .. +0.25 of a viewport
        let cam = panned(&opened.camera, frac);
        let view = ViewUniform::from_camera(&cam, target.width(), target.height());
        let t = Instant::now();
        draw_retained_frame(ctx, &renderer, &target, &view);
        let dt = t.elapsed();
        if dt > max {
            max = dt;
        }
    }
    let pan_frame_avg = start.elapsed() / iters.max(1);

    Retained {
        build_upload,
        rect_instances,
        chunk_count,
        pan_frame_avg,
        pan_frame_max: max,
    }
}

/// Whether the expanded scene has no tessellated mesh geometry.
fn expanded_mesh_empty(expanded: &ExpandedScene) -> bool {
    // `ExpandedScene` exposes rect_count; a scene with rects is non-empty. For the
    // mesh-only case we conservatively treat "no rects" as possibly-empty and let the
    // caller's assertion be skipped, so a legitimately mesh-only design does not trip
    // a false "blank frame" panic. Rects are the common case for the corpus.
    expanded.rect_count() == 0
}

/// Milliseconds, three decimals.
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn report(path: &Path, opened: &Opened, first_frame: Option<Duration>, retained: Option<Retained>) {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    println!("\n=== {name} ===");
    println!("  top cell        : {}", opened.top);
    println!("  flattened leaves: {}", opened.leaf_count);
    println!(
        "  open (CPU)      : {:>8.3} ms total  (parse {:.3} + bbox {:.3} + flatten {:.3})",
        ms(opened.parse) + ms(opened.bbox) + ms(opened.flatten),
        ms(opened.parse),
        ms(opened.bbox),
        ms(opened.flatten),
    );
    match first_frame {
        Some(d) => println!(
            "  first frame     : {:>8.3} ms  (one-shot render_document_offscreen, {WIDTH}x{HEIGHT})",
            ms(d)
        ),
        None => println!("  first frame     : (no GPU, skipped)"),
    }
    match retained {
        Some(r) => {
            println!(
                "  scene build+up  : {:>8.3} ms  ({} rect instances, {} page draw(s))",
                ms(r.build_upload),
                r.rect_instances,
                r.chunk_count,
            );
            println!(
                "  pan/redraw      : {:>8.3} ms/frame avg, {:.3} ms max  (retained, camera-uniform + draw + readback)",
                ms(r.pan_frame_avg),
                ms(r.pan_frame_max),
            );
        }
        None => println!("  retained path   : (no GPU, skipped)"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut iters = 200u32;
    let mut files: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--iters" => {
                iters = args.next().and_then(|v| v.parse().ok()).unwrap_or(iters);
            }
            other => files.push(other.to_owned()),
        }
    }

    if files.is_empty() {
        println!(
            "usage: cargo run -p reticle-render --example interaction_latency --release -- \
             [--iters N] FILE [FILE ...]"
        );
        println!("measures open / first-frame / pan-redraw latency on real designs.");
        return;
    }

    println!("== Reticle interaction-latency harness ==");
    println!("resolution : {WIDTH}x{HEIGHT}");
    println!("pan iters  : {iters}");

    // Parse every design first (CPU only), so the open numbers are recorded even if
    // no GPU is available.
    let mut opened: Vec<(String, Opened)> = Vec::new();
    for f in &files {
        match open_design(Path::new(f)) {
            Ok(o) => opened.push((f.clone(), o)),
            Err(e) => eprintln!("skip {f}: {e}"),
        }
    }

    let ctx = WgpuContext::new_blocking();
    match &ctx {
        Some(ctx) => {
            let info = ctx.adapter().get_info();
            println!(
                "adapter    : {} ({:?}, {:?})",
                info.name, info.backend, info.device_type
            );
        }
        None => println!("adapter    : none (GPU phases skipped)"),
    }

    for (f, o) in &opened {
        let (first_frame, retained) = match &ctx {
            Some(ctx) => (
                Some(time_first_frame_oneshot(ctx, o)),
                Some(measure_retained(ctx, o, iters)),
            ),
            None => (None, None),
        };
        report(Path::new(f), o, first_frame, retained);
    }

    // Keep SceneGeometry in the linked set so the import is not flagged unused when
    // the GPU path is compiled out on some configs; a no-op reference.
    let _ = std::mem::size_of::<SceneGeometry>();
}

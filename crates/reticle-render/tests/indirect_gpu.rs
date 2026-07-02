//! Offscreen equality test for the GPU-driven indirect draw path.
//!
//! Renders the same set of rectangle instances two ways into single-sample offscreen
//! targets and asserts the images match within tolerance:
//!
//! * the direct path: [`RetainedRenderer`] issues `draw(0..4, 0..count)` with a
//!   CPU-known instance count;
//! * the indirect path: [`CellCuller`] flags every instance visible, [`CellCompactor`]
//!   compacts the survivors and fills a `DrawIndexedIndirectArgs`, and
//!   [`IndirectRects`] issues `draw_indexed_indirect` whose instance count comes from
//!   the GPU.
//!
//! With a viewport covering every instance, compaction keeps them all, so the indirect
//! image must reproduce the direct one. Both render single-sampled, so the only
//! difference under test is direct-instanced vs indirect-indexed-instanced drawing.
//!
//! Skips (and passes) without a usable GPU adapter, or when the adapter cannot execute
//! indirect draws at all (the WebGL2 fallback, where the direct path is the only
//! option).

use reticle_geometry::{Point, Rect};
use reticle_model::Camera;
use reticle_render::{
    CellCompactor, CellCuller, CullAabb, ExpandedScene, IndirectRects, MultiDraw, OffscreenTarget,
    RectInstanceT, RetainedRenderer, Rgba, TARGET_FORMAT, ViewUniform, WgpuContext,
    upload_instances,
};

const SIZE: u32 = 96;

/// The world the camera frames: `[0, 96]^2` in DBU at 1 px/DBU.
const WORLD: Rect = Rect {
    min: Point { x: 0, y: 0 },
    max: Point { x: 96, y: 96 },
};

/// A solid opaque color from packed `0xRRGGBBAA`.
fn color(packed: u32) -> [f32; 4] {
    Rgba::from_packed(packed).components
}

/// A world-space rectangle instance at identity placement (no orientation, unit scale,
/// no translation), so its `min_xy`/`max_xy` are already world coordinates.
fn rect(x0: f32, y0: f32, x1: f32, y1: f32, packed: u32) -> RectInstanceT {
    RectInstanceT {
        min_xy: [x0, y0],
        max_xy: [x1, y1],
        color: color(packed),
        orientation_code: 0,
        magnification: 1.0,
        translate: [0, 0],
    }
}

/// A handful of disjoint colored rectangles scattered over the world.
fn scene() -> Vec<RectInstanceT> {
    vec![
        rect(8.0, 8.0, 28.0, 28.0, 0xff00_00ff),   // red, lower-left
        rect(40.0, 12.0, 60.0, 32.0, 0x00ff_00ff), // green
        rect(66.0, 40.0, 86.0, 60.0, 0x0000_ffff), // blue
        rect(16.0, 62.0, 36.0, 82.0, 0xffff_00ff), // yellow, upper-left
        rect(50.0, 66.0, 70.0, 86.0, 0x00ff_ffff), // cyan
    ]
}

/// The framing camera: centers `WORLD` in a `SIZE`x`SIZE` target at 1 px/DBU.
fn camera() -> Camera {
    Camera {
        center: Point::new(48, 48),
        pixels_per_dbu: 1.0,
        viewport: WORLD,
    }
}

/// Records `f` into a fresh single-sample clearing render pass over `target.view()`
/// and reads the frame back.
fn render_into(
    ctx: &WgpuContext,
    target: &OffscreenTarget,
    f: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Vec<u8> {
    let device = ctx.device();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("indirect test encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("indirect test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        f(&mut pass);
    }
    target.copy_to_buffer(&mut encoder);
    ctx.queue().submit(std::iter::once(encoder.finish()));
    target.read_pixels(ctx)
}

/// Counts pixels whose any channel differs by more than `max_delta`.
fn differing(a: &[u8], b: &[u8], max_delta: u8) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .any(|(&x, &y)| x.abs_diff(y) > max_delta)
        })
        .count()
}

#[test]
fn indirect_draw_matches_direct() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    if !IndirectRects::supported(&ctx) {
        eprintln!("adapter cannot execute indirect draws; skipping");
        return;
    }
    let device = ctx.device();
    let queue = ctx.queue();

    let instances = scene();
    let cam = camera();
    let view = ViewUniform::from_camera(&cam, SIZE, SIZE);

    // ---- Direct reference: RetainedRenderer draws every instance (single-sample). ----
    let mut direct = RetainedRenderer::new(device, TARGET_FORMAT);
    let expanded = ExpandedScene {
        rects: instances.clone(),
        mesh_vertices: Vec::new(),
        mesh_indices: Vec::new(),
    };
    direct.upload_expanded(device, queue, &expanded);
    direct.set_camera(queue, &view);
    let direct_target = OffscreenTarget::with_sample_count(&ctx, SIZE, SIZE, 1);
    let direct_pixels = render_into(&ctx, &direct_target, |pass| direct.paint(pass));

    // ---- Indirect path: cull (all visible) -> compact -> draw_indexed_indirect. ----
    let boxes: Vec<CullAabb> = instances
        .iter()
        .map(|r| CullAabb {
            min_xy: r.min_xy,
            max_xy: r.max_xy,
        })
        .collect();
    let flags = CellCuller::new(&ctx).cull(&ctx, &boxes, WORLD);
    // Sanity: the world viewport keeps every instance.
    assert_eq!(
        flags.iter().filter(|&&f| f != 0).count(),
        instances.len(),
        "world viewport should keep every instance"
    );

    let compactor = CellCompactor::new(&ctx);
    let compaction = compactor.compact(&ctx, &flags);
    let (_survivors, instance_count) = compactor.read_back(&ctx, &compaction);
    assert_eq!(
        instance_count as usize,
        instances.len(),
        "compaction should keep every instance"
    );

    let indirect = IndirectRects::new(device, TARGET_FORMAT, 1);
    indirect.set_camera(queue, &view);
    let storage = upload_instances(device, &instances);
    let instances_bind = indirect.bind_instances(device, &storage);
    let indirect_target = OffscreenTarget::with_sample_count(&ctx, SIZE, SIZE, 1);
    let indirect_pixels = render_into(&ctx, &indirect_target, |pass| {
        indirect.paint(
            pass,
            &instances_bind,
            compaction.compacted_buffer(),
            compaction.draw_args_buffer(),
        );
    });

    // Both frames must be non-blank (the scene actually drew).
    let lit = indirect_pixels
        .chunks_exact(4)
        .filter(|px| *px != [0, 0, 0, 255])
        .count();
    assert!(lit > 0, "indirect frame must be non-blank");

    // The two images must match within a tight tolerance: same geometry, same math,
    // both single-sampled, differing only in how the draw was issued.
    let total = (SIZE * SIZE) as usize;
    let diff = differing(&direct_pixels, &indirect_pixels, 2);
    let fraction = diff as f64 / total as f64;
    assert!(
        fraction <= 0.01,
        "indirect vs direct differ at {diff}/{total} pixels ({:.2}%), tolerance 1%",
        fraction * 100.0
    );

    // The multi-draw entry point must produce the same image. On an adapter/device
    // without native multi-draw enabled it falls back to one indirect draw per bucket;
    // here a single bucket (draw_count = 1) exercises that path and must still match.
    let multi_target = OffscreenTarget::with_sample_count(&ctx, SIZE, SIZE, 1);
    let multi_pixels = render_into(&ctx, &multi_target, |pass| {
        indirect.paint_multi(
            pass,
            device,
            &MultiDraw {
                instances: &instances_bind,
                compacted: compaction.compacted_buffer(),
                draw_args: compaction.draw_args_buffer(),
                base_offset: 0,
                draw_count: 1,
            },
        );
    });
    let multi_diff = differing(&direct_pixels, &multi_pixels, 2);
    assert!(
        multi_diff as f64 / total as f64 <= 0.01,
        "multi-draw vs direct differ at {multi_diff}/{total} pixels (tolerance 1%); \
         native multi-draw available on this device: {}",
        IndirectRects::supports_multi_draw(device)
    );
}

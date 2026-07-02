//! GPU test for the retained on-surface renderer.
//!
//! Renders a tiny hierarchical scene (a leaf rect placed by an instance and by an
//! array) through [`RetainedRenderer`] into an offscreen target, then checks that
//! placed rectangles light their pixels while the background stays clear. This
//! exercises the shared-device pipeline path, the per-instance transform in the
//! vertex shader, and the paged geometry upload. Skips (and passes) without a GPU.

use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_model::{ArrayInstance, Camera, Cell, Document, DrawShape, Instance, ShapeKind};
use reticle_render::{
    OffscreenTarget, Palette, RetainedRenderer, RetainedScene, Rgba, TARGET_FORMAT, ViewUniform,
    WgpuContext,
};

const SIZE: u32 = 64;

/// Builds a document: a 10x10 leaf rect placed once at (20,20) and as a 1x1 array at
/// (40,40), so two disjoint squares land in a 64x64 world.
fn doc() -> Document {
    let mut leaf = Cell::new("leaf");
    leaf.shapes.push(DrawShape::new(
        LayerId::new(0, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
    ));
    let mut top = Cell::new("top");
    top.instances.push(Instance {
        cell: "leaf".to_owned(),
        transform: Transform::translate(20, 20),
    });
    top.arrays.push(ArrayInstance {
        cell: "leaf".to_owned(),
        transform: Transform::translate(40, 40),
        columns: 1,
        rows: 1,
        column_pitch: 0,
        row_pitch: 0,
    });
    let mut d = Document::new();
    d.insert_cell(leaf);
    d.insert_cell(top);
    d.set_top_cells(vec!["top".to_owned()]);
    d
}

fn pixel_at(pixels: &[u8], px: u32, py: u32) -> [u8; 4] {
    let idx = ((py * SIZE + px) * 4) as usize;
    [
        pixels[idx],
        pixels[idx + 1],
        pixels[idx + 2],
        pixels[idx + 3],
    ]
}

/// World (x, y) to image pixel: world +y up, rows top-down, 1 px/DBU.
fn world_to_pixel(x: i32, y: i32) -> (u32, u32) {
    let px = x.clamp(0, SIZE as i32 - 1) as u32;
    let py = (SIZE as i32 - 1 - y).clamp(0, SIZE as i32 - 1) as u32;
    (px, py)
}

#[test]
fn retained_renderer_draws_placed_geometry() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let device = ctx.device();
    let queue = ctx.queue();

    let document = doc();
    let palette = Palette::from_technology(document.technology());
    let scene = RetainedScene::new(&document, "top", &palette);

    let mut renderer = RetainedRenderer::new(device, TARGET_FORMAT);
    renderer.sync(device, queue, &scene, 1);
    // Two placements of a one-rect cell => two retained rect instances.
    assert_eq!(renderer.rect_count(), 2);

    let target = OffscreenTarget::new(&ctx, SIZE, SIZE);
    let camera = Camera {
        center: Point::new(32, 32),
        pixels_per_dbu: 1.0,
        viewport: Rect::new(Point::new(0, 0), Point::new(64, 64)),
    };
    let view = ViewUniform::from_camera(&camera, SIZE, SIZE);
    renderer.set_camera(queue, &view);

    // Record our own clearing pass and let the retained renderer draw into it.
    let clear = Rgba {
        components: [0.0, 0.0, 0.0, 1.0],
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("retained test encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("retained test pass"),
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
    queue.submit(std::iter::once(encoder.finish()));
    let pixels = target.read_pixels(&ctx);

    // Center of the instance-placed square (20,20)-(30,30): world (25, 25).
    let (ix, iy) = world_to_pixel(25, 25);
    let inst_px = pixel_at(&pixels, ix, iy);
    assert_ne!(
        inst_px,
        [0, 0, 0, 255],
        "instance-placed rect should light pixel ({ix},{iy})"
    );

    // Center of the array-placed square (40,40)-(50,50): world (45, 45).
    let (ax, ay) = world_to_pixel(45, 45);
    let arr_px = pixel_at(&pixels, ax, ay);
    assert_ne!(
        arr_px,
        [0, 0, 0, 255],
        "array-placed rect should light pixel ({ax},{ay})"
    );

    // A gap between the two squares stays background.
    let (gx, gy) = world_to_pixel(5, 5);
    assert_eq!(
        pixel_at(&pixels, gx, gy),
        [0, 0, 0, 255],
        "empty region should stay clear"
    );
}

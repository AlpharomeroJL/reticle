//! Golden-style offscreen tests for the 3D layer-stack view.
//!
//! Builds a tiny document (one cell, one rectangle on a layer with a physical
//! `stack` entry), renders the extruded view offscreen, reads the pixels back,
//! and checks that the top face of the prism carries the layer color at the
//! exact Lambert shade while the background stays the clear color. A second
//! test drives the `StackView` prepare/paint pair (the egui paint-callback
//! shape) headlessly through a blit into a plain color pass.
//!
//! Both tests skip (and pass) on machines without a usable GPU adapter,
//! following the pattern of `tests/golden.rs`.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, LayerInfo, ShapeKind, StackEntry, Technology};
use reticle_render::{
    DEFAULT_CLEAR, LIGHT_DIR, Mesh3d, OrbitCamera, Palette, Rgba, StackView, WgpuContext,
    layer_spans, render_stack_offscreen,
};

const SIZE: u32 = 128;

/// Packed `0xRRGGBBAA` layer color, distinct from the black clear.
const RED: u32 = 0xff00_00ff;
const LAYER: LayerId = LayerId::new(1, 0);

/// One 1000 x 1000 DBU rectangle centered on the origin, extruded 0..100 nm
/// (which is 0..100 world units at 1000 DBU per micron).
fn sample_document() -> Document {
    let mut cell = Cell::new("top");
    cell.shapes.push(DrawShape::new(
        LAYER,
        ShapeKind::Rect(Rect::new(Point::new(-500, -500), Point::new(500, 500))),
    ));

    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_owned()]);
    doc.set_technology(Technology {
        name: "test".to_owned(),
        dbu_per_micron: 1000,
        layers: vec![LayerInfo {
            id: LAYER,
            name: "M1".to_owned(),
            color_rgba: RED,
            visible: true,
        }],
        rules: Vec::new(),
        stack: vec![StackEntry {
            layer: LAYER,
            z_bottom_nm: 0,
            thickness_nm: 100,
        }],
    });
    doc
}

/// The RGBA bytes at pixel `(px, py)` (top-left origin) in a tightly packed image.
fn pixel_at(pixels: &[u8], width: u32, px: u32, py: u32) -> [u8; 4] {
    let idx = ((py * width + px) * 4) as usize;
    [
        pixels[idx],
        pixels[idx + 1],
        pixels[idx + 2],
        pixels[idx + 3],
    ]
}

/// Asserts each channel of `got` is within `tol` of `want`.
fn assert_close(got: [u8; 4], want: [u8; 4], tol: i16, what: &str) {
    for (channel, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
        let delta = (i16::from(g) - i16::from(w)).abs();
        assert!(
            delta <= tol,
            "{what}: channel {channel} = {g}, want {w} (+/- {tol}); full pixel {got:?} vs {want:?}"
        );
    }
}

#[test]
fn renders_one_extruded_rect_offscreen() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let doc = sample_document();
    // Orbit above the slab, looking at its mid-height point: the screen center
    // ray hits the prism's top face.
    let camera = OrbitCamera {
        yaw: -1.2,
        pitch: 1.0,
        distance: 2500.0,
        target: [0.0, 0.0, 50.0],
    };
    let pixels = render_stack_offscreen(&ctx, &doc, "top", &camera, (SIZE, SIZE));
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);

    // The top face normal is +z, so its Lambert term is exactly LIGHT_DIR.z and
    // the shade is 0.4 + 0.6 * 0.8 = 0.88 (LIGHT_DIR is unit length by design).
    let shade = 0.4 + 0.6 * LIGHT_DIR[2];
    let expected_top = [
        (255.0 * shade).round() as u8, // red layer, full red channel
        0,
        0,
        255,
    ];
    let center = pixel_at(&pixels, SIZE, SIZE / 2, SIZE / 2);
    assert_close(
        center,
        expected_top,
        3,
        "top-face pixel at the image center",
    );

    // A corner stays the clear color: the prism does not reach it.
    let clear = DEFAULT_CLEAR.to_packed().to_be_bytes();
    let corner = pixel_at(&pixels, SIZE, 2, 2);
    assert_eq!(corner, clear, "background corner must stay the clear color");
}

#[test]
fn stack_view_prepares_and_blits_into_a_pass() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let doc = sample_document();
    let palette = Palette::from_technology(doc.technology());
    let shapes = doc.flatten("top");
    let spans = layer_spans(doc.technology(), &shapes);
    let mesh = Mesh3d::build(&shapes, &spans, &palette);
    let camera = OrbitCamera::framing(mesh.bounds().expect("non-empty mesh"));

    // A recognizable 3D-frame clear color, distinct from the pass clear (black).
    let frame_clear = Rgba::from_packed(0x1020_30ff);

    let mut view = StackView::new(ctx.device(), reticle_render::TARGET_FORMAT);
    let target = reticle_render::OffscreenTarget::new(&ctx, 64, 64);
    let mut encoder = ctx
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stack view test encoder"),
        });
    view.prepare(
        ctx.device(),
        &mut encoder,
        (64, 64),
        &mesh,
        &camera,
        frame_clear,
    );
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("stack view present pass"),
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
        view.paint(&mut pass);
    }
    target.copy_to_buffer(&mut encoder);
    ctx.queue().submit(std::iter::once(encoder.finish()));
    let pixels = target.read_pixels(&ctx);

    // The blit fills the pass, so a corner shows the 3D frame's clear color
    // (not the pass's black), and the framed prism covers the center with
    // something that is neither.
    let corner = pixel_at(&pixels, 64, 1, 1);
    assert_close(corner, [16, 32, 48, 255], 2, "blitted 3D clear at corner");
    let center = pixel_at(&pixels, 64, 32, 32);
    assert_ne!(center, corner, "prism must cover the view center");
    assert_ne!(center, [0, 0, 0, 255], "center must not be the pass clear");
}

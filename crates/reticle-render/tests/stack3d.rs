//! Golden-style offscreen tests for the 3D layer-stack view.
//!
//! Builds a tiny document (one cell, one rectangle on a layer with a physical
//! `stack` entry), renders the extruded view offscreen, reads the pixels back,
//! and checks that the top face of the prism carries the layer color at the
//! exact Lambert shade while the background stays the clear color. A second
//! test drives the `StackView` prepare/paint pair (the egui paint-callback
//! shape) headlessly through a blit into a plain color pass.
//!
//! The `sky130_*` tests then load the committed SKY130 technology file and
//! prove the real process stack flows through: `layer_spans` and the extruded
//! mesh place every layer at its physical `z_bottom`/`thickness` (nanometers;
//! 1 nm = 1 world unit at SKY130's 1000 DBU per micron), and a side-on
//! offscreen render shows the layers at distinct, correctly ordered image
//! rows with the met2..met5 gap empty.
//!
//! GPU tests skip (and pass) on machines without a usable adapter, following
//! the pattern of `tests/golden.rs`.

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

// ---------------------------------------------------------------------------
// The real SKY130 stack.
// ---------------------------------------------------------------------------

/// The committed SKY130 technology file (1 dbu = 1 nm, so at its 1000 DBU per
/// micron every stack nanometer is exactly one world unit).
const SKY130: &str = include_str!("../../../tech/sky130.tech");

/// The SKY130 layers exercised here: `(GDS id, half-width DBU, z_bottom nm,
/// z_top nm)`, with the z band transcribed from the official process stack
/// diagram (see `tech/sky130.tech`). Half-widths shrink with height, so the
/// origin-centered squares form a ziggurat whose every layer stays visible
/// from the side.
const SKY130_LAYERS: [(LayerId, i32, f32, f32); 6] = [
    (LayerId::new(64, 20), 2000, -1000.0, 0.0), // nwell (approximate substrate)
    (LayerId::new(66, 20), 1700, 326.0, 506.0), // poly
    (LayerId::new(67, 20), 1400, 936.0, 1036.0), // li1
    (LayerId::new(68, 20), 1100, 1376.0, 1736.0), // met1
    (LayerId::new(69, 20), 800, 2006.0, 2366.0), // met2
    (LayerId::new(72, 20), 500, 5371.0, 6631.0), // met5
];

/// Parses the committed SKY130 technology and builds a document with one
/// origin-centered square per entry of [`SKY130_LAYERS`].
fn sky130_document() -> Document {
    let tech = reticle_io::parse_technology(SKY130).expect("committed sky130.tech parses");
    let mut cell = Cell::new("top");
    for (layer, half, _, _) in SKY130_LAYERS {
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(-half, -half), Point::new(half, half))),
        ));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_owned()]);
    doc.set_technology(tech);
    doc
}

/// Projects a world point through a column-major `view_proj` into the pixel
/// coordinates of a square image of `size` pixels (top-left origin), clamped
/// to the image.
fn project(view_proj: &[[f32; 4]; 4], p: [f32; 3], size: u32) -> (u32, u32) {
    let v = [p[0], p[1], p[2], 1.0];
    let mut clip = [0.0f32; 4];
    for (column, component) in view_proj.iter().zip(v) {
        for (out, &m) in clip.iter_mut().zip(column.iter()) {
            *out += m * component;
        }
    }
    assert!(clip[3] > 0.0, "sample point must be in front of the camera");
    let half = size as f32 / 2.0;
    let max = (size - 1) as f32;
    let px = ((clip[0] / clip[3] + 1.0) * half).round().clamp(0.0, max);
    let py = ((1.0 - clip[1] / clip[3]) * half).round().clamp(0.0, max);
    (px as u32, py as u32)
}

#[test]
fn sky130_layers_extrude_at_their_real_stack_heights() {
    let doc = sky130_document();
    let shapes = doc.flatten("top");
    let spans = layer_spans(doc.technology(), &shapes);
    assert_eq!(spans.len(), SKY130_LAYERS.len());

    // Every span carries the physical z from the `stack` directives (met5 at
    // 5371 nm and 1260 nm thick, and so on), not a synthetic slab.
    for (layer, _, bottom, top) in SKY130_LAYERS {
        let span = spans
            .iter()
            .find(|s| s.layer == layer)
            .unwrap_or_else(|| panic!("no span for {layer:?}"));
        assert!(
            (span.z_bottom - bottom).abs() < 1e-3 && (span.z_top - top).abs() < 1e-3,
            "layer {layer:?} spans {}..{}, want {bottom}..{top}",
            span.z_bottom,
            span.z_top,
        );
    }

    // The extruded prisms sit exactly inside their slab: per layer, the mesh
    // z extent equals the real band.
    let palette = Palette::from_technology(doc.technology());
    for (layer, _, bottom, top) in SKY130_LAYERS {
        let only: Vec<DrawShape> = shapes
            .iter()
            .filter(|s| s.layer == layer)
            .cloned()
            .collect();
        let mesh = Mesh3d::build(&only, &spans, &palette);
        let (min, max) = mesh.bounds().expect("one prism per layer");
        assert!(
            (min[2] - bottom).abs() < 1e-3 && (max[2] - top).abs() < 1e-3,
            "layer {layer:?} extrudes {}..{}, want {bottom}..{top}",
            min[2],
            max[2],
        );
    }

    // The conductor ladder is strictly ordered bottom to top with real gaps
    // (the connecting contacts and vias are not in this scene).
    let mut ladder: Vec<_> = spans.iter().filter(|s| s.z_bottom >= 0.0).collect();
    ladder.sort_by(|a, b| a.z_bottom.total_cmp(&b.z_bottom));
    for pair in ladder.windows(2) {
        assert!(
            pair[0].z_top < pair[1].z_bottom,
            "conductors must sit at distinct heights: {:?} then {:?}",
            pair[0],
            pair[1],
        );
    }
}

#[test]
fn sky130_stack_renders_layers_at_ordered_heights() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let doc = sky130_document();
    // A side-on orbit (pitch 0) looking along -x at the stack's mid height
    // ((-1000 + 6631) / 2 nm): world z maps monotonically to image rows, so
    // the real stack heights are directly observable in pixel space.
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: 0.0,
        distance: 12_000.0,
        target: [0.0, 0.0, 2_815.5],
    };
    let side: u32 = 256;
    let pixels = render_stack_offscreen(&ctx, &doc, "top", &camera, (side, side));
    assert_eq!(pixels.len(), (side * side * 4) as usize);

    // Sample the camera-facing wall of three layers at mid-band height, plus
    // a point in the empty band between met2 (top 2366 nm) and met5 (bottom
    // 5371 nm); nothing else in the scene occupies that height.
    let view_proj = camera.view_proj(1.0);
    let met5 = project(&view_proj, [500.0, 0.0, 6_001.0], side);
    let gap = project(&view_proj, [0.0, 0.0, 4_000.0], side);
    let met2 = project(&view_proj, [800.0, 0.0, 2_186.0], side);
    let nwell = project(&view_proj, [2_000.0, 0.0, -500.0], side);

    // Higher stack z must land on higher image rows (smaller pixel y).
    assert!(
        met5.1 < gap.1 && gap.1 < met2.1 && met2.1 < nwell.1,
        "projected rows must follow stack order: met5 {met5:?}, gap {gap:?}, met2 {met2:?}, nwell {nwell:?}"
    );

    // The three walls render over the clear color; the gap stays exactly clear.
    let clear = DEFAULT_CLEAR.to_packed().to_be_bytes();
    for (name, (px, py)) in [("met5", met5), ("met2", met2), ("nwell", nwell)] {
        let got = pixel_at(&pixels, side, px, py);
        let covered = got
            .iter()
            .zip(clear.iter())
            .any(|(&g, &c)| (i16::from(g) - i16::from(c)).abs() >= 8);
        assert!(
            covered,
            "{name} wall at ({px}, {py}) must render over the clear color, got {got:?}"
        );
    }
    let gap_pixel = pixel_at(&pixels, side, gap.0, gap.1);
    assert_eq!(
        gap_pixel, clear,
        "the met2..met5 band carries no geometry and must stay clear"
    );
}

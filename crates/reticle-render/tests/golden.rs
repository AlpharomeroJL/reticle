//! Golden-style offscreen render test.
//!
//! Builds a tiny document (one cell, two rectangles on two differently colored
//! layers), renders it offscreen at 64x64, reads the pixels back, and checks that a
//! pixel inside each rectangle carries that layer's color while the background stays
//! the clear color.
//!
//! The test skips (and passes) on machines without a usable GPU adapter, so it is
//! safe in CI; on a real GPU it exercises the full device -> pipeline -> readback
//! path.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Camera, Cell, Document, DrawShape, LayerInfo, ShapeKind, Technology};
use reticle_render::{CellCuller, CullAabb, DEFAULT_CLEAR, Rgba, WgpuContext, WgpuRenderer};

const SIZE: u32 = 64;

/// Packed `0xRRGGBBAA` colors distinct from each other and from the black clear.
const RED: u32 = 0xff00_00ff;
const GREEN: u32 = 0x00ff_00ff;

/// Layer A holds the red rectangle, layer B the green one.
const LAYER_A: LayerId = LayerId::new(1, 0);
const LAYER_B: LayerId = LayerId::new(2, 0);

/// The red rectangle, low-left quadrant of the [0, 64]^2 world.
const RECT_A: Rect = Rect {
    min: Point { x: 8, y: 8 },
    max: Point { x: 28, y: 28 },
};
/// The green rectangle, upper-right quadrant.
const RECT_B: Rect = Rect {
    min: Point { x: 36, y: 36 },
    max: Point { x: 56, y: 56 },
};

fn sample_document() -> Document {
    let mut cell = Cell::new("top");
    cell.shapes
        .push(DrawShape::new(LAYER_A, ShapeKind::Rect(RECT_A)));
    cell.shapes
        .push(DrawShape::new(LAYER_B, ShapeKind::Rect(RECT_B)));

    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_owned()]);

    let tech = Technology {
        name: "test".to_owned(),
        dbu_per_micron: 1000,
        layers: vec![
            LayerInfo {
                id: LAYER_A,
                name: "A".to_owned(),
                color_rgba: RED,
                visible: true,
            },
            LayerInfo {
                id: LAYER_B,
                name: "B".to_owned(),
                color_rgba: GREEN,
                visible: true,
            },
        ],
        rules: Vec::new(),
    };
    doc.set_technology(tech);
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

/// Maps a world DBU point to the pixel it lands on, for this fixed camera.
///
/// The camera centers the [0, 64]^2 world in a 64x64 target at 1 px/DBU, and world
/// `+y` is up while image rows run top-down, so `py = SIZE - 1 - y`.
fn world_to_pixel(x: i32, y: i32) -> (u32, u32) {
    let px = x.clamp(0, SIZE as i32 - 1) as u32;
    let py = (SIZE as i32 - 1 - y).clamp(0, SIZE as i32 - 1) as u32;
    (px, py)
}

#[test]
fn renders_two_layers_offscreen() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let doc = sample_document();
    // Center the [0, 64]^2 world region in the target at 1 pixel per DBU.
    let camera = Camera {
        center: Point::new(32, 32),
        pixels_per_dbu: 1.0,
        viewport: Rect::new(Point::new(0, 0), Point::new(64, 64)),
    };

    let mut renderer = WgpuRenderer::new();
    let pixels = renderer.render_document_offscreen(&ctx, &doc, "top", &camera, (SIZE, SIZE));
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);

    let expected_red = Rgba::from_packed(RED).to_packed().to_be_bytes();
    let expected_green = Rgba::from_packed(GREEN).to_packed().to_be_bytes();
    let expected_clear = clear_bytes();

    // A pixel near the center of each rectangle should carry that layer's color.
    let (ax, ay) = world_to_pixel(18, 18); // inside RECT_A
    let (bx, by) = world_to_pixel(46, 46); // inside RECT_B
    let red = pixel_at(&pixels, SIZE, ax, ay);
    let green = pixel_at(&pixels, SIZE, bx, by);

    assert_eq!(
        red, expected_red,
        "pixel inside red rect at ({ax},{ay}) = {red:?}, want {expected_red:?}"
    );
    assert_eq!(
        green, expected_green,
        "pixel inside green rect at ({bx},{by}) = {green:?}, want {expected_green:?}"
    );

    // A gap between the two rectangles is background (clear color).
    let (gx, gy) = world_to_pixel(32, 32);
    let gap = pixel_at(&pixels, SIZE, gx, gy);
    assert_eq!(
        gap, expected_clear,
        "background pixel at ({gx},{gy}) = {gap:?}, want {expected_clear:?}"
    );
}

/// The default clear color as RGBA bytes.
fn clear_bytes() -> [u8; 4] {
    assert_eq!(DEFAULT_CLEAR, Rgba::from_packed(0x0000_00ff));
    DEFAULT_CLEAR.to_packed().to_be_bytes()
}

#[test]
fn compute_culling_matches_cpu() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Viewport covers the [0, 100]^2 region.
    let viewport = Rect::new(Point::new(0, 0), Point::new(100, 100));
    let rects = [
        Rect::new(Point::new(10, 10), Point::new(20, 20)), // fully inside
        Rect::new(Point::new(90, 90), Point::new(150, 150)), // straddles a corner
        Rect::new(Point::new(200, 200), Point::new(210, 210)), // fully outside
        Rect::new(Point::new(-50, 40), Point::new(5, 60)), // overlaps left edge
        Rect::new(Point::new(100, 0), Point::new(120, 20)), // touches right edge only
    ];
    let boxes: Vec<CullAabb> = rects.iter().copied().map(CullAabb::from_rect).collect();

    let culler = CellCuller::new(&ctx);
    let flags = culler.cull(&ctx, &boxes, viewport);

    // CPU reference using the same half-open overlap test.
    let expected: Vec<u32> = rects
        .iter()
        .map(|r| u32::from(r.intersects(&viewport)))
        .collect();

    assert_eq!(flags, expected, "GPU cull flags must match CPU reference");
    // Spot-check the intended semantics: inside kept, outside and edge-touch culled.
    assert_eq!(flags[0], 1);
    assert_eq!(flags[1], 1);
    assert_eq!(flags[2], 0);
    assert_eq!(flags[3], 1);
    assert_eq!(flags[4], 0);
}

#[test]
fn compute_culling_empty_input() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let culler = CellCuller::new(&ctx);
    let flags = culler.cull(&ctx, &[], Rect::new(Point::new(0, 0), Point::new(10, 10)));
    assert!(flags.is_empty());
}

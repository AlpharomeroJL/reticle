//! Golden-style offscreen render test.
//!
//! Builds a tiny document (one cell, two rectangles on two differently colored
//! layers), renders it offscreen at 64x64, reads the pixels back, and compares the
//! whole frame against a committed golden PNG.
//!
//! The offscreen path is multisampled (4x MSAA where the device supports it), so
//! shape edges are anti-aliased and edge pixels blend toward the background. An exact
//! byte comparison would therefore be brittle, so the comparator is tolerance-based:
//! it allows a small per-channel difference and a small fraction of pixels to differ
//! at all (edge fringes), which is enough to catch a real regression while tolerating
//! benign rasterizer and driver variation.
//!
//! The golden is regenerated when it is missing or when `RETICLE_REGEN_GOLDEN` is set
//! in the environment, so it can be refreshed once on a real GPU and then committed.
//! The test skips (and passes) on machines without a usable GPU adapter, so it is
//! safe in CI; on a real GPU it exercises the full device -> pipeline -> readback
//! path.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Camera, Cell, Document, DrawShape, LayerInfo, ShapeKind, Technology};
use reticle_render::{CellCuller, CullAabb, DEFAULT_CLEAR, Rgba, WgpuContext, WgpuRenderer};
use std::path::PathBuf;

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
        stack: Vec::new(),
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

/// Maximum absolute per-channel difference (0..=255) tolerated at a pixel before it
/// counts as "differing".
///
/// The golden and the frame under test come from the same deterministic pipeline on
/// the same GPU, so matching pixels are normally bit-identical; this small epsilon
/// only absorbs minor rasterizer or driver variation. A real regression (a shape
/// moved, missing, or recolored) swings affected pixels by far more than this, so it
/// still trips the differing-pixel count below.
const MAX_CHANNEL_DELTA: u8 = 8;

/// Maximum fraction of pixels allowed to differ from the golden by more than
/// [`MAX_CHANNEL_DELTA`]. The two 20x20 rectangles expose on the order of 160 edge
/// pixels out of 4096, so a few percent covers anti-aliased edge variation (and a
/// single-sample fallback where MSAA is unavailable) while a shifted or missing shape,
/// which flips hundreds to thousands of pixels, trips the check.
const MAX_DIFFERING_FRACTION: f64 = 0.05;

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

    // Semantic spot checks that survive MSAA: interior and background pixels have full
    // sample coverage, so they resolve to exact colors regardless of anti-aliasing.
    let expected_red = Rgba::from_packed(RED).to_packed().to_be_bytes();
    let expected_green = Rgba::from_packed(GREEN).to_packed().to_be_bytes();
    let expected_clear = clear_bytes();
    let (ax, ay) = world_to_pixel(18, 18); // deep inside RECT_A
    let (bx, by) = world_to_pixel(46, 46); // deep inside RECT_B
    let (gx, gy) = world_to_pixel(32, 32); // gap between the rects
    assert_eq!(
        pixel_at(&pixels, SIZE, ax, ay),
        expected_red,
        "interior of red rect should be exact"
    );
    assert_eq!(
        pixel_at(&pixels, SIZE, bx, by),
        expected_green,
        "interior of green rect should be exact"
    );
    assert_eq!(
        pixel_at(&pixels, SIZE, gx, gy),
        expected_clear,
        "background gap should be exact clear"
    );

    // Whole-frame golden comparison within tolerance.
    let golden_path = golden_path();
    if regen_requested() || !golden_path.exists() {
        write_golden(&golden_path, &pixels);
        eprintln!(
            "regenerated golden at {} ({} bytes); commit it",
            golden_path.display(),
            pixels.len()
        );
        return;
    }

    let golden = read_golden(&golden_path);
    assert_eq!(
        golden.len(),
        pixels.len(),
        "golden size mismatch: golden {} vs rendered {} bytes",
        golden.len(),
        pixels.len()
    );

    let differing = count_differing_pixels(&pixels, &golden, MAX_CHANNEL_DELTA);
    let total = (SIZE * SIZE) as usize;
    let fraction = differing as f64 / total as f64;
    assert!(
        fraction <= MAX_DIFFERING_FRACTION,
        "{differing} of {total} pixels ({:.2}%) differ from the golden beyond tolerance \
         (allowed {:.2}%); set RETICLE_REGEN_GOLDEN=1 to refresh {}",
        fraction * 100.0,
        MAX_DIFFERING_FRACTION * 100.0,
        golden_path.display()
    );
}

/// The default clear color as RGBA bytes.
fn clear_bytes() -> [u8; 4] {
    assert_eq!(DEFAULT_CLEAR, Rgba::from_packed(0x0000_00ff));
    DEFAULT_CLEAR.to_packed().to_be_bytes()
}

/// Path to the committed golden PNG for [`renders_two_layers_offscreen`].
fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("two_layers.png")
}

/// Whether the environment requests a golden regeneration (`RETICLE_REGEN_GOLDEN`
/// set to any non-empty value).
fn regen_requested() -> bool {
    std::env::var_os("RETICLE_REGEN_GOLDEN").is_some_and(|v| !v.is_empty())
}

/// Writes `pixels` (tightly packed RGBA, row 0 at the top) as a PNG golden, creating
/// the parent directory if needed.
fn write_golden(path: &std::path::Path, pixels: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create golden dir");
    }
    image::save_buffer(path, pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
        .expect("write golden png");
}

/// Reads the golden PNG back into tightly packed RGBA bytes (row 0 at the top).
fn read_golden(path: &std::path::Path) -> Vec<u8> {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("open golden {}: {e}", path.display()))
        .to_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (SIZE, SIZE),
        "golden dimensions must be {SIZE}x{SIZE}"
    );
    img.into_raw()
}

/// Counts pixels whose any channel differs from `golden` by more than `max_delta`.
/// Both slices are tightly packed RGBA of the same length.
fn count_differing_pixels(rendered: &[u8], golden: &[u8], max_delta: u8) -> usize {
    rendered
        .chunks_exact(4)
        .zip(golden.chunks_exact(4))
        .filter(|(a, b)| {
            a.iter()
                .zip(b.iter())
                .any(|(&x, &y)| x.abs_diff(y) > max_delta)
        })
        .count()
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

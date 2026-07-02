//! Offscreen media capture: the hero image, demo GIFs, and feature stills.
//!
//! Renders generated and demo layouts through the offscreen `reticle-render`
//! paths, writes PNG frames with the `image` crate, and assembles GIFs with the
//! installed `gifski` CLI. Skips gracefully when no GPU adapter is available.

use crate::overlay::{Canvas, WorldMap};
use image::{ImageBuffer, Rgba};
use reticle_app::camera::ScreenRect;
use reticle_app::minimap::MinimapLayout;
use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{
    Camera, Cell, Document, DrawShape, LayerInfo, NetSpec, RouteRequest, Router, Rule, RuleKind,
    RuleSet, ShapeKind, StackEntry,
};
use reticle_render::{OrbitCamera, WgpuContext, WgpuRenderer, render_stack_offscreen};
use reticle_route::{MazeRouter, RouteConfig};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The demo technology's diffusion layer.
const ACTIVE: LayerId = LayerId::new(2, 0);
/// The demo technology's polysilicon layer.
const POLY: LayerId = LayerId::new(3, 0);
/// The demo technology's first metal layer.
const METAL1: LayerId = LayerId::new(4, 0);
/// The demo technology's second metal layer.
const METAL2: LayerId = LayerId::new(5, 0);

const HERO: (u32, u32) = (2560, 1440);
const GIF: (u32, u32) = (960, 540);
const GIF_FRAMES: u32 = 48;
/// Render size for the single-frame feature stills.
const STILL: (u32, u32) = (1600, 1000);

/// Renders the media set into `out_dir`: the hero image, the browse GIF, and the
/// feature stills. `only` restricts the run to one named asset (`hero`, `browse`,
/// `stack3d`, ...). Returns `Ok(false)` (skipped) if no GPU adapter is available.
///
/// # Errors
///
/// Propagates filesystem errors from creating the output directory or writing files.
pub fn capture(out_dir: &Path, only: Option<&str>) -> std::io::Result<bool> {
    let wants = |name: &str| only.is_none_or(|o| o == name);
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping media capture");
        return Ok(false);
    };
    let mut renderer = WgpuRenderer::new();
    std::fs::create_dir_all(out_dir)?;

    if wants("hero") || wants("browse") {
        capture_hero_and_browse(&ctx, &mut renderer, out_dir, &wants)?;
    }
    if wants("stack3d") {
        capture_stack3d(&ctx, out_dir)?;
    }
    if wants("drc") {
        capture_drc(&ctx, &mut renderer, out_dir)?;
    }
    if wants("minimap") {
        capture_minimap(&ctx, &mut renderer, out_dir)?;
    }
    if wants("route") {
        capture_route(&ctx, &mut renderer, out_dir)?;
    }
    if wants("collab") {
        capture_collab(&ctx, &mut renderer, out_dir)?;
    }
    Ok(true)
}

/// Renders the demo document with two collaborators' live presence (cursor,
/// initial chip, and viewport rectangle from `reticle-sync`) to `collab.png`.
fn capture_collab(
    ctx: &WgpuContext,
    renderer: &mut WgpuRenderer,
    out_dir: &Path,
) -> std::io::Result<()> {
    use reticle_sync::{Awareness, Presence};

    let doc = reticle_app::demo::demo_document();
    let top = reticle_app::demo::TOP_CELL;
    let bbox = document_bounds(&doc, top);
    let camera = frame_camera(bbox, STILL, 0.94);
    let mut rgba = renderer.render_document_offscreen(ctx, &doc, top, &camera, STILL);

    // Two collaborators, exchanged through the real awareness map.
    let mut awareness = Awareness::new();
    awareness.set(Presence {
        actor: "ada".to_owned(),
        display_name: "Ada".to_owned(),
        color_rgba: 0x5A_C8_FA_FF,
        cursor: frac_point(bbox, 0.42, 0.46),
        selection: Vec::new(),
        viewport: frac_rect(bbox, 0.16, 0.20, 0.58, 0.72),
    });
    awareness.set(Presence {
        actor: "grace".to_owned(),
        display_name: "Grace".to_owned(),
        color_rgba: 0xFF_9F_0A_FF,
        cursor: frac_point(bbox, 0.68, 0.64),
        selection: Vec::new(),
        viewport: frac_rect(bbox, 0.46, 0.34, 0.94, 0.90),
    });
    let mut actors: Vec<&Presence> = awareness.iter().map(|(_, presence)| presence).collect();
    actors.sort_by(|a, b| a.actor.cmp(&b.actor));

    let map = WorldMap::new(&camera, STILL);
    let mut canvas = Canvas::new(&mut rgba, STILL);
    for presence in actors {
        let color = rgba_bytes(presence.color_rgba);
        // Their visible viewport.
        let (x0, y0, x1, y1) = map.rect_to_px(presence.viewport);
        canvas.stroke_rect(x0, y0, x1, y1, 2.0, color);
        // Cursor arrow with the display-name initial in a chip beside it.
        let (cx, cy) = map.to_px(presence.cursor);
        canvas.fill_tri(
            (cx, cy),
            (cx + 5.0, cy + 17.0),
            (cx + 12.0, cy + 11.0),
            [255, 255, 255, 255],
        );
        canvas.fill_tri(
            (cx + 1.5, cy + 2.5),
            (cx + 5.0, cy + 14.0),
            (cx + 9.5, cy + 10.0),
            color,
        );
        let (chip_x, chip_y) = (cx + 14.0, cy + 16.0);
        canvas.fill_rect(chip_x, chip_y, chip_x + 24.0, chip_y + 24.0, color);
        canvas.stroke_rect(
            chip_x,
            chip_y,
            chip_x + 24.0,
            chip_y + 24.0,
            1.0,
            [20, 22, 28, 200],
        );
        let initial = presence
            .display_name
            .chars()
            .next()
            .unwrap_or('A')
            .to_ascii_uppercase();
        canvas.draw_glyph(initial, chip_x + 7.0, chip_y + 5.0, 2, [16, 18, 24, 255]);
    }
    save_png(&out_dir.join("collab.png"), &rgba, STILL)?;
    eprintln!("wrote {}", out_dir.join("collab.png").display());
    Ok(())
}

/// The point at the fractional position `(fx, fy)` of `bbox`.
fn frac_point(bbox: Rect, fx: f32, fy: f32) -> Point {
    Point::new(
        bbox.min.x + (bbox.width() as f32 * fx) as i32,
        bbox.min.y + (bbox.height() as f32 * fy) as i32,
    )
}

/// The sub-rectangle of `bbox` between fractional corners.
fn frac_rect(bbox: Rect, fx0: f32, fy0: f32, fx1: f32, fy1: f32) -> Rect {
    Rect::new(frac_point(bbox, fx0, fy0), frac_point(bbox, fx1, fy1))
}

/// Unpacks a `0xRRGGBBAA` color into RGBA bytes.
fn rgba_bytes(color: u32) -> [u8; 4] {
    [
        (color >> 24) as u8,
        (color >> 16) as u8,
        (color >> 8) as u8,
        color as u8,
    ]
}

/// Runs the real maze router over a small obstacle course and renders the routed
/// paths with terminal markers to `route.png`.
fn capture_route(
    ctx: &WgpuContext,
    renderer: &mut WgpuRenderer,
    out_dir: &Path,
) -> std::io::Result<()> {
    const CELL: &str = "ROUTE_DEMO";
    /// Per-net terminal marker colors.
    const TERMINAL_COLORS: [[u8; 4]; 5] = [
        [90, 200, 250, 255],
        [255, 159, 10, 255],
        [255, 45, 133, 255],
        [88, 214, 141, 255],
        [191, 90, 242, 255],
    ];
    let mut doc = route_demo_doc(CELL);
    let request = route_demo_request(CELL);
    let mut router = MazeRouter::with_config(
        RouteConfig::new()
            .with_pitch(200)
            .with_spacing(100)
            .with_wire_width(120)
            .with_layers(2)
            .with_via_cost(60),
    );
    let report = router.route(&mut doc, &request);
    eprintln!(
        "route demo: {} routed, {} failed, {} DBU of wire",
        report.routed, report.failed, report.total_length_dbu
    );

    let bbox = document_bounds(&doc, CELL);
    let camera = frame_camera(bbox, STILL, 0.94);
    let mut rgba = renderer.render_document_offscreen(ctx, &doc, CELL, &camera, STILL);
    let map = WorldMap::new(&camera, STILL);
    let mut canvas = Canvas::new(&mut rgba, STILL);
    for (index, net) in request.nets.iter().enumerate() {
        let color = TERMINAL_COLORS[index % TERMINAL_COLORS.len()];
        for terminal in &net.terminals {
            let (x, y) = map.to_px(*terminal);
            canvas.fill_rect(x - 6.0, y - 6.0, x + 6.0, y + 6.0, color);
            canvas.stroke_rect(
                x - 6.0,
                y - 6.0,
                x + 6.0,
                y + 6.0,
                1.0,
                [255, 255, 255, 230],
            );
        }
    }
    save_png(&out_dir.join("route.png"), &rgba, STILL)?;
    eprintln!("wrote {}", out_dir.join("route.png").display());
    Ok(())
}

/// A routing obstacle course on the demo technology: metal-2 blocks in the middle
/// of an otherwise empty cell. The technology gains a display entry for
/// `(METAL1, 1)`, the upper routing plane the router addresses by datatype.
fn route_demo_doc(cell_name: &str) -> Document {
    let mut cell = Cell::new(cell_name);
    cell.shapes.push(rect_shape(METAL2, 4000, 2000, 5200, 6000));
    cell.shapes.push(rect_shape(METAL2, 6000, 1000, 7200, 4400));
    cell.shapes.push(rect_shape(METAL2, 8000, 3600, 9200, 7000));

    let mut tech = reticle_app::demo::demo_technology();
    tech.layers.push(LayerInfo {
        id: LayerId::new(4, 1),
        name: "METAL1.UP".to_owned(),
        color_rgba: 0x7F_B3_E8_FF,
        visible: true,
    });

    let mut doc = Document::new();
    doc.set_technology(tech);
    doc.insert_cell(cell);
    doc.set_top_cells(vec![cell_name.to_owned()]);
    doc
}

/// The nets for the routing still: four left-to-right nets that must clear the
/// obstacle field, plus one three-terminal vertical net that crosses them all.
fn route_demo_request(cell_name: &str) -> RouteRequest {
    let net = |name: &str, terminals: Vec<Point>| NetSpec {
        name: name.to_owned(),
        terminals,
        layer: METAL1,
    };
    RouteRequest {
        cell: cell_name.to_owned(),
        nets: vec![
            net("n1", vec![Point::new(1000, 1600), Point::new(11000, 2000)]),
            net("n2", vec![Point::new(1000, 3200), Point::new(11000, 3600)]),
            net("n3", vec![Point::new(1000, 4800), Point::new(11000, 5200)]),
            net("n4", vec![Point::new(1000, 6400), Point::new(11000, 6800)]),
            net(
                "n5",
                vec![
                    Point::new(2600, 800),
                    Point::new(2600, 7200),
                    Point::new(1600, 7200),
                ],
            ),
        ],
    }
}

/// Renders a zoomed-in view of a generated layout with the app's minimap in the
/// top-right corner (overview box plus the current viewport rectangle) to
/// `minimap.png`. The panel placement and both rectangles come from the app's
/// own [`MinimapLayout`] math.
fn capture_minimap(
    ctx: &WgpuContext,
    renderer: &mut WgpuRenderer,
    out_dir: &Path,
) -> std::io::Result<()> {
    /// The minimap accent color for the viewport rectangle.
    const ACCENT: [u8; 4] = [90, 200, 250, 255];
    let doc = crate::generator::generate_layout(60_000, 6, 3);
    let Some(top) = doc.top_cells().first().cloned() else {
        eprintln!("generated document has no top cell");
        return Ok(());
    };
    let bbox = document_bounds(&doc, &top);
    let camera = offset_camera(bbox, STILL, 5.0, 0.38, 0.60);
    let mut rgba = renderer.render_document_offscreen(ctx, &doc, &top, &camera, STILL);

    let canvas_rect = ScreenRect {
        left: 0.0,
        top: 0.0,
        width: STILL.0 as f32,
        height: STILL.1 as f32,
    };
    let Some(layout) = MinimapLayout::compute(&canvas_rect, bbox) else {
        eprintln!("minimap layout is degenerate; skipping minimap.png");
        return Ok(());
    };

    // Overview content: the whole document fit to the panel's content rectangle.
    let (cx, cy, cw, ch) = layout.world_rect_to_panel(bbox);
    let mini_size = ((cw.round() as u32).max(1), (ch.round() as u32).max(1));
    let mini_cam = frame_camera(bbox, mini_size, 1.0);
    let mini = renderer.render_document_offscreen(ctx, &doc, &top, &mini_cam, mini_size);

    let mut canvas = Canvas::new(&mut rgba, STILL);
    let panel = layout.panel;
    let (px1, py1) = (panel.left + panel.width, panel.top + panel.height);
    canvas.fill_rect(panel.left, panel.top, px1, py1, [14, 16, 22, 240]);
    canvas.blit(&mini, mini_size, cx.round() as i32, cy.round() as i32);
    canvas.stroke_rect(panel.left, panel.top, px1, py1, 2.0, [96, 104, 120, 255]);

    // The camera's visible world rectangle, clamped into the panel.
    let (vx, vy, vw, vh) = layout.world_rect_to_panel(camera.viewport);
    canvas.fill_rect(vx, vy, vx + vw, vy + vh, [90, 200, 250, 36]);
    canvas.stroke_rect(vx, vy, vx + vw, vy + vh, 2.0, ACCENT);

    save_png(&out_dir.join("minimap.png"), &rgba, STILL)?;
    eprintln!("wrote {}", out_dir.join("minimap.png").display());
    Ok(())
}

/// Runs real DRC on a small deliberately broken layout and renders the geometry
/// with a marker over every violation to `drc.png`.
fn capture_drc(
    ctx: &WgpuContext,
    renderer: &mut WgpuRenderer,
    out_dir: &Path,
) -> std::io::Result<()> {
    const CELL: &str = "DRC_DEMO";
    /// The marker stroke color.
    const MARKER: [u8; 4] = [255, 45, 85, 255];
    let (doc, rules) = drc_demo_doc(CELL);
    let engine = DrcEngine::new(rules);
    let violations = engine.check_cell(&doc, CELL);
    eprintln!("drc demo: {} violations", violations.len());

    let bbox = document_bounds(&doc, CELL);
    let camera = frame_camera(bbox, STILL, 0.94);
    let mut rgba = renderer.render_document_offscreen(ctx, &doc, CELL, &camera, STILL);

    let map = WorldMap::new(&camera, STILL);
    let mut canvas = Canvas::new(&mut rgba, STILL);
    for violation in &violations {
        let (x0, y0, x1, y1) = map.rect_to_px(violation.location);
        canvas.fill_rect(x0, y0, x1, y1, [255, 45, 85, 56]);
        canvas.stroke_rect(x0 - 3.0, y0 - 3.0, x1 + 3.0, y1 + 3.0, 3.0, MARKER);
        canvas.stroke_rect(x0, y0, x1, y1, 1.0, [255, 255, 255, 220]);
    }
    save_png(&out_dir.join("drc.png"), &rgba, STILL)?;
    eprintln!("wrote {}", out_dir.join("drc.png").display());
    Ok(())
}

/// A small flat layout on the demo technology that deliberately breaks three
/// rules: two metal-1 pairs sit 160 DBU apart (spacing >= 300), one poly line is
/// 140 DBU wide (width >= 200), and one diffusion pad is under the minimum area.
fn drc_demo_doc(cell_name: &str) -> (Document, Vec<Rule>) {
    let mut cell = Cell::new(cell_name);
    let shapes = &mut cell.shapes;
    // Metal-1 buses and stubs at legal spacing.
    shapes.push(rect_shape(METAL1, 800, 1000, 11200, 1400));
    shapes.push(rect_shape(METAL1, 800, 2000, 11200, 2400));
    shapes.push(rect_shape(METAL1, 800, 3000, 11200, 3400));
    shapes.push(rect_shape(METAL1, 1000, 4200, 1400, 6800));
    shapes.push(rect_shape(METAL1, 1800, 4200, 2200, 6800));
    // Two metal-1 pairs with a 160 DBU gap: spacing violations.
    shapes.push(rect_shape(METAL1, 2600, 4200, 4200, 4600));
    shapes.push(rect_shape(METAL1, 4360, 4200, 5800, 4600));
    shapes.push(rect_shape(METAL1, 7000, 4600, 7400, 6800));
    shapes.push(rect_shape(METAL1, 7560, 4600, 7960, 6800));
    // Poly lines at legal width, plus one 140 DBU sliver: width violation.
    shapes.push(rect_shape(POLY, 8600, 4600, 8900, 7200));
    shapes.push(rect_shape(POLY, 9300, 4600, 9600, 7200));
    shapes.push(rect_shape(POLY, 10000, 4600, 10300, 7200));
    shapes.push(rect_shape(POLY, 10700, 4600, 10840, 7200));
    // Diffusion pads, plus one tiny pad under the minimum area.
    shapes.push(rect_shape(ACTIVE, 2800, 5000, 4800, 6400));
    shapes.push(rect_shape(ACTIVE, 6800, 1200, 8300, 2300));
    shapes.push(rect_shape(ACTIVE, 3400, 6800, 3900, 7200));

    let mut doc = Document::new();
    doc.set_technology(reticle_app::demo::demo_technology());
    doc.insert_cell(cell);
    doc.set_top_cells(vec![cell_name.to_owned()]);

    let rules = vec![
        rule("M1.S1 spacing >= 0.30um", RuleKind::Spacing, METAL1, 300),
        rule("PO.W1 width >= 0.20um", RuleKind::Width, POLY, 200),
        rule("AA.A1 area >= 1.0um^2", RuleKind::Area, ACTIVE, 1_000_000),
    ];
    (doc, rules)
}

/// A rectangle shape on `layer` from corner coordinates.
fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// A single-layer rule with the given threshold.
fn rule(name: &str, kind: RuleKind, layer: LayerId, value: i64) -> Rule {
    Rule {
        name: name.to_owned(),
        kind,
        layer,
        other_layer: None,
        value,
    }
}

/// Renders the dense generated layout as the hero image and the browse zoom GIF.
fn capture_hero_and_browse(
    ctx: &WgpuContext,
    renderer: &mut WgpuRenderer,
    out_dir: &Path,
    wants: &dyn Fn(&str) -> bool,
) -> std::io::Result<()> {
    let doc = crate::generator::generate_layout(200_000, 8, 3);
    let Some(top) = doc.top_cells().first().cloned() else {
        eprintln!("generated document has no top cell");
        return Ok(());
    };
    let bbox = document_bounds(&doc, &top);

    if wants("hero") {
        // Hero: the whole design at high resolution.
        let hero_cam = frame_camera(bbox, HERO, 0.92);
        let rgba = renderer.render_document_offscreen(ctx, &doc, &top, &hero_cam, HERO);
        save_png(&out_dir.join("hero.png"), &rgba, HERO)?;
        eprintln!("wrote {}", out_dir.join("hero.png").display());
    }

    if wants("browse") {
        // Browse GIF: ease-in zoom from the full view toward the center.
        let frames_dir = out_dir.join("frames");
        std::fs::create_dir_all(&frames_dir)?;
        let mut frames = Vec::with_capacity(GIF_FRAMES as usize);
        for index in 0..GIF_FRAMES {
            let t = index as f32 / GIF_FRAMES as f32;
            let zoom = 0.92 * (1.0 + 3.0 * smoothstep(t));
            let cam = frame_camera(bbox, GIF, zoom);
            let rgba = renderer.render_document_offscreen(ctx, &doc, &top, &cam, GIF);
            let path = frames_dir.join(format!("frame_{index:04}.png"));
            save_png(&path, &rgba, GIF)?;
            frames.push(path);
        }
        assemble_gif(&frames, &out_dir.join("browse.gif"));
        eprintln!("wrote {}", out_dir.join("browse.gif").display());
    }
    Ok(())
}

/// Renders the extruded 3D layer stack of the demo document to `stack3d.png`.
fn capture_stack3d(ctx: &WgpuContext, out_dir: &Path) -> std::io::Result<()> {
    let doc = demo_doc_with_stack();
    let top = reticle_app::demo::TOP_CELL;
    let bbox = document_bounds(&doc, top);
    let stack = &doc.technology().stack;
    let z_min = stack.iter().map(|e| e.z_bottom_nm).min().unwrap_or(0);
    let z_max = stack.iter().map(StackEntry::z_top_nm).max().unwrap_or(1);
    // The demo technology has 1000 DBU per micron, so 1 nm of stack height is
    // exactly 1 world unit and xy DBU need no conversion.
    let bounds = (
        [bbox.min.x as f32, bbox.min.y as f32, z_min as f32],
        [bbox.max.x as f32, bbox.max.y as f32, z_max as f32],
    );
    let mut camera = OrbitCamera::framing(bounds);
    camera.orbit(0.25, 0.05);
    camera.zoom(0.62);
    let rgba = render_stack_offscreen(ctx, &doc, top, &camera, STILL);
    save_png(&out_dir.join("stack3d.png"), &rgba, STILL)?;
    eprintln!("wrote {}", out_dir.join("stack3d.png").display());
    Ok(())
}

/// The app's demo document with physical `stack` directives added, so the 3D view
/// extrudes real slabs (well below the surface, metals above) instead of the
/// synthetic uniform fallback.
fn demo_doc_with_stack() -> Document {
    let mut doc = reticle_app::demo::demo_document();
    let mut tech = doc.technology().clone();
    tech.stack = vec![
        stack_entry(1, -400, 400), // NWELL: buried, its top at the substrate surface.
        stack_entry(2, 0, 450),    // ACTIVE
        stack_entry(3, 550, 500),  // POLY
        stack_entry(4, 1350, 600), // METAL1
        stack_entry(5, 2350, 700), // METAL2
    ];
    doc.set_technology(tech);
    // Drop the TEXT label: a flat label extruded into a slab reads as noise in 3D.
    if let Some(cell) = doc.cell_mut(reticle_app::demo::TOP_CELL) {
        cell.shapes.retain(|s| s.layer != LayerId::new(6, 0));
    }
    doc
}

/// A stack directive for demo layer `(layer, 0)`, in nanometers.
fn stack_entry(layer: u16, z_bottom_nm: i64, thickness_nm: i64) -> StackEntry {
    StackEntry {
        layer: LayerId::new(layer, 0),
        z_bottom_nm,
        thickness_nm,
    }
}

/// The bounding box of the top cell, with a sane fallback for an empty design.
fn document_bounds(doc: &Document, top: &str) -> Rect {
    doc.cell_bbox(top)
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1000, 1000)))
}

/// A camera that fits `bbox` into `size` pixels, scaled by `zoom` (`1.0` = fit).
fn frame_camera(bbox: Rect, size: (u32, u32), zoom: f32) -> Camera {
    let (width, height) = (size.0 as f32, size.1 as f32);
    let world_w = bbox.width().max(1) as f32;
    let world_h = bbox.height().max(1) as f32;
    let fit = (width / world_w).min(height / world_h);
    let ppd = fit * zoom;
    let cx = i64::midpoint(i64::from(bbox.min.x), i64::from(bbox.max.x));
    let cy = i64::midpoint(i64::from(bbox.min.y), i64::from(bbox.max.y));
    let center = Point::new(cx as i32, cy as i32);
    let half_w = (width / ppd / 2.0) as i32;
    let half_h = (height / ppd / 2.0) as i32;
    Camera {
        center,
        pixels_per_dbu: ppd,
        viewport: Rect::new(
            center.translate(-half_w, -half_h),
            center.translate(half_w, half_h),
        ),
    }
}

/// A camera like [`frame_camera`] but centered at the fractional position
/// `(fx, fy)` of `bbox` instead of its middle, for looking at an off-center
/// region.
fn offset_camera(bbox: Rect, size: (u32, u32), zoom: f32, fx: f32, fy: f32) -> Camera {
    let (width, height) = (size.0 as f32, size.1 as f32);
    let world_w = bbox.width().max(1) as f32;
    let world_h = bbox.height().max(1) as f32;
    let fit = (width / world_w).min(height / world_h);
    let ppd = fit * zoom;
    let center = Point::new(
        bbox.min.x + (world_w * fx) as i32,
        bbox.min.y + (world_h * fy) as i32,
    );
    let half_w = (width / ppd / 2.0) as i32;
    let half_h = (height / ppd / 2.0) as i32;
    Camera {
        center,
        pixels_per_dbu: ppd,
        viewport: Rect::new(
            center.translate(-half_w, -half_h),
            center.translate(half_w, half_h),
        ),
    }
}

/// Smooth ease-in/ease-out on `[0, 1]`.
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Saves tightly packed RGBA bytes as a PNG.
fn save_png(path: &Path, rgba: &[u8], size: (u32, u32)) -> std::io::Result<()> {
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(size.0, size.1, rgba.to_vec())
            .ok_or_else(|| std::io::Error::other("rgba buffer size does not match dimensions"))?;
    buffer
        .save(path)
        .map_err(|err| std::io::Error::other(err.to_string()))
}

/// Assembles PNG frames into a GIF with the installed `gifski` CLI.
fn assemble_gif(frames: &[PathBuf], out: &Path) {
    let mut cmd = Command::new("gifski");
    cmd.arg("--fps")
        .arg("20")
        .arg("--quality")
        .arg("90")
        .arg("-o")
        .arg(out);
    for frame in frames {
        cmd.arg(frame);
    }
    match cmd.status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("gifski exited with {status}"),
        Err(err) => eprintln!("could not run gifski (is it installed and on PATH?): {err}"),
    }
}

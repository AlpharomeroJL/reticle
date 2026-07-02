//! Offscreen media capture: the hero image, demo GIFs, and feature stills.
//!
//! Renders generated and demo layouts through the offscreen `reticle-render`
//! paths, writes PNG frames with the `image` crate, and assembles GIFs with the
//! installed `gifski` CLI. Skips gracefully when no GPU adapter is available.

use image::{ImageBuffer, Rgba};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Camera, Document, StackEntry};
use reticle_render::{OrbitCamera, WgpuContext, WgpuRenderer, render_stack_offscreen};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    Ok(true)
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

//! Offscreen media capture: the hero image and a browse GIF.
//!
//! Renders a generated layout through the offscreen `reticle-render` path, writes
//! PNG frames with the `image` crate, and assembles GIFs with the installed
//! `gifski` CLI. Skips gracefully when no GPU adapter is available.

use image::{ImageBuffer, Rgba};
use reticle_geometry::{Point, Rect};
use reticle_model::{Camera, Document};
use reticle_render::{WgpuContext, WgpuRenderer};
use std::path::{Path, PathBuf};
use std::process::Command;

const HERO: (u32, u32) = (2560, 1440);
const GIF: (u32, u32) = (960, 540);
const GIF_FRAMES: u32 = 48;

/// Renders the hero image and a browse GIF into `out_dir`. Returns `Ok(false)`
/// (skipped) if no GPU adapter is available.
///
/// # Errors
///
/// Propagates filesystem errors from creating the output directory or writing files.
pub fn capture(out_dir: &Path) -> std::io::Result<bool> {
    let doc = crate::generator::generate_layout(200_000, 8, 3);
    let Some(top) = doc.top_cells().first().cloned() else {
        eprintln!("generated document has no top cell");
        return Ok(false);
    };
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping media capture");
        return Ok(false);
    };
    let mut renderer = WgpuRenderer::new();
    let bbox = document_bounds(&doc, &top);

    std::fs::create_dir_all(out_dir)?;

    // Hero: the whole design at high resolution.
    let hero_cam = frame_camera(bbox, HERO, 0.92);
    let rgba = renderer.render_document_offscreen(&ctx, &doc, &top, &hero_cam, HERO);
    save_png(&out_dir.join("hero.png"), &rgba, HERO)?;
    eprintln!("wrote {}", out_dir.join("hero.png").display());

    // Browse GIF: ease-in zoom from the full view toward the center.
    let frames_dir = out_dir.join("frames");
    std::fs::create_dir_all(&frames_dir)?;
    let mut frames = Vec::with_capacity(GIF_FRAMES as usize);
    for index in 0..GIF_FRAMES {
        let t = index as f32 / GIF_FRAMES as f32;
        let zoom = 0.92 * (1.0 + 3.0 * smoothstep(t));
        let cam = frame_camera(bbox, GIF, zoom);
        let rgba = renderer.render_document_offscreen(&ctx, &doc, &top, &cam, GIF);
        let path = frames_dir.join(format!("frame_{index:04}.png"));
        save_png(&path, &rgba, GIF)?;
        frames.push(path);
    }
    assemble_gif(&frames, &out_dir.join("browse.gif"));
    eprintln!("wrote {}", out_dir.join("browse.gif").display());
    Ok(true)
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

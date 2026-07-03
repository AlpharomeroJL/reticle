//! `capture-ui`: drive the real editor through committed demo scripts and assemble
//! the README media (a hero still plus the tour GIFs) from full-window screenshots.
//!
//! Unlike [`media`](crate::media), which renders the canvas offscreen, this path runs
//! the actual `reticle-app` window under a scripted demo mode (`--demo-script`), so
//! the media shows the whole application: panels, canvas, and chrome. For each named
//! capture it spawns the app on that capture's committed `.demo` script, reads the
//! `manifest.json` the run wrote, and either copies the single still to
//! `assets/<name>.png` or assembles the frames into `assets/<name>.gif` under a 6 MB
//! budget (downscaling with gifski `--width` and lowering quality as needed). Every
//! capture is reproducible: `just capture-ui` replays the same scripts.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::media::assemble_gif_scaled;

/// The GIF size budget: each tour GIF must stay under this.
const MAX_GIF_BYTES: u64 = 6 * 1024 * 1024;

/// The committed captures, by base name. Each has a `<name>.demo` script and produces
/// `assets/<name>.png` (a snap) or `assets/<name>.gif` (a tour), per its manifest.
const CAPTURES: [&str; 6] = [
    "hero",
    "tour-drc",
    "tour-edit",
    "tour-agent",
    "tour-query",
    "tour-3d",
];

/// GIF output widths to try, largest first, when fitting under the size budget.
const WIDTH_LADDER: [u32; 6] = [1600, 1360, 1200, 1040, 900, 800];

/// A chosen GIF encoding: output width and gifski quality, plus the size estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodePlan {
    /// Output GIF width in pixels (frames are downscaled to this).
    pub width: u32,
    /// gifski quality (1..=100).
    pub quality: u8,
    /// The estimated assembled size in bytes.
    pub est_bytes: u64,
}

/// A coarse estimate of the assembled GIF size for `frames` at `width`/`quality`.
///
/// GIF size grows with the pixel area and the frame count and falls with quality; the
/// constant is tuned so a full-width capture lands in the low megabytes. It only has
/// to pick a sensible starting point, since the real assembled size is re-checked and
/// shrunk if it overshoots.
fn est_bytes(frames: u32, width: u32, quality: u8) -> u64 {
    let per_frame = (u64::from(width) * u64::from(width) / 100) * u64::from(quality) / 100;
    per_frame * u64::from(frames)
}

/// Picks the largest width (then quality) whose size estimate fits `budget`.
#[must_use]
pub fn plan_encoding(frame_count: u32, budget: u64) -> EncodePlan {
    for &width in &WIDTH_LADDER {
        for &quality in &[85u8, 75, 65] {
            let est = est_bytes(frame_count, width, quality);
            if est <= budget {
                return EncodePlan {
                    width,
                    quality,
                    est_bytes: est,
                };
            }
        }
    }
    let width = WIDTH_LADDER[WIDTH_LADDER.len() - 1];
    EncodePlan {
        width,
        quality: 60,
        est_bytes: est_bytes(frame_count, width, 60),
    }
}

/// Handles `capture-ui`: run each committed demo script through the real app window
/// and assemble the resulting frames into the README media. `only` limits the run to
/// a single named capture.
pub fn cmd_capture_ui(only: Option<&str>) -> ExitCode {
    let assets = Path::new("assets");
    if let Err(e) = std::fs::create_dir_all(assets) {
        eprintln!("capture-ui: cannot create {}: {e}", assets.display());
        return ExitCode::FAILURE;
    }

    for name in CAPTURES {
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let script = format!("crates/reticle-app/demo-scripts/{name}.demo");
        if !Path::new(&script).exists() {
            eprintln!("capture-ui: no script {script}, skipping {name}");
            continue;
        }
        let frames_dir = PathBuf::from("scratch/ui-frames").join(name);
        let _ = std::fs::remove_dir_all(&frames_dir);

        println!("capture-ui: recording {name} (window opens) ...");
        let status = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "-p",
                "reticle-app",
                "--release",
                "--",
                "--demo-script",
                &script,
                "--out",
            ])
            .arg(&frames_dir)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("capture-ui: app for {name} exited with {s}");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("capture-ui: could not run the app for {name}: {e}");
                return ExitCode::FAILURE;
            }
        }

        match assemble_capture(name, &frames_dir, assets) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("capture-ui: {name}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("capture-ui: done");
    ExitCode::SUCCESS
}

/// Reads a capture's manifest and produces its asset (a copied still or a
/// budget-fitted GIF).
fn assemble_capture(name: &str, frames_dir: &Path, assets: &Path) -> Result<(), String> {
    let manifest_path = frames_dir.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parsing manifest: {e}"))?;

    let kind = manifest["kind"].as_str().unwrap_or("gif");
    let fps = u32::try_from(manifest["fps"].as_u64().unwrap_or(20)).unwrap_or(20);
    let frames: Vec<PathBuf> = manifest["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|f| frames_dir.join(f))
        .collect();
    if frames.is_empty() {
        return Err("captured no frames".to_owned());
    }

    if kind == "snap" {
        let out = assets.join(format!("{name}.png"));
        std::fs::copy(&frames[0], &out)
            .map_err(|e| format!("copying still to {}: {e}", out.display()))?;
        println!(
            "capture-ui: {} ({})",
            out.display(),
            human_size(file_len(&out))
        );
    } else {
        let out = assets.join(format!("{name}.gif"));
        let size = assemble_under_budget(&frames, &out, fps);
        println!("capture-ui: {} ({})", out.display(), human_size(size));
        if size > MAX_GIF_BYTES {
            return Err(format!(
                "{} is {} even at the smallest setting; over the 6 MB budget",
                out.display(),
                human_size(size)
            ));
        }
    }
    Ok(())
}

/// Assembles a GIF, shrinking width and quality down the ladder until it fits the
/// budget (or the smallest setting is reached). Returns the final file size.
fn assemble_under_budget(frames: &[PathBuf], out: &Path, fps: u32) -> u64 {
    let n = u32::try_from(frames.len()).unwrap_or(u32::MAX);
    let start = plan_encoding(n, MAX_GIF_BYTES);
    let start_idx = WIDTH_LADDER
        .iter()
        .position(|&w| w == start.width)
        .unwrap_or(0);
    let mut quality = start.quality;

    for &width in &WIDTH_LADDER[start_idx..] {
        assemble_gif_scaled(frames, out, fps, quality, width);
        let size = file_len(out);
        if size <= MAX_GIF_BYTES {
            return size;
        }
        eprintln!(
            "capture-ui: {} is {} at width {width} q{quality}; shrinking",
            out.display(),
            human_size(size)
        );
        quality = quality.saturating_sub(10).max(50);
    }
    file_len(out)
}

/// The length of `path` in bytes, or 0 if it cannot be read.
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// A short human-readable byte size.
fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_GIF_BYTES, plan_encoding};

    #[test]
    fn plan_fits_the_budget_and_stays_in_range() {
        let plan = plan_encoding(120, MAX_GIF_BYTES);
        assert!(
            plan.est_bytes <= MAX_GIF_BYTES,
            "estimate {} over budget",
            plan.est_bytes
        );
        assert!(
            (800..=1600).contains(&plan.width),
            "width {} out of range",
            plan.width
        );
    }

    #[test]
    fn a_bigger_frame_count_never_picks_a_larger_width() {
        let many = plan_encoding(5000, MAX_GIF_BYTES);
        let few = plan_encoding(40, MAX_GIF_BYTES);
        assert!(
            many.width <= few.width,
            "many-frame width {} should not exceed few-frame width {}",
            many.width,
            few.width
        );
    }

    #[test]
    fn full_width_when_the_clip_is_small() {
        // A short clip fits at full width and top quality.
        let plan = plan_encoding(60, MAX_GIF_BYTES);
        assert_eq!(plan.width, 1600);
        assert_eq!(plan.quality, 85);
    }
}

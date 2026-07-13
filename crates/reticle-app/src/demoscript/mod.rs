//! Scripted demo-capture mode: drive the real editor window through a timed step
//! list and capture full-window frames via egui's viewport screenshot.
//!
//! The rendered media that ships in the README comes from here: a native launch of
//! the app (`--demo-script <file>` / `--screenshot-smoke <path>`) plays a committed
//! script, requests a full-window screenshot at each capture step, writes each frame
//! as a PNG, and lets `xtask capture-ui` assemble them into GIFs. A windowed
//! screenshot has no meaning on wasm, so the capture plumbing (PNG writing, the
//! capture state machine) is gated off wasm; the pure frame helpers stay portable so
//! they compile and unit-test everywhere.

use eframe::egui;

mod script;
pub use script::{Script, Step};

#[cfg(not(target_arch = "wasm32"))]
mod run;
#[cfg(not(target_arch = "wasm32"))]
pub use run::{DemoRun, Tick};

/// One captured frame: tightly packed RGBA8, row 0 at the top.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, row-major, top row first.
    pub rgba: Vec<u8>,
}

/// Converts an egui screenshot reply into a [`Frame`] (straight RGBA8).
#[must_use]
pub fn frame_from_color_image(image: &egui::ColorImage) -> Frame {
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w.saturating_mul(h).saturating_mul(4));
    for px in &image.pixels {
        rgba.extend_from_slice(&px.to_array());
    }
    Frame {
        width: u32::try_from(w).expect("screenshot width fits in u32"),
        height: u32::try_from(h).expect("screenshot height fits in u32"),
        rgba,
    }
}

/// Whether the frame varies (a real capture) rather than being a single flat color.
///
/// A blank (single-color) readback is the failure mode a windowed screenshot can hit
/// when the backend does not honor the request, so the smoke path asserts on this
/// before the harness is trusted.
#[must_use]
pub fn is_nonblank(frame: &Frame) -> bool {
    let Some(first) = frame.rgba.get(0..4) else {
        return false;
    };
    frame.rgba.chunks_exact(4).any(|px| px != first)
}

/// The minimum non-background significant color buckets a shipped media frame must show.
/// A starry point-scatter, a blank readback, or a flat fill has fewer. Mirrors the JS
/// media gate (`e2e/media-gate.mjs` `MIN_NONBG_BUCKETS`) so the native tour captures and
/// the headed browser captures gate on the same rule.
pub const MIN_NONBG_BUCKETS: usize = 3;

/// Significant-color-bucket statistics for the media gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BucketStats {
    /// Buckets covering at least 0.5% of pixels (background included).
    pub significant: usize,
    /// Significant buckets excluding the single most common (background) bucket.
    pub non_background: usize,
}

/// Histograms `frame` into 5-bit-per-channel color buckets and reports how many are
/// significant (cover at least 0.5% of pixels). A correct multi-layer render has several
/// non-background buckets; a starry point-scatter or a flat fill has few. This is the same
/// rule the browser gate applies to a canvas screenshot.
#[must_use]
pub fn color_buckets(frame: &Frame) -> BucketStats {
    let total = (frame.rgba.len() / 4) as u64;
    if total == 0 {
        return BucketStats {
            significant: 0,
            non_background: 0,
        };
    }
    // 5 bits per channel => 32768 buckets.
    let mut counts = vec![0u32; 1 << 15];
    for px in frame.rgba.chunks_exact(4) {
        let key = (usize::from(px[0] >> 3) << 10)
            | (usize::from(px[1] >> 3) << 5)
            | usize::from(px[2] >> 3);
        counts[key] = counts[key].saturating_add(1);
    }
    // Significant = covers >= 0.5% (1/200) of pixels; integer form avoids float.
    let significant = counts
        .iter()
        .filter(|&&n| u64::from(n) * 200 >= total)
        .count();
    BucketStats {
        significant,
        non_background: significant.saturating_sub(1),
    }
}

/// Like [`color_buckets`] but over the CENTRAL geometry viewport only, excluding the left
/// layer panel, the right inspector, the top toolbar, and the bottom status bar. In this
/// egui app the whole UI paints to one surface, so a full-frame histogram is dominated by
/// chrome (already several colors) and would pass even a starry or empty viewport. The
/// media gate uses this so it measures the geometry, not the chrome.
#[must_use]
pub fn color_buckets_center(frame: &Frame) -> BucketStats {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w == 0 || h == 0 || frame.rgba.len() < w.saturating_mul(h).saturating_mul(4) {
        return BucketStats {
            significant: 0,
            non_background: 0,
        };
    }
    // The geometry viewport as a fraction of the window (panels/toolbar/status excluded).
    let frac = |f: f32, n: usize| ((f * n as f32) as usize).min(n);
    let x0 = frac(0.20, w);
    let x1 = frac(0.70, w);
    let y0 = frac(0.12, h);
    let y1 = frac(0.88, h);
    let total = (x1.saturating_sub(x0) as u64) * (y1.saturating_sub(y0) as u64);
    if total == 0 {
        return BucketStats {
            significant: 0,
            non_background: 0,
        };
    }
    let mut counts = vec![0u32; 1 << 15];
    for y in y0..y1 {
        let row = y * w;
        for x in x0..x1 {
            let i = (row + x) * 4;
            let px = &frame.rgba[i..i + 3];
            let key = (usize::from(px[0] >> 3) << 10)
                | (usize::from(px[1] >> 3) << 5)
                | usize::from(px[2] >> 3);
            counts[key] = counts[key].saturating_add(1);
        }
    }
    let significant = counts
        .iter()
        .filter(|&&n| u64::from(n) * 200 >= total)
        .count();
    BucketStats {
        significant,
        non_background: significant.saturating_sub(1),
    }
}

/// The media-gate verdict over a captured clip's per-frame non-background bucket counts.
///
/// A still (`is_snap`) must itself clear [`MIN_NONBG_BUCKETS`]; a GIF's MEDIAN frame must
/// clear it (a few transition or legitimate solid-fill frames are tolerated), and no frame
/// may be blank. On failure the capture aborts so nothing starry/flat ships
/// (reject-and-recapture).
///
/// # Errors
///
/// Returns an error if the clip is empty, a captured frame is blank, or the representative
/// frame has fewer than [`MIN_NONBG_BUCKETS`] non-background color buckets.
pub fn gate_verdict(nonbg_per_frame: &[usize], is_snap: bool) -> Result<(), String> {
    if nonbg_per_frame.is_empty() {
        return Err("media gate: no frames were captured".to_owned());
    }
    let mut sorted = nonbg_per_frame.to_vec();
    sorted.sort_unstable();
    if sorted[0] < 1 {
        return Err(format!(
            "media gate: a captured frame is blank (0 non-background buckets); distribution {sorted:?}"
        ));
    }
    let representative = if is_snap {
        sorted[0]
    } else {
        sorted[sorted.len() / 2]
    };
    if representative < MIN_NONBG_BUCKETS {
        return Err(format!(
            "media gate: {} frame has {representative} non-background buckets \
             (< {MIN_NONBG_BUCKETS}); the clip is starry/flat. distribution {sorted:?}",
            if is_snap { "still" } else { "median" }
        ));
    }
    Ok(())
}

/// Writes `frame` to `path` as a PNG, creating parent directories.
///
/// # Errors
///
/// Returns an error if the buffer size does not match `width * height * 4`, a parent
/// directory cannot be created, or the encode/write fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_frame_png(path: &std::path::Path, frame: &Frame) -> std::io::Result<()> {
    let buf = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        .ok_or_else(|| std::io::Error::other("frame buffer size does not match its dimensions"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    buf.save(path).map_err(std::io::Error::other)
}

/// A one-shot capture used by the `--screenshot-smoke` de-risking path: wait a few
/// frames for the scene to settle, request one screenshot, save it, and report
/// whether it was non-blank so the window can close.
///
/// This is the smallest possible exercise of the full round trip
/// (`ViewportCommand::Screenshot` -> `Event::Screenshot` -> PNG) on the real wgpu
/// window, which is the single biggest risk in the media harness.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct CaptureState {
    out_path: std::path::PathBuf,
    /// Frames to let the retained GPU scene build and the camera fit before capture.
    warmup: u32,
    /// Whether the screenshot command has been sent.
    requested: bool,
    /// Frames waited since the request, so a backend that never replies gives up.
    waited: u32,
    /// Set once the capture (or the give-up) is done.
    finished: bool,
    /// Whether the captured frame varied (a real, non-blank capture).
    nonblank: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl CaptureState {
    /// A smoke capture that writes a single frame to `out_path`.
    #[must_use]
    pub fn smoke(out_path: std::path::PathBuf) -> Self {
        Self {
            out_path,
            warmup: 12,
            requested: false,
            waited: 0,
            finished: false,
            nonblank: false,
        }
    }

    /// Advances the capture by one frame. Returns `true` when finished (the caller
    /// should drop the state and close the window).
    ///
    /// The dance: warm up, send `ViewportCommand::Screenshot`, then on a later frame
    /// find the `Event::Screenshot` reply, save it, and stop. If no reply arrives
    /// within a bounded number of frames the backend does not support windowed
    /// screenshots on this host; the smoke reports that and stops so the operator can
    /// fall back to a window-capture path.
    pub fn tick(&mut self, ctx: &egui::Context) -> bool {
        if self.finished {
            return true;
        }
        if self.requested {
            let shot = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = shot {
                let frame = frame_from_color_image(&image);
                self.nonblank = is_nonblank(&frame);
                if let Err(e) = save_frame_png(&self.out_path, &frame) {
                    eprintln!("smoke: save failed: {e}");
                }
                eprintln!(
                    "smoke: {} ({}x{}) -> {}",
                    if self.nonblank {
                        "nonblank OK"
                    } else {
                        "BLANK"
                    },
                    frame.width,
                    frame.height,
                    self.out_path.display()
                );
                self.finished = true;
                return true;
            }
            self.waited += 1;
            if self.waited > 180 {
                eprintln!(
                    "smoke: no Event::Screenshot after {} frames; this backend may not \
                     support windowed viewport screenshots (fall back to window capture)",
                    self.waited
                );
                self.finished = true;
                return true;
            }
            return false;
        }
        if self.warmup > 0 {
            self.warmup -= 1;
            return false;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        self.requested = true;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Frame, MIN_NONBG_BUCKETS, color_buckets, color_buckets_center, gate_verdict, is_nonblank,
    };

    /// A 20x20 frame split into four colored quadrants (or one flat color if `flat`).
    fn quadrant_frame(flat: bool) -> Frame {
        let colors = [[20u8, 20, 20], [220, 20, 20], [20, 220, 20], [20, 20, 220]];
        let mut rgba = Vec::with_capacity(20 * 20 * 4);
        for y in 0..20u32 {
            for x in 0..20u32 {
                let q = if flat {
                    0
                } else {
                    usize::from(x >= 10) + 2 * usize::from(y >= 10)
                };
                let c = colors[q];
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Frame {
            width: 20,
            height: 20,
            rgba,
        }
    }

    /// A one-row frame from a list of opaque RGB pixels.
    fn frame_from_pixels(pixels: &[[u8; 3]]) -> Frame {
        let mut rgba = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        Frame {
            width: u32::try_from(pixels.len()).unwrap(),
            height: 1,
            rgba,
        }
    }

    #[test]
    fn color_buckets_counts_distinct_layers() {
        let four = frame_from_pixels(&[[10, 10, 10], [200, 10, 10], [10, 200, 10], [10, 10, 200]]);
        let b = color_buckets(&four);
        assert_eq!(b.significant, 4);
        assert_eq!(b.non_background, 3);

        let flat = frame_from_pixels(&[[10, 10, 10], [10, 10, 10]]);
        assert_eq!(
            color_buckets(&flat).non_background,
            0,
            "a flat fill has no layers"
        );

        // Starry: a dark field with a few sparse specks, each below 0.5% of pixels.
        let mut px = vec![[8, 8, 8]; 1000];
        px.push([250, 250, 250]);
        px.push([250, 10, 10]);
        px.push([10, 250, 10]);
        let starry = frame_from_pixels(&px);
        assert_eq!(
            color_buckets(&starry).non_background,
            0,
            "sparse specks are not significant, so a starry frame gates as blank",
        );
    }

    #[test]
    fn color_buckets_center_ignores_the_chrome_and_measures_the_viewport() {
        // Four colored quadrants across the central crop => three non-background buckets.
        assert_eq!(
            color_buckets_center(&quadrant_frame(false)).non_background,
            3
        );
        // A flat central region gates as blank even if a real frame's chrome is colorful.
        assert_eq!(
            color_buckets_center(&quadrant_frame(true)).non_background,
            0
        );
    }

    #[test]
    fn gate_verdict_matches_the_asset_rules() {
        // A still must itself clear the bar.
        assert!(gate_verdict(&[5], true).is_ok());
        assert!(gate_verdict(&[2], true).is_err());
        // A GIF passes on its MEDIAN frame; a few sparse/solid frames are tolerated.
        assert!(gate_verdict(&[1, 1, 5, 6, 6], false).is_ok());
        // But a blank frame always fails, and a starry median fails.
        assert!(gate_verdict(&[0, 3, 3], false).is_err());
        assert!(gate_verdict(&[1, 2, 2, 2, 2], false).is_err());
        assert!(gate_verdict(&[], false).is_err());
        assert_eq!(MIN_NONBG_BUCKETS, 3);
    }

    #[test]
    fn nonblank_detects_variation() {
        let flat = Frame {
            width: 2,
            height: 1,
            rgba: vec![10, 10, 10, 255, 10, 10, 10, 255],
        };
        assert!(!is_nonblank(&flat), "a single flat color is blank");

        let varied = Frame {
            width: 2,
            height: 1,
            rgba: vec![10, 10, 10, 255, 200, 10, 10, 255],
        };
        assert!(is_nonblank(&varied), "two different pixels is non-blank");
    }

    #[test]
    fn empty_frame_is_blank() {
        let empty = Frame {
            width: 0,
            height: 0,
            rgba: vec![],
        };
        assert!(!is_nonblank(&empty));
    }
}

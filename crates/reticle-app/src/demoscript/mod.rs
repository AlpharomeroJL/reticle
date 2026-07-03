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
    use super::{Frame, is_nonblank};

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

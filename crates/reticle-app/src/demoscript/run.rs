//! The demo-run scheduler: sequence a [`Script`] into per-frame ticks and collect
//! the captured frames, so `App::ui` can drive the real window and screenshot it.
//!
//! The app is not headless: a screenshot needs a real drawn frame, so the run is a
//! small state machine advanced once per `App::ui` frame. Each call to [`next_tick`] hands
//! the app one instruction ([`Tick`]): idle, apply one instantaneous action, capture
//! a frame (request a screenshot, optionally orbiting the 3D camera first), or save
//! the reply that a capture request produced on the previous frame. Frames land as
//! PNG files under `out_dir` with a `manifest.json` describing them, which `xtask
//! capture-ui` reads to assemble the GIF (or use the single still).
//!
//! [`next_tick`]: DemoRun::next_tick

use std::path::PathBuf;

use super::{Frame, MIN_NONBG_BUCKETS, Script, Step, color_buckets_center, save_frame_png};

/// What `App` should do on the current frame, decided by [`DemoRun::next_tick`].
#[derive(Clone, Debug)]
pub enum Tick {
    /// Nothing to do this frame (warmup or a `wait`).
    Idle,
    /// Apply this instantaneous action to the app, then continue next frame.
    Apply(Step),
    /// Request a full-window screenshot this frame, orbiting the 3D camera first by
    /// `orbit` `(dx, dy)` so a following capture segment animates.
    Capture {
        /// Per-frame 3D orbit delta to apply before capturing.
        orbit: (f32, f32),
    },
    /// A screenshot was requested last frame; read its reply and store it.
    Save,
    /// The script is complete; the window should close.
    Done,
}

/// Whether the run captures a GIF (many frames) or a single still.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Gif,
    Snap,
}

/// Drives a [`Script`] frame by frame and collects the captured frames.
#[derive(Debug)]
pub struct DemoRun {
    viewport: (u32, u32),
    steps: Vec<Step>,
    cursor: usize,
    out_dir: PathBuf,
    warmup: u32,
    wait: u32,
    orbit: (f32, f32),
    kind: Kind,
    seg_remaining: u32,
    fps: u32,
    awaiting: bool,
    misses: u32,
    frames: Vec<String>,
    /// Non-background color-bucket count of each stored frame, for the media gate.
    nonbg: Vec<usize>,
    /// Remaining `settle` probe budget while waiting for a colored render, or `None`.
    settling: Option<u32>,
    /// Whether the screenshot in flight is a `settle` probe (checked, not stored).
    probing: bool,
    done: bool,
}

impl DemoRun {
    /// Frames to idle before the first step, so the GPU device and first paint settle.
    const WARMUP_FRAMES: u32 = 6;

    /// Builds a run for `script`, writing frames under `out_dir`.
    #[must_use]
    pub fn new(script: Script, out_dir: PathBuf) -> Self {
        Self {
            viewport: script.viewport,
            steps: script.steps,
            cursor: 0,
            out_dir,
            warmup: Self::WARMUP_FRAMES,
            wait: 0,
            orbit: (0.0, 0.0),
            kind: Kind::Gif,
            seg_remaining: 0,
            fps: 20,
            awaiting: false,
            misses: 0,
            frames: Vec::new(),
            nonbg: Vec::new(),
            settling: None,
            probing: false,
            done: false,
        }
    }

    /// The window size the script asked for.
    #[must_use]
    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    /// Whether the run has finished.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// How many frames have been captured so far.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Decides what `App` should do on this frame.
    pub fn next_tick(&mut self) -> Tick {
        if self.done {
            return Tick::Done;
        }
        if self.awaiting {
            return Tick::Save;
        }
        if self.warmup > 0 {
            self.warmup -= 1;
            return Tick::Idle;
        }
        loop {
            if self.wait > 0 {
                self.wait -= 1;
                return Tick::Idle;
            }
            if self.settling.is_some() {
                // Probe the current frame for a colored render; the reply routes to the
                // settle check (receive_frame), not to a stored capture.
                self.awaiting = true;
                self.probing = true;
                return Tick::Capture { orbit: (0.0, 0.0) };
            }
            if self.seg_remaining > 0 {
                self.awaiting = true;
                return Tick::Capture { orbit: self.orbit };
            }
            if self.cursor >= self.steps.len() {
                self.done = true;
                return Tick::Done;
            }
            let step = self.steps[self.cursor].clone();
            self.cursor += 1;
            match step {
                Step::Wait(n) => self.wait = n,
                Step::Settle(max) => self.settling = Some(max.max(1)),
                Step::Orbit(dx, dy) => self.orbit = (dx, dy),
                Step::Capture { frames, fps } => {
                    self.kind = Kind::Gif;
                    self.fps = fps;
                    self.seg_remaining = frames;
                }
                Step::Snap(_) => {
                    self.kind = Kind::Snap;
                    self.seg_remaining = 1;
                }
                other => return Tick::Apply(other),
            }
        }
    }

    /// Routes a captured screenshot the app read on a [`Tick::Save`]: during a `settle`
    /// probe it checks whether the frame is a colored render (clearing the settle once it
    /// is, or after the probe budget runs out), otherwise it stores the frame as a capture.
    pub fn receive_frame(&mut self, frame: &Frame) {
        if self.probing {
            self.awaiting = false;
            self.misses = 0;
            self.probing = false;
            if color_buckets_center(frame).non_background >= MIN_NONBG_BUCKETS {
                self.settling = None;
            } else if let Some(budget) = self.settling {
                let left = budget.saturating_sub(1);
                self.settling = (left > 0).then_some(left);
            }
            return;
        }
        self.store_frame(frame);
    }

    /// Stores a captured frame and records its non-background color-bucket count for the
    /// media gate; the app reaches this through [`receive_frame`](Self::receive_frame).
    pub fn store_frame(&mut self, frame: &Frame) {
        self.awaiting = false;
        self.misses = 0;
        let name = format!("frame_{:04}.png", self.frames.len());
        let path = self.out_dir.join(&name);
        if let Err(e) = save_frame_png(&path, frame) {
            eprintln!("demo: failed to write {}: {e}", path.display());
        }
        self.frames.push(name);
        self.nonbg.push(color_buckets_center(frame).non_background);
        self.seg_remaining = self.seg_remaining.saturating_sub(1);
    }

    /// The media-gate verdict over the captured frames: fails if the clip is starry,
    /// blank, or flat so a bad asset never assembles (reject-and-recapture).
    ///
    /// # Errors
    ///
    /// Returns an error describing the failing per-frame distribution.
    pub fn media_gate(&self) -> Result<(), String> {
        super::gate_verdict(&self.nonbg, self.kind == Kind::Snap)
    }

    /// Records that no `Screenshot` reply was present on a [`Tick::Save`]; gives up if
    /// the backend has gone silent for too long so a run can never hang forever.
    pub fn miss(&mut self) {
        self.misses += 1;
        if self.misses > 240 {
            eprintln!("demo: no screenshot replies for 240 frames; aborting run");
            self.awaiting = false;
            self.done = true;
        }
    }

    /// Writes `manifest.json` describing the captured frames and returns its path.
    ///
    /// # Errors
    ///
    /// Returns an error if the output directory or the manifest file cannot be written.
    pub fn write_manifest(&self) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.out_dir)?;
        let kind = match self.kind {
            Kind::Gif => "gif",
            Kind::Snap => "snap",
        };
        let manifest = serde_json::json!({
            "viewport": [self.viewport.0, self.viewport.1],
            "kind": kind,
            "fps": self.fps,
            "frames": self.frames,
        });
        let json = serde_json::to_string_pretty(&manifest).map_err(std::io::Error::other)?;
        let path = self.out_dir.join("manifest.json");
        std::fs::write(&path, json)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{DemoRun, Tick};
    use crate::demoscript::{Script, Step};

    /// A run with no capture segments reaches `Done` after warmup and its steps.
    #[test]
    fn advances_through_instant_steps_to_done() {
        let script = Script {
            viewport: (800, 600),
            steps: vec![Step::RunDrc, Step::Wait(2)],
        };
        let mut run = DemoRun::new(script, std::env::temp_dir().join("reticle-demo-test-none"));

        // Warmup frames are idle.
        for _ in 0..DemoRun::WARMUP_FRAMES {
            assert!(matches!(run.next_tick(), Tick::Idle));
        }
        // Then the RunDrc step is applied.
        assert!(matches!(run.next_tick(), Tick::Apply(Step::RunDrc)));
        // The Wait(2) idles two frames.
        assert!(matches!(run.next_tick(), Tick::Idle));
        assert!(matches!(run.next_tick(), Tick::Idle));
        // Then the run is done.
        assert!(matches!(run.next_tick(), Tick::Done));
        assert!(run.is_done());
    }

    /// A capture segment yields one `Capture` then one `Save` per requested frame.
    #[test]
    fn capture_segment_pipelines_request_then_save() {
        let script = Script {
            viewport: (800, 600),
            steps: vec![Step::Capture { frames: 2, fps: 20 }],
        };
        let mut run = DemoRun::new(script, std::env::temp_dir().join("reticle-demo-test-cap"));
        for _ in 0..DemoRun::WARMUP_FRAMES {
            assert!(matches!(run.next_tick(), Tick::Idle));
        }
        // Frame 1: request, then the app is told to save.
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.store_frame(&blank_frame());
        // Frame 2.
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.store_frame(&blank_frame());
        assert_eq!(run.frame_count(), 2);
        assert!(matches!(run.next_tick(), Tick::Done));
    }

    #[test]
    fn settle_probes_until_a_colored_render_then_proceeds() {
        let script = Script {
            viewport: (8, 8),
            steps: vec![Step::Settle(5), Step::Capture { frames: 1, fps: 20 }],
        };
        let mut run = DemoRun::new(
            script,
            std::env::temp_dir().join("reticle-demo-test-settle"),
        );
        for _ in 0..DemoRun::WARMUP_FRAMES {
            assert!(matches!(run.next_tick(), Tick::Idle));
        }
        // First settle probe: a blank frame keeps it settling and stores nothing.
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.receive_frame(&blank_frame());
        assert_eq!(run.frame_count(), 0, "a probe frame is not stored");
        // Second probe: a colored frame ends the settle.
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.receive_frame(&colored_frame());
        // Now the real capture segment runs and stores its frame.
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.receive_frame(&colored_frame());
        assert_eq!(run.frame_count(), 1);
        assert!(matches!(run.next_tick(), Tick::Done));
        assert!(run.media_gate().is_ok());
    }

    #[test]
    fn media_gate_rejects_a_blank_capture() {
        let script = Script {
            viewport: (8, 8),
            steps: vec![Step::Capture { frames: 1, fps: 20 }],
        };
        let mut run = DemoRun::new(script, std::env::temp_dir().join("reticle-demo-test-gate"));
        for _ in 0..DemoRun::WARMUP_FRAMES {
            run.next_tick();
        }
        assert!(matches!(run.next_tick(), Tick::Capture { .. }));
        assert!(matches!(run.next_tick(), Tick::Save));
        run.receive_frame(&blank_frame());
        assert!(
            run.media_gate().is_err(),
            "a blank capture must fail the gate"
        );
    }

    fn blank_frame() -> super::Frame {
        // A 20x20 flat fill: large enough for the central crop, but a single color, so it
        // gates as blank (zero non-background buckets).
        let mut rgba = Vec::with_capacity(20 * 20 * 4);
        for _ in 0..(20 * 20) {
            rgba.extend_from_slice(&[0, 0, 0, 255]);
        }
        super::Frame {
            width: 20,
            height: 20,
            rgba,
        }
    }

    fn colored_frame() -> super::Frame {
        // A 20x20 frame in four distinctly colored quadrants: the central crop sees all
        // four, i.e. three non-background buckets.
        let colors = [[20u8, 20, 20], [220, 20, 20], [20, 220, 20], [20, 20, 220]];
        let mut rgba = Vec::with_capacity(20 * 20 * 4);
        for y in 0..20u32 {
            for x in 0..20u32 {
                let q = usize::from(x >= 10) + 2 * usize::from(y >= 10);
                let c = colors[q];
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        super::Frame {
            width: 20,
            height: 20,
            rgba,
        }
    }
}

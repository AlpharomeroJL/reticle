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

use super::{Frame, Script, Step, save_frame_png};

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

    /// Stores a captured frame; the app calls this after reading the `Screenshot`
    /// event that a [`Tick::Capture`] produced.
    pub fn store_frame(&mut self, frame: &Frame) {
        self.awaiting = false;
        self.misses = 0;
        let name = format!("frame_{:04}.png", self.frames.len());
        let path = self.out_dir.join(&name);
        if let Err(e) = save_frame_png(&path, frame) {
            eprintln!("demo: failed to write {}: {e}", path.display());
        }
        self.frames.push(name);
        self.seg_remaining = self.seg_remaining.saturating_sub(1);
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

    fn blank_frame() -> super::Frame {
        super::Frame {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        }
    }
}

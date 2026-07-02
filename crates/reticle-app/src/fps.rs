//! A rolling frame-time meter for the status bar.
//!
//! [`FrameMeter`] keeps a fixed-size window of recent frame durations and reports a
//! smoothed frames-per-second and average frame time. It is deliberately free of any
//! clock or egui type: the egui layer samples the frame duration once per repaint
//! (from `ctx.input(|i| i.stable_dt)`) and feeds it in, so the averaging logic is
//! pure and unit-tested without a window or a real timer.

use std::collections::VecDeque;
use std::time::Duration;

/// The default number of frames averaged over. About a third of a second at 60 fps:
/// long enough to steady the readout, short enough to react to a real slowdown.
pub const DEFAULT_WINDOW: usize = 20;

/// A rolling average of recent frame durations.
///
/// Push one sample per frame with [`FrameMeter::record`]; read the smoothed rate and
/// time with [`FrameMeter::fps`] and [`FrameMeter::frame_ms`]. Only the most recent
/// [`FrameMeter::window`] samples are kept.
#[derive(Clone, Debug)]
pub struct FrameMeter {
    /// Recent frame durations, oldest at the front.
    samples: VecDeque<Duration>,
    /// The maximum number of samples retained.
    window: usize,
}

impl Default for FrameMeter {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

impl FrameMeter {
    /// Creates a meter averaging over the last `window` frames (clamped to at least
    /// one).
    #[must_use]
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            samples: VecDeque::with_capacity(window),
            window,
        }
    }

    /// The window size (number of frames averaged over).
    #[must_use]
    pub fn window(&self) -> usize {
        self.window
    }

    /// The number of samples currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Records one frame's duration, evicting the oldest sample past the window.
    ///
    /// Zero and absurdly small durations are kept as-is (the readers guard against a
    /// zero average), so a paused or first frame does not corrupt the window.
    pub fn record(&mut self, dt: Duration) {
        if self.samples.len() == self.window {
            self.samples.pop_front();
        }
        self.samples.push_back(dt);
    }

    /// The average frame duration over the window, or `None` if no samples yet.
    #[must_use]
    pub fn average(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let total: Duration = self.samples.iter().sum();
        Some(total / self.samples.len() as u32)
    }

    /// The average frame time in milliseconds, or `0.0` with no samples.
    #[must_use]
    pub fn frame_ms(&self) -> f64 {
        self.average().map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }

    /// The smoothed frames-per-second, or `0.0` with no samples or a zero average.
    ///
    /// A zero (or negative-rounding) average yields `0.0` rather than an infinity, so
    /// the status readout never shows `inf`.
    #[must_use]
    pub fn fps(&self) -> f64 {
        match self.average() {
            Some(d) if d > Duration::ZERO => 1.0 / d.as_secs_f64(),
            _ => 0.0,
        }
    }

    /// A compact status string like `"60.0 fps  (16.7 ms)"`, or `"-- fps"` before any
    /// frame is recorded.
    #[must_use]
    pub fn label(&self) -> String {
        if self.is_empty() {
            return "-- fps".to_owned();
        }
        format!("{:.1} fps  ({:.1} ms)", self.fps(), self.frame_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_WINDOW, FrameMeter};
    use std::time::Duration;

    // The meter returns literal `0.0` in the no-samples and zero-average cases, so
    // exact float equality is the precise assertion here.
    #[test]
    #[allow(clippy::float_cmp)]
    fn empty_meter_reports_zero_and_placeholder() {
        let m = FrameMeter::default();
        assert!(m.is_empty());
        assert_eq!(m.window(), DEFAULT_WINDOW);
        assert_eq!(m.fps(), 0.0);
        assert_eq!(m.frame_ms(), 0.0);
        assert!(m.average().is_none());
        assert_eq!(m.label(), "-- fps");
    }

    #[test]
    fn steady_16_67ms_reads_60fps() {
        let mut m = FrameMeter::new(10);
        for _ in 0..10 {
            m.record(Duration::from_micros(16_667)); // 1/60 s
        }
        assert!((m.fps() - 60.0).abs() < 0.1, "fps was {}", m.fps());
        assert!(
            (m.frame_ms() - 16.667).abs() < 0.01,
            "ms was {}",
            m.frame_ms()
        );
    }

    #[test]
    fn window_evicts_oldest_samples() {
        let mut m = FrameMeter::new(3);
        // Fill with slow frames, then push fast ones; only the last 3 count.
        m.record(Duration::from_millis(100));
        m.record(Duration::from_millis(100));
        m.record(Duration::from_millis(100));
        assert_eq!(m.len(), 3);
        for _ in 0..3 {
            m.record(Duration::from_millis(10)); // 100 fps
        }
        assert_eq!(m.len(), 3, "window stays capped at 3");
        assert!((m.fps() - 100.0).abs() < 0.5, "fps was {}", m.fps());
    }

    #[test]
    fn average_is_the_mean_of_the_window() {
        let mut m = FrameMeter::new(4);
        m.record(Duration::from_millis(10));
        m.record(Duration::from_millis(20));
        m.record(Duration::from_millis(30));
        m.record(Duration::from_millis(40));
        // Mean = 25 ms.
        assert_eq!(m.average(), Some(Duration::from_millis(25)));
        assert!((m.frame_ms() - 25.0).abs() < 1e-9);
        assert!((m.fps() - 40.0).abs() < 0.01); // 1000/25
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn zero_duration_average_yields_zero_fps_not_infinity() {
        let mut m = FrameMeter::new(2);
        m.record(Duration::ZERO);
        m.record(Duration::ZERO);
        assert_eq!(m.fps(), 0.0);
        assert!(m.fps().is_finite());
    }

    #[test]
    fn window_of_zero_is_clamped_to_one() {
        let mut m = FrameMeter::new(0);
        assert_eq!(m.window(), 1);
        m.record(Duration::from_millis(20));
        m.record(Duration::from_millis(10));
        // Only the most recent sample is retained.
        assert_eq!(m.len(), 1);
        assert!((m.fps() - 100.0).abs() < 0.5);
    }

    #[test]
    fn label_formats_rate_and_time() {
        let mut m = FrameMeter::new(1);
        m.record(Duration::from_micros(16_667));
        let label = m.label();
        assert!(label.contains("fps"), "label was {label}");
        assert!(label.contains("ms"), "label was {label}");
        assert!(label.starts_with("60.0"), "label was {label}");
    }
}

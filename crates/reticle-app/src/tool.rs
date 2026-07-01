//! The canvas tool state machine.
//!
//! Exactly one [`Tool`] is active at a time and it decides how pointer input on the
//! canvas is interpreted: [`Tool::Select`] picks shapes and rubber-bands,
//! [`Tool::Pan`] drags the view, and [`Tool::Measure`] captures two points and
//! reports the distance between them. The machine here is pure state; the egui
//! layer reads [`ToolState::active`] each frame and routes interaction accordingly.

use crate::measure::Measurement;
use reticle_geometry::Point;

/// A canvas interaction tool.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    /// Select and rubber-band shapes.
    #[default]
    Select,
    /// Pan the view by dragging.
    Pan,
    /// Measure the distance between two clicked points.
    Measure,
}

impl Tool {
    /// A short human-readable label for toolbars and the command palette.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Pan => "Pan",
            Self::Measure => "Measure",
        }
    }

    /// Every tool, in toolbar order.
    #[must_use]
    pub fn all() -> [Tool; 3] {
        [Tool::Select, Tool::Pan, Tool::Measure]
    }
}

/// The mutable tool state: which tool is active and the measure tool's progress.
#[derive(Clone, Debug, Default)]
pub struct ToolState {
    active: Tool,
    /// The first measure point, once placed; `None` before the first click.
    measure_start: Option<Point>,
    /// The completed measurement, if two points have been placed.
    measurement: Option<Measurement>,
}

impl ToolState {
    /// Creates a tool state defaulting to [`Tool::Select`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently active tool.
    #[must_use]
    pub fn active(&self) -> Tool {
        self.active
    }

    /// Switches to `tool`, resetting any in-progress measurement.
    ///
    /// Switching tools always clears a half-placed measure point so a stale start
    /// never leaks into the next measurement.
    pub fn set_active(&mut self, tool: Tool) {
        if self.active != tool {
            self.measure_start = None;
        }
        self.active = tool;
    }

    /// The measure tool's first point, if one has been placed but not yet closed.
    #[must_use]
    pub fn measure_start(&self) -> Option<Point> {
        self.measure_start
    }

    /// The most recent completed measurement, if any.
    #[must_use]
    pub fn measurement(&self) -> Option<&Measurement> {
        self.measurement.as_ref()
    }

    /// Handles a click at world point `at` while the measure tool is active.
    ///
    /// The first click records the start point; the second click completes the
    /// measurement (using `dbu_per_micron` to also report a physical distance) and
    /// arms the tool for a fresh measurement on the next click. Returns the
    /// completed [`Measurement`] on the closing click.
    pub fn measure_click(&mut self, at: Point, dbu_per_micron: i64) -> Option<Measurement> {
        match self.measure_start.take() {
            None => {
                self.measure_start = Some(at);
                None
            }
            Some(start) => {
                let m = Measurement::new(start, at, dbu_per_micron);
                self.measurement = Some(m);
                Some(m)
            }
        }
    }

    /// Clears any in-progress and completed measurement.
    pub fn clear_measure(&mut self) {
        self.measure_start = None;
        self.measurement = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_select() {
        let s = ToolState::new();
        assert_eq!(s.active(), Tool::Select);
    }

    #[test]
    fn switching_tool_changes_active() {
        let mut s = ToolState::new();
        s.set_active(Tool::Pan);
        assert_eq!(s.active(), Tool::Pan);
        s.set_active(Tool::Measure);
        assert_eq!(s.active(), Tool::Measure);
    }

    #[test]
    fn switching_tool_resets_measure_start() {
        let mut s = ToolState::new();
        s.set_active(Tool::Measure);
        assert!(s.measure_click(Point::new(0, 0), 1000).is_none());
        assert!(s.measure_start().is_some());
        s.set_active(Tool::Select);
        assert!(s.measure_start().is_none());
    }

    #[test]
    fn two_clicks_complete_a_measurement() {
        let mut s = ToolState::new();
        s.set_active(Tool::Measure);
        assert!(s.measure_click(Point::new(0, 0), 1000).is_none());
        let m = s
            .measure_click(Point::new(3000, 4000), 1000)
            .expect("second click closes");
        assert_eq!(m.distance_dbu().round() as i64, 5000);
        // The tool re-arms for another measurement.
        assert!(s.measure_start().is_none());
        assert!(s.measurement().is_some());
    }

    #[test]
    fn third_click_starts_a_new_measurement() {
        let mut s = ToolState::new();
        s.set_active(Tool::Measure);
        s.measure_click(Point::new(0, 0), 1000);
        s.measure_click(Point::new(1000, 0), 1000);
        // Third click begins a fresh measurement.
        assert!(s.measure_click(Point::new(5000, 0), 1000).is_none());
        assert_eq!(s.measure_start(), Some(Point::new(5000, 0)));
    }

    #[test]
    fn all_tools_have_labels() {
        for t in Tool::all() {
            assert!(!t.label().is_empty());
        }
    }
}

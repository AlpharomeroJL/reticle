//! Distance measurement between two world points.
//!
//! [`Measurement`] captures the two endpoints of a measure-tool gesture and derives
//! the axis deltas and the Euclidean distance in both database units and microns.
//! It is pure arithmetic so the measure tool can be tested without a canvas.

use reticle_geometry::Point;

/// A completed distance measurement between two world points.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Measurement {
    /// The first (anchor) point, in DBU.
    pub start: Point,
    /// The second (release) point, in DBU.
    pub end: Point,
    /// Database units per micron, used to convert the distance to a physical span.
    dbu_per_micron: i64,
}

impl Measurement {
    /// Builds a measurement between `start` and `end`.
    ///
    /// `dbu_per_micron` comes from the document technology; a non-positive value is
    /// treated as `1` so the micron conversion never divides by zero.
    #[must_use]
    pub fn new(start: Point, end: Point, dbu_per_micron: i64) -> Self {
        Self {
            start,
            end,
            dbu_per_micron: dbu_per_micron.max(1),
        }
    }

    /// The signed horizontal span, `end.x - start.x`, in DBU.
    #[must_use]
    pub fn dx(&self) -> i64 {
        i64::from(self.end.x) - i64::from(self.start.x)
    }

    /// The signed vertical span, `end.y - start.y`, in DBU.
    #[must_use]
    pub fn dy(&self) -> i64 {
        i64::from(self.end.y) - i64::from(self.start.y)
    }

    /// The Euclidean distance between the two points, in DBU.
    #[must_use]
    pub fn distance_dbu(&self) -> f64 {
        let d2 = self.start.distance_squared(self.end);
        (d2 as f64).sqrt()
    }

    /// The Euclidean distance between the two points, in microns.
    #[must_use]
    pub fn distance_microns(&self) -> f64 {
        self.distance_dbu() / self.dbu_per_micron as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_aligned_distance() {
        let m = Measurement::new(Point::new(0, 0), Point::new(4000, 0), 1000);
        assert!((m.distance_dbu() - 4000.0).abs() < 1e-6);
        assert!((m.distance_microns() - 4.0).abs() < 1e-9);
        assert_eq!(m.dx(), 4000);
        assert_eq!(m.dy(), 0);
    }

    #[test]
    fn pythagorean_triple() {
        let m = Measurement::new(Point::new(1000, 1000), Point::new(4000, 5000), 1000);
        assert!((m.distance_dbu() - 5000.0).abs() < 1e-6);
        assert_eq!(m.dx(), 3000);
        assert_eq!(m.dy(), 4000);
    }

    #[test]
    fn negative_deltas() {
        let m = Measurement::new(Point::new(5000, 5000), Point::new(2000, 1000), 1000);
        assert_eq!(m.dx(), -3000);
        assert_eq!(m.dy(), -4000);
        assert!((m.distance_dbu() - 5000.0).abs() < 1e-6);
    }

    #[test]
    fn micron_conversion_respects_resolution() {
        let m = Measurement::new(Point::new(0, 0), Point::new(2000, 0), 2000);
        // 2000 DBU at 2000 DBU/micron is 1 micron.
        assert!((m.distance_microns() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn nonpositive_resolution_does_not_divide_by_zero() {
        let m = Measurement::new(Point::new(0, 0), Point::new(1000, 0), 0);
        assert!(m.distance_microns().is_finite());
    }
}

//! Point, rectangle, orientation, and affine transform primitives on the integer
//! database-unit grid.

use crate::Dbu;

/// A point on the integer database-unit (DBU) grid.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Point {
    /// X coordinate in DBU.
    pub x: Dbu,
    /// Y coordinate in DBU.
    pub y: Dbu,
}

impl Point {
    /// The origin, `(0, 0)`.
    pub const ORIGIN: Self = Self { x: 0, y: 0 };

    /// Creates a point from its coordinates.
    #[must_use]
    pub const fn new(x: Dbu, y: Dbu) -> Self {
        Self { x, y }
    }

    /// Translates the point by `(dx, dy)`, saturating on overflow.
    #[must_use]
    pub fn translate(self, dx: Dbu, dy: Dbu) -> Self {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
        }
    }

    /// Squared Euclidean distance to `other`, in DBU² (widened to [`i64`]).
    #[must_use]
    pub fn distance_squared(self, other: Self) -> i64 {
        let dx = i64::from(self.x) - i64::from(other.x);
        let dy = i64::from(self.y) - i64::from(other.y);
        dx * dx + dy * dy
    }
}

/// An axis-aligned rectangle, stored as inclusive-exclusive `[min, max)` corners.
///
/// Invariant: `min.x <= max.x` and `min.y <= max.y`. Construct via [`Rect::new`]
/// (which normalizes) or [`Rect::from_points`].
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Rect {
    /// Lower-left corner (minimum x and y).
    pub min: Point,
    /// Upper-right corner (maximum x and y).
    pub max: Point,
}

impl Rect {
    /// Creates a rectangle from two opposite corners, normalizing so `min <= max`.
    #[must_use]
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    /// Creates a rectangle spanning all `points`, or `None` if the iterator is empty.
    #[must_use]
    pub fn from_points(points: impl IntoIterator<Item = Point>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut r = Self {
            min: first,
            max: first,
        };
        for p in it {
            r.min.x = r.min.x.min(p.x);
            r.min.y = r.min.y.min(p.y);
            r.max.x = r.max.x.max(p.x);
            r.max.y = r.max.y.max(p.y);
        }
        Some(r)
    }

    /// Width in DBU, widened to [`i64`].
    #[must_use]
    pub fn width(&self) -> i64 {
        i64::from(self.max.x) - i64::from(self.min.x)
    }

    /// Height in DBU, widened to [`i64`].
    #[must_use]
    pub fn height(&self) -> i64 {
        i64::from(self.max.y) - i64::from(self.min.y)
    }

    /// Area in DBU², widened to [`i64`].
    #[must_use]
    pub fn area(&self) -> i64 {
        self.width() * self.height()
    }

    /// Returns `true` if the rectangle has zero width or height.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    /// Returns `true` if `p` lies within `[min, max)`.
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// Returns `true` if the two rectangles overlap in a region of positive area.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }

    /// The smallest rectangle containing both `self` and `other`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    /// The overlapping rectangle, or `None` if the two do not intersect.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if !self.intersects(other) {
            return None;
        }
        Some(Self {
            min: Point::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Point::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        })
    }

    /// Expands the rectangle by `margin` DBU on every side (saturating).
    #[must_use]
    pub fn expanded(&self, margin: Dbu) -> Self {
        Self {
            min: self.min.translate(-margin, -margin),
            max: self.max.translate(margin, margin),
        }
    }
}

/// One of the eight orientations of the dihedral group D4: a rotation by a
/// multiple of 90°, optionally preceded by a reflection about the x-axis.
///
/// This matches the GDSII/OASIS placement model (reflect-then-rotate).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Orientation {
    /// No rotation, no reflection.
    #[default]
    R0,
    /// Rotate 90° counter-clockwise.
    R90,
    /// Rotate 180°.
    R180,
    /// Rotate 270° counter-clockwise.
    R270,
    /// Reflect about the x-axis.
    MirrorX,
    /// Reflect about the x-axis, then rotate 90°.
    MirrorX90,
    /// Reflect about the x-axis, then rotate 180° (equivalently, reflect about y).
    MirrorX180,
    /// Reflect about the x-axis, then rotate 270°.
    MirrorX270,
}

impl Orientation {
    /// Returns `true` if this orientation includes a reflection (flips handedness).
    #[must_use]
    pub fn is_mirrored(self) -> bool {
        matches!(
            self,
            Self::MirrorX | Self::MirrorX90 | Self::MirrorX180 | Self::MirrorX270
        )
    }

    /// Applies the orientation to a point about the origin.
    #[must_use]
    pub fn apply(self, p: Point) -> Point {
        // Reflect about the x-axis first (y -> -y) for the mirrored variants,
        // then rotate counter-clockwise by the associated angle.
        let (x, y) = (p.x, if self.is_mirrored() { -p.y } else { p.y });
        let (rx, ry) = match self {
            Self::R0 | Self::MirrorX => (x, y),
            Self::R90 | Self::MirrorX90 => (-y, x),
            Self::R180 | Self::MirrorX180 => (-x, -y),
            Self::R270 | Self::MirrorX270 => (y, -x),
        };
        Point::new(rx, ry)
    }

    /// The eight orientations, in the order matching [`Orientation::code`].
    pub const ALL: [Self; 8] = [
        Self::R0,
        Self::R90,
        Self::R180,
        Self::R270,
        Self::MirrorX,
        Self::MirrorX90,
        Self::MirrorX180,
        Self::MirrorX270,
    ];

    /// A stable `0..8` index for this orientation, for compact GPU encoding.
    ///
    /// The renderer packs this code into a per-instance transform and reconstructs
    /// the 2x2 linear map in the vertex shader, so the numbering here is a contract
    /// with `shapes.wgsl`.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            Self::R0 => 0,
            Self::R90 => 1,
            Self::R180 => 2,
            Self::R270 => 3,
            Self::MirrorX => 4,
            Self::MirrorX90 => 5,
            Self::MirrorX180 => 6,
            Self::MirrorX270 => 7,
        }
    }

    /// The orientation for a `0..8` [`Orientation::code`], wrapping out-of-range
    /// values modulo 8 so the mapping is total.
    #[must_use]
    pub fn from_code(code: u32) -> Self {
        Self::ALL[(code % 8) as usize]
    }

    /// Composes two orientations: `self.then(next)` is the orientation that applies
    /// `self` first and then `next`, so
    /// `self.then(next).apply(p) == next.apply(self.apply(p))` for every point.
    ///
    /// D4 is a group under composition, so the result is always one of the eight
    /// orientations. This is how instance orientations fold together as the renderer
    /// flattens a placement hierarchy into a single per-instance transform.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        // Recover the composed linear map from its action on the basis vectors, then
        // match it back to one of the eight orientations. Exact integer arithmetic,
        // no lookup table to get out of sync.
        let e_x = next.apply(self.apply(Point::new(1, 0)));
        let e_y = next.apply(self.apply(Point::new(0, 1)));
        Self::ALL
            .into_iter()
            .find(|o| o.apply(Point::new(1, 0)) == e_x && o.apply(Point::new(0, 1)) == e_y)
            .unwrap_or(Self::R0)
    }
}

/// A rational magnification factor, `num / den`. Defaults to unity.
///
/// Most layouts place instances at unit magnification; non-unit magnification is
/// represented exactly as a rational and applied with widened arithmetic and
/// round-to-nearest so integer coordinates stay on-grid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Magnification {
    num: u32,
    den: u32,
}

impl Default for Magnification {
    fn default() -> Self {
        Self::UNITY
    }
}

impl Magnification {
    /// Unit magnification (`1 / 1`).
    pub const UNITY: Self = Self { num: 1, den: 1 };

    /// Creates a magnification `num / den`, or `None` if `den == 0`.
    #[must_use]
    pub fn new(num: u32, den: u32) -> Option<Self> {
        if den == 0 {
            None
        } else {
            Some(Self { num, den })
        }
    }

    /// Returns `true` if this is exactly unit magnification.
    #[must_use]
    pub fn is_unity(self) -> bool {
        self.num == self.den
    }

    /// The numerator of the reduced `num / den` ratio.
    #[must_use]
    pub fn numerator(self) -> u32 {
        self.num
    }

    /// The denominator of the reduced `num / den` ratio.
    #[must_use]
    pub fn denominator(self) -> u32 {
        self.den
    }

    /// This magnification as a floating-point factor, for GPU upload.
    #[must_use]
    pub fn factor(self) -> f32 {
        self.num as f32 / self.den as f32
    }

    /// Composes two magnifications by multiplying their factors, saturating each
    /// term at [`u32::MAX`]. Used when folding a placement hierarchy into one
    /// per-instance scale.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        let num = u64::from(self.num) * u64::from(next.num);
        let den = u64::from(self.den) * u64::from(next.den);
        let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        // den is a product of two non-zero u32s, so it is non-zero.
        Self {
            num: clamp(num),
            den: clamp(den).max(1),
        }
    }

    /// Scales a single coordinate by this magnification, rounding to nearest DBU.
    #[must_use]
    pub fn scale(self, v: Dbu) -> Dbu {
        if self.is_unity() {
            return v;
        }
        let num = i64::from(self.num);
        let den = i64::from(self.den);
        let scaled = i64::from(v) * num;
        // Round half away from zero.
        let rounded = if scaled >= 0 {
            (scaled + den / 2) / den
        } else {
            (scaled - den / 2) / den
        };
        rounded.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
    }
}

/// An affine placement transform: reflect/rotate ([`Orientation`]), scale
/// ([`Magnification`]), then translate. This is the transform applied to an
/// instanced cell's contents.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Transform {
    /// Translation applied last, in DBU.
    pub translation: Point,
    /// Orientation applied first (about the origin).
    pub orientation: Orientation,
    /// Magnification applied after orientation, before translation.
    pub magnification: Magnification,
}

impl Transform {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        translation: Point::ORIGIN,
        orientation: Orientation::R0,
        magnification: Magnification::UNITY,
    };

    /// A pure translation by `(dx, dy)`.
    #[must_use]
    pub fn translate(dx: Dbu, dy: Dbu) -> Self {
        Self {
            translation: Point::new(dx, dy),
            ..Self::IDENTITY
        }
    }

    /// Applies the transform to a point: orient, magnify, then translate.
    #[must_use]
    pub fn apply(&self, p: Point) -> Point {
        let oriented = self.orientation.apply(p);
        let scaled = Point::new(
            self.magnification.scale(oriented.x),
            self.magnification.scale(oriented.y),
        );
        scaled.translate(self.translation.x, self.translation.y)
    }

    /// Composes two transforms: `self.then(next)` applies `self` first and then
    /// `next`, so `self.then(next).apply(p) == next.apply(self.apply(p))` for every
    /// point (up to the round-to-nearest of non-unit magnification).
    ///
    /// The orient/scale/translate placement group is closed under composition, so
    /// the result is again a [`Transform`]. The renderer uses this to fold a chain
    /// of nested placements into a single per-instance transform, so it can cache
    /// each cell's tessellation once and expand it in the vertex shader.
    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            translation: next.apply(self.translation),
            orientation: self.orientation.then(next.orientation),
            magnification: self.magnification.then(next.magnification),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Magnification, Orientation, Point, Transform};

    /// Points probed when checking that composed maps agree pointwise.
    const PROBES: [Point; 5] = [
        Point::new(0, 0),
        Point::new(1, 0),
        Point::new(0, 1),
        Point::new(3, -7),
        Point::new(-11, 5),
    ];

    #[test]
    fn orientation_code_round_trips() {
        for o in Orientation::ALL {
            assert_eq!(Orientation::from_code(o.code()), o);
        }
        // Codes are the array positions, and from_code wraps modulo 8.
        assert_eq!(Orientation::from_code(8), Orientation::R0);
        assert_eq!(Orientation::from_code(9), Orientation::R90);
    }

    #[test]
    fn orientation_then_matches_sequential_application() {
        // self.then(next) applies self first, then next.
        for a in Orientation::ALL {
            for b in Orientation::ALL {
                let composed = a.then(b);
                for p in PROBES {
                    assert_eq!(
                        composed.apply(p),
                        b.apply(a.apply(p)),
                        "({a:?}).then({b:?}) disagreed at {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn orientation_r0_is_identity_under_then() {
        for o in Orientation::ALL {
            assert_eq!(Orientation::R0.then(o), o);
            assert_eq!(o.then(Orientation::R0), o);
        }
    }

    #[test]
    fn magnification_then_multiplies_factors() {
        let two = Magnification::new(2, 1).unwrap();
        let half = Magnification::new(1, 2).unwrap();
        assert!(two.then(half).is_unity());
        let three_halves = Magnification::new(3, 2).unwrap();
        let combined = two.then(three_halves);
        assert!((combined.factor() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn transform_then_matches_sequential_application() {
        let inner = Transform {
            translation: Point::new(5, -2),
            orientation: Orientation::R90,
            magnification: Magnification::UNITY,
        };
        let outer = Transform {
            translation: Point::new(-3, 7),
            orientation: Orientation::MirrorX180,
            magnification: Magnification::UNITY,
        };
        let composed = inner.then(&outer);
        for p in PROBES {
            assert_eq!(
                composed.apply(p),
                outer.apply(inner.apply(p)),
                "composed transform disagreed at {p:?}"
            );
        }
    }

    #[test]
    fn transform_then_identity_is_noop() {
        let t = Transform {
            translation: Point::new(9, 4),
            orientation: Orientation::R270,
            magnification: Magnification::UNITY,
        };
        assert_eq!(t.then(&Transform::IDENTITY), t);
        assert_eq!(Transform::IDENTITY.then(&t), t);
    }
}

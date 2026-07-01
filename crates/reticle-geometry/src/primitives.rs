//! Point, rectangle, orientation, and affine transform primitives on the integer
//! database-unit grid.

use crate::Dbu;

/// A point on the integer database-unit (DBU) grid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
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
}

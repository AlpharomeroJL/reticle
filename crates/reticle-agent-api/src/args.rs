//! Serde-friendly argument types for the command surface.
//!
//! `reticle-geometry` does not derive serde, so the wire contract owns its own
//! plain-data input types and converts to engine types when a command is applied
//! (in a later wave). Coordinates are database units (`i32`); layer and datatype
//! are GDSII numbers (`u16`).

use serde::{Deserialize, Serialize};

/// A point in database units.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PointArg {
    /// X coordinate in database units.
    pub x: i32,
    /// Y coordinate in database units.
    pub y: i32,
}

/// An axis-aligned rectangle in database units, `min` corner to `max` corner.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RectArg {
    /// The minimum (lower-left) corner.
    pub min: PointArg,
    /// The maximum (upper-right) corner.
    pub max: PointArg,
}

/// A GDSII layer and datatype pair.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct LayerArg {
    /// GDSII layer number.
    pub layer: u16,
    /// GDSII datatype number.
    pub datatype: u16,
}

/// A planar boolean operation over two polygon sets.
///
/// The serde mirror of [`reticle_geometry::BooleanOp`], carried on the wire so the
/// command surface need not import the geometry enum. The `op` field of a
/// [`BooleanCombine`](crate::AgentCommand::BooleanCombine) command selects one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOpArg {
    /// The union `A ∪ B` (merge the shapes into one region).
    Union,
    /// The intersection `A ∩ B` (keep only the common region).
    Intersection,
    /// The difference `A \ B` (subtract the later shapes from the first).
    Difference,
    /// The symmetric difference `A △ B` (region covered an odd number of times).
    Xor,
}

/// How to align a set of shapes within their combined bounding box.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignArg {
    /// Line up left edges to the leftmost edge.
    Left,
    /// Line up right edges to the rightmost edge.
    Right,
    /// Center horizontally on the selection's center.
    CenterX,
    /// Line up top edges to the topmost edge.
    Top,
    /// Line up bottom edges to the bottommost edge.
    Bottom,
    /// Center vertically on the selection's center.
    CenterY,
}

/// The axis along which shapes are distributed to equalize their gaps.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisArg {
    /// Equalize horizontal (left-to-right) gaps.
    Horizontal,
    /// Equalize vertical (top-to-bottom) gaps.
    Vertical,
}

/// A path end-cap style.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndcapArg {
    /// No extension past the end vertex.
    Flat,
    /// Extend by half the path width (a square cap).
    Square,
    /// A rounded cap (approximated).
    Round,
}

/// One of the eight D4 orientations: a rotation, optionally mirrored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientationArg {
    /// No rotation.
    R0,
    /// 90 degrees counter-clockwise.
    R90,
    /// 180 degrees.
    R180,
    /// 270 degrees counter-clockwise.
    R270,
    /// Mirrored across the x axis.
    MirrorX,
    /// Mirrored across x, then rotated 90 degrees.
    MirrorX90,
    /// Mirrored across x, then rotated 180 degrees.
    MirrorX180,
    /// Mirrored across x, then rotated 270 degrees.
    MirrorX270,
}

/// A placement transform: orientation, an integer magnification ratio, and a
/// translation in database units.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TransformArg {
    /// The D4 orientation.
    pub orientation: OrientationArg,
    /// Magnification numerator (`mag_num / mag_den`).
    pub mag_num: i64,
    /// Magnification denominator; must be non-zero.
    pub mag_den: i64,
    /// Translation along x, in database units.
    pub dx: i32,
    /// Translation along y, in database units.
    pub dy: i32,
}

impl Default for TransformArg {
    fn default() -> Self {
        Self {
            orientation: OrientationArg::R0,
            mag_num: 1,
            mag_den: 1,
            dx: 0,
            dy: 0,
        }
    }
}

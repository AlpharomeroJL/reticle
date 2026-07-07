//! Mapping DEF/LEF placement orientations to the Reticle [`Orientation`].
//!
//! DEF names the eight placements `N`, `S`, `E`, `W` (pure rotations) and `FN`,
//! `FS`, `FE`, `FW` (their flipped forms). The flip in DEF is a mirror about the
//! **Y axis** applied *before* the rotation: `F<dir>` is "mirror about Y, then
//! rotate as `<dir>`". Reticle's [`Orientation`] instead models "reflect about the
//! **X axis**, then rotate counter-clockwise", the GDSII `strans` convention.
//!
//! The two conventions describe the same eight elements of the dihedral group D4;
//! the table below is the exact correspondence, verified in the unit test by
//! comparing the point transforms directly:
//!
//! | DEF  | transform     | Reticle       |
//! |------|---------------|---------------|
//! | `N`  | `( x,  y)`    | `R0`          |
//! | `W`  | `(-y,  x)`    | `R90`         |
//! | `S`  | `(-x, -y)`    | `R180`        |
//! | `E`  | `( y, -x)`    | `R270`        |
//! | `FN` | `(-x,  y)`    | `MirrorX180`  |
//! | `FS` | `( x, -y)`    | `MirrorX`     |
//! | `FW` | `(-y, -x)`    | `MirrorX270`  |
//! | `FE` | `( y,  x)`    | `MirrorX90`   |

use reticle_geometry::Orientation;

/// Maps a DEF orientation token (case-insensitive; an optional `R`/`FS`-style form
/// as written in DEF) to a Reticle [`Orientation`]. An unrecognized token maps to
/// [`Orientation::R0`] so a malformed placement still lands somewhere sensible.
///
/// `N` and the unknown fallback share a body (`R0`); the explicit `N` arm is kept
/// for the reader, so `match_same_arms` is allowed here.
#[must_use]
#[allow(clippy::match_same_arms)]
pub(crate) fn from_def(token: &str) -> Orientation {
    match token.to_ascii_uppercase().as_str() {
        "N" => Orientation::R0,
        "W" => Orientation::R90,
        "S" => Orientation::R180,
        "E" => Orientation::R270,
        "FN" => Orientation::MirrorX180,
        "FS" => Orientation::MirrorX,
        "FW" => Orientation::MirrorX270,
        "FE" => Orientation::MirrorX90,
        _ => Orientation::R0,
    }
}

#[cfg(test)]
mod tests {
    use super::from_def;
    use reticle_geometry::{Orientation, Point};

    /// The DEF point transform for each orientation, from the reference: `F<dir>`
    /// is "mirror about the Y axis (x -> -x), then rotate as `<dir>`".
    #[allow(clippy::match_same_arms)]
    fn def_transform(token: &str, p: Point) -> Point {
        let (x, y) = (p.x, p.y);
        match token {
            "N" => Point::new(x, y),
            "W" => Point::new(-y, x),
            "S" => Point::new(-x, -y),
            "E" => Point::new(y, -x),
            "FN" => Point::new(-x, y),
            "FS" => Point::new(x, -y),
            "FW" => Point::new(-y, -x),
            "FE" => Point::new(y, x),
            _ => Point::new(x, y),
        }
    }

    const PROBES: [Point; 5] = [
        Point::new(1, 0),
        Point::new(0, 1),
        Point::new(3, -7),
        Point::new(-11, 5),
        Point::new(13, 29),
    ];

    #[test]
    fn every_def_orientation_matches_reticle_transform() {
        for token in ["N", "W", "S", "E", "FN", "FS", "FW", "FE"] {
            let orient = from_def(token);
            for p in PROBES {
                assert_eq!(
                    orient.apply(p),
                    def_transform(token, p),
                    "DEF {token} -> {orient:?} disagreed at {p:?}"
                );
            }
        }
    }

    #[test]
    fn lowercase_and_unknown_tokens() {
        assert_eq!(from_def("fn"), Orientation::MirrorX180);
        assert_eq!(from_def("n"), Orientation::R0);
        assert_eq!(from_def("bogus"), Orientation::R0);
    }
}

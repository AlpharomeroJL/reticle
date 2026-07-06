//! The "New Tiny Tapeout tile" template: a correctly framed SKY130 GDS-mode tile.
//!
//! Tiny Tapeout's GDS-mode (analog / custom-layout) path asks a submitter to hand
//! it a finished tile GDS whose die area, analog pin positions, and power straps
//! all match a fixed template. This module builds that frame as a Reticle
//! [`Document`] so "New Tiny Tapeout tile" on the Start screen drops the user into a
//! correctly shaped, pinned tile with nothing to guess. The user then fills the
//! interior; the frame is the part they must not move.
//!
//! # What the frame is, and where every number comes from
//!
//! The geometry is transcribed from Tiny Tapeout's own template files, fetched
//! 2026-07-06 for the `TTSKY26c` shuttle (closing 2026-09-07). Nothing here is
//! invented; the sources are:
//!
//! * The **1x2 tile** die area and the six `ua[0]`..`ua[5]` analog pins on met4,
//!   from the DEF template `tt_analog_1x2.def`:
//!   <https://raw.githubusercontent.com/TinyTapeout/tt-support-tools/main/tech/sky130A/def/analog/tt_analog_1x2.def>.
//!   `DIEAREA ( 0 0 ) ( 161000 225760 )`; each `ua[n]` is a met4 PORT
//!   `( -450 -500 ) ( 450 500 )` `PLACED ( x 500 )`, so its absolute rectangle is
//!   `(x-450, 0)`..`(x+450, 1000)`. The DEF carries eight `ua` pins physically, but
//!   the analog spec caps *usable* analog pins at six, used in order from 0, so the
//!   template exposes `ua[0]`..`ua[5]`.
//! * The **power straps** (VDPWR, VGND, and the optional 3.3 V VAPWR) as vertical
//!   met4 stripes, from the official init script `magic_init_project.tcl`:
//!   <https://raw.githubusercontent.com/TinyTapeout/tt-support-tools/main/tech/sky130A/def/analog/magic_init_project.tcl>.
//!   Each stripe is `box $x 5um $x 220.76um` painted `met4` at `box width 2um`
//!   (minimum 1.2 um), so it spans y `5000`..`220760` and is 2000 nm wide; the net
//!   x positions are `VDPWR` at 1 um, `VGND` at 4 um, `VAPWR` at 7 um.
//!
//! The technology is [`crate::tinytapeout::technology`], parsed from the committed
//! `tech/tinytapeout-sky130.tech`, which names met4 (`71/20`), its pin/label
//! purposes, the `tt_boundary` marker (SKY130 areaid.sc `81/4`), and, so the rule
//! is on record, that met5 (`72/20`) is off limits.
//!
//! # No "locked" flag: the frame is documented, not enforced
//!
//! The Reticle model has no per-shape lock. These frame shapes are the fixed part
//! of the template the user must not move, but the model cannot forbid moving them,
//! so this is stated here and in the technology file rather than faked. The
//! validation tests instead assert the frame's *own* correctness: the six pins are
//! on met4, named in order, and inside the tile; the straps span the required
//! bottom-10 to top-10 window on met4; and nothing lands on met5.
//!
//! # Portability
//!
//! Like [`crate::usecases`], the technology is compiled in with [`include_str!`]
//! and the document is built in pure code, so this works identically on native and
//! on `wasm32` (the Start screen runs in the browser, where there is no
//! filesystem).

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{
    Anchor, Cell, Document, DrawShape, Label, Pin, PinDirection, ShapeKind, Technology,
};

/// The committed Tiny Tapeout technology file, compiled in so the frame's layers
/// are named and colored with no runtime path, exactly as [`crate::usecases`]
/// embeds `tech/sky130.tech`.
const TT_TECH: &str = include_str!("../../../tech/tinytapeout-sky130.tech");

/// The template's top-cell name. Tiny Tapeout requires the top macro name to start
/// with `tt_um_` and be unique on the shuttle; the user renames it to their own
/// `tt_um_*` before submitting.
pub const TT_TILE_TOP: &str = "tt_um_reticle_tile";

/// SKY130 met4 drawing layer (`71/20`): the layer every Tiny Tapeout analog pin and
/// power strap lives on.
const MET4: LayerId = LayerId::new(71, 20);
/// SKY130 met4 pin purpose (`71/16`): the terminal region of each pin.
const MET4_PIN: LayerId = LayerId::new(71, 16);
/// SKY130 met4 label purpose (`71/5`): where the pin/strap names are drawn.
const MET4_LABEL: LayerId = LayerId::new(71, 5);
/// The tile-boundary marker layer (SKY130 areaid.sc `81/4`).
const TT_BOUNDARY: LayerId = LayerId::new(81, 4);

/// The 1x2 tile die area, in DBU (1 dbu = 1 nm): `(0,0)`..`(161000, 225760)`, i.e.
/// 161.0 x 225.76 um. From `tt_analog_1x2.def` `DIEAREA`. DBU is `i32`, and every
/// coordinate in this frame fits comfortably.
const DIE_MAX_X: i32 = 161_000;
const DIE_MAX_Y: i32 = 225_760;

/// Half-width of a `ua[*]` pin port in x (the DEF PORT is `( -450 ... 450 )`).
const UA_HALF_W: i32 = 450;
/// Bottom of a `ua[*]` pin port, in y: `PLACED` y (500) minus the PORT half-height
/// (500), which is the tile's bottom edge (0). Its top is `500 + 500 = 1000`.
const UA_BOTTOM_Y: i32 = 0;
const UA_TOP_Y: i32 = 1_000;

/// The x centers of `ua[0]`..`ua[5]`, verbatim from the `PLACED` coordinates in
/// `tt_analog_1x2.def`. The pitch is a constant 19320 nm; the template stops at
/// `ua[5]` because the analog spec allows at most six analog pins, used in order.
const UA_CENTERS_X: [i32; 6] = [152_260, 132_940, 113_620, 94_300, 74_980, 55_660];

/// A power strap's vertical extent, in DBU: y `5000`..`220760`. From the init
/// script's `box $x 5um $x 220.76um`. This is within 5 um of the bottom and
/// (`225760 - 220760 = 5000`) within 5 um of the top, satisfying the "bottom 10 um
/// to top 10 um" rule with margin.
const STRAP_BOTTOM_Y: i32 = 5_000;
const STRAP_TOP_Y: i32 = 220_760;
/// Half of a strap's 2000 nm width (`box width 2um`; the minimum is 1.2 um).
const STRAP_HALF_W: i32 = 1_000;

/// One power strap: its net name, x center, and direction sense (ground vs power).
struct Strap {
    /// The power-net name (`VGND`, `VDPWR`, or `VAPWR`).
    name: &'static str,
    /// The strap centerline x, in DBU. From the init script's per-net position.
    center_x: i32,
    /// Whether this is the ground net (affects only the label/pin semantics).
    ground: bool,
}

/// The three power straps a 3.3 V-capable tile carries, at the init script's
/// positions: `VDPWR` at 1 um, `VGND` at 4 um, `VAPWR` at 7 um. A 1.8 V-only tile
/// simply leaves `VAPWR` unconnected; including it keeps the frame ready for either
/// and matches the widest template.
const STRAPS: [Strap; 3] = [
    Strap {
        name: "VDPWR",
        center_x: 1_000,
        ground: false,
    },
    Strap {
        name: "VGND",
        center_x: 4_000,
        ground: true,
    },
    Strap {
        name: "VAPWR",
        center_x: 7_000,
        ground: false,
    },
];

/// The Tiny Tapeout technology (met4 with its pin/label purposes, the tile-boundary
/// marker, and the met5 keep-out on record), parsed from the committed
/// `tech/tinytapeout-sky130.tech`.
///
/// # Panics
///
/// Panics only if the compiled-in `tech/tinytapeout-sky130.tech` fails to parse,
/// which can happen solely if the committed file is malformed; a unit test guards
/// against that so no caller can observe it.
#[must_use]
pub fn technology() -> Technology {
    reticle_io::parse_technology(TT_TECH).expect("bundled tech/tinytapeout-sky130.tech must parse")
}

/// The absolute met4 rectangle of the `n`-th analog pin (`ua[n]`), for `n` in
/// `0..6`. `(center_x - 450, 0)`..`(center_x + 450, 1000)`.
#[must_use]
fn ua_rect(n: usize) -> Rect {
    let cx = UA_CENTERS_X[n];
    Rect::new(
        Point::new(cx - UA_HALF_W, UA_BOTTOM_Y),
        Point::new(cx + UA_HALF_W, UA_TOP_Y),
    )
}

/// The absolute met4 rectangle of a power strap centered at `center_x`.
/// `(center_x - 1000, 5000)`..`(center_x + 1000, 220760)`.
#[must_use]
fn strap_rect(center_x: i32) -> Rect {
    Rect::new(
        Point::new(center_x - STRAP_HALF_W, STRAP_BOTTOM_Y),
        Point::new(center_x + STRAP_HALF_W, STRAP_TOP_Y),
    )
}

/// Builds the Tiny Tapeout tile template document: the `tt_um_reticle_tile` cell
/// with the tile boundary, the six `ua[0]`..`ua[5]` analog pins on met4, and the
/// VDPWR/VGND/VAPWR power straps on met4, carrying the Tiny Tapeout technology.
///
/// The cell holds, all in the tile's own coordinate system (origin at the
/// bottom-left of the die):
///
/// * the tile outline as a rectangle on `tt_boundary` (SKY130 areaid.sc);
/// * each `ua[n]` as a met4 drawing rectangle plus a [`Pin`] on the met4 pin
///   purpose (region equal to that rectangle) plus a center [`Label`] on the met4
///   label purpose, so the pin round-trips through GDSII and reads in the editor;
/// * each power strap as a met4 drawing rectangle plus a [`Pin`] and a bottom-anchored
///   [`Label`] at the strap's base.
///
/// Nothing is drawn on met5. The frame shapes are the fixed template the user must
/// not move (the model has no lock; see the module docs).
#[must_use]
pub fn tile_document() -> Document {
    let mut cell = Cell::new(TT_TILE_TOP);

    // The tile outline: a boundary-layer rectangle over the whole die area.
    cell.shapes.push(DrawShape::new(
        TT_BOUNDARY,
        ShapeKind::Rect(Rect::new(
            Point::new(0, 0),
            Point::new(DIE_MAX_X, DIE_MAX_Y),
        )),
    ));

    // The six analog pins, in order from ua[0]. Each gets drawing metal, a pin
    // terminal on the pin purpose, and a centered label.
    for n in 0..UA_CENTERS_X.len() {
        let name = format!("ua[{n}]");
        let region = ua_rect(n);
        cell.shapes
            .push(DrawShape::new(MET4, ShapeKind::Rect(region)));
        cell.pins.push(Pin {
            name: name.clone(),
            region,
            layer: MET4_PIN,
            direction: PinDirection::Inout,
        });
        // The label sits at the pin's center (the port rectangle's midpoint).
        let center = Point::new(
            i32::midpoint(region.min.x, region.max.x),
            i32::midpoint(region.min.y, region.max.y),
        );
        cell.labels.push(Label::new(name, center, MET4_LABEL));
    }

    // The three power straps. Each gets drawing metal, a pin terminal spanning the
    // strap, and a label at the strap's base so it does not collide with the pins.
    for strap in &STRAPS {
        let region = strap_rect(strap.center_x);
        cell.shapes
            .push(DrawShape::new(MET4, ShapeKind::Rect(region)));
        cell.pins.push(Pin {
            name: strap.name.to_owned(),
            region,
            layer: MET4_PIN,
            direction: PinDirection::Inout,
        });
        cell.labels.push(Label {
            text: strap.name.to_owned(),
            position: Point::new(strap.center_x, STRAP_BOTTOM_Y),
            layer: MET4_LABEL,
            anchor: Anchor::SouthWest,
        });
        // `ground` distinguishes VGND from the power nets for downstream intent;
        // it does not change geometry, but keeping it read here documents the
        // difference the init script encodes via `port use ground|power`.
        let _ = strap.ground;
    }

    let mut doc = Document::new();
    doc.set_technology(technology());
    doc.insert_cell(cell);
    doc.set_top_cells(vec![TT_TILE_TOP.to_owned()]);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Shape;

    /// SKY130 met5 drawing layer (`72/20`): FORBIDDEN in a Tiny Tapeout tile. Only
    /// the tests need it, to assert the template draws nothing on it.
    const MET5: LayerId = LayerId::new(72, 20);
    /// The Tiny Tapeout minimum power-strap width, 1.2 um in DBU. The straps here are
    /// wider (2.0 um); the frame must clear this floor.
    const MIN_STRAP_WIDTH_DBU: i64 = 1_200;
    /// The "within 10 um of the edge" window the straps must reach at top and
    /// bottom, in DBU. Compared against `i32` y coordinates.
    const EDGE_WINDOW_DBU: i32 = 10_000;
    /// The number of analog pins the analog spec allows, used in order from 0.
    const MAX_ANALOG_PINS: usize = 6;

    #[test]
    fn technology_parses_and_names_met4_and_boundary() {
        let tech = technology();
        assert_eq!(tech.name, "tinytapeout_sky130");
        assert_eq!(tech.dbu_per_micron, 1000);
        let met4 = tech
            .layers
            .iter()
            .find(|l| l.id == MET4)
            .expect("met4 in the layer table");
        assert_eq!(met4.name, "met4");
        assert!(
            tech.layers.iter().any(|l| l.id == TT_BOUNDARY),
            "the tile-boundary marker layer is present"
        );
    }

    #[test]
    fn document_frames_the_tile_top_cell() {
        let doc = tile_document();
        assert_eq!(doc.top_cells(), &[TT_TILE_TOP.to_owned()]);
        // The top-macro name starts with the required tt_um_ prefix.
        assert!(TT_TILE_TOP.starts_with("tt_um_"));
        let cell = doc.cell(TT_TILE_TOP).expect("tile top cell");
        assert!(!cell.shapes.is_empty(), "the tile has frame geometry");
    }

    #[test]
    fn die_area_is_the_1x2_template() {
        let doc = tile_document();
        // The boundary rectangle is exactly the DEF DIEAREA.
        let cell = doc.cell(TT_TILE_TOP).unwrap();
        let boundary = cell
            .shapes
            .iter()
            .find(|s| s.layer == TT_BOUNDARY)
            .expect("a boundary shape");
        let bb = boundary.bounding_box();
        assert_eq!(bb.min, Point::new(0, 0));
        assert_eq!(bb.max, Point::new(DIE_MAX_X, DIE_MAX_Y));
    }

    #[test]
    fn exactly_six_analog_pins_named_in_order_on_met4_inside_the_tile() {
        let doc = tile_document();
        let cell = doc.cell(TT_TILE_TOP).unwrap();
        let ua: Vec<&Pin> = cell
            .pins
            .iter()
            .filter(|p| p.name.starts_with("ua["))
            .collect();
        assert_eq!(ua.len(), MAX_ANALOG_PINS, "six analog pins, no more");
        let die = Rect::new(Point::new(0, 0), Point::new(DIE_MAX_X, DIE_MAX_Y));
        for (n, expected_cx) in UA_CENTERS_X.iter().enumerate() {
            let pin = cell
                .pins
                .iter()
                .find(|p| p.name == format!("ua[{n}]"))
                .unwrap_or_else(|| panic!("ua[{n}] present"));
            // On the met4 pin purpose.
            assert_eq!(pin.layer, MET4_PIN, "ua[{n}] on met4 pin purpose");
            // At the DEF-specified rectangle.
            assert_eq!(pin.region, ua_rect(n), "ua[{n}] rectangle matches the DEF");
            // Centered where the DEF PLACED it.
            assert_eq!(
                i32::midpoint(pin.region.min.x, pin.region.max.x),
                *expected_cx
            );
            // Wholly inside the die area.
            assert_eq!(die.intersection(&pin.region), Some(pin.region));
        }
    }

    #[test]
    fn power_straps_are_met4_and_span_bottom10_to_top10() {
        let doc = tile_document();
        let cell = doc.cell(TT_TILE_TOP).unwrap();
        for name in ["VGND", "VDPWR", "VAPWR"] {
            let pin = cell
                .pins
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("{name} strap present"));
            assert_eq!(pin.layer, MET4_PIN, "{name} strap on met4");
            let r = pin.region;
            // At least 1.2 um wide.
            assert!(
                r.width() >= MIN_STRAP_WIDTH_DBU,
                "{name} is {} wide, under the 1.2um floor",
                r.width()
            );
            // Reaches within 10 um of the bottom and of the top of the tile.
            assert!(
                r.min.y <= EDGE_WINDOW_DBU,
                "{name} bottom {} is not within 10um of the die bottom",
                r.min.y
            );
            assert!(
                r.max.y >= DIE_MAX_Y - EDGE_WINDOW_DBU,
                "{name} top {} is not within 10um of the die top",
                r.max.y
            );
        }
    }

    #[test]
    fn nothing_is_drawn_on_the_forbidden_met5_layer() {
        let doc = tile_document();
        let cell = doc.cell(TT_TILE_TOP).unwrap();
        assert!(
            !cell.shapes.iter().any(|s| s.layer == MET5),
            "met5 is forbidden in a Tiny Tapeout tile"
        );
        assert!(!cell.pins.iter().any(|p| p.layer == MET5));
        assert!(!cell.labels.iter().any(|l| l.layer == MET5));
    }

    #[test]
    fn every_pin_and_strap_carries_a_label() {
        let doc = tile_document();
        let cell = doc.cell(TT_TILE_TOP).unwrap();
        // Six analog pins plus three straps, each with a matching label.
        for name in (0..MAX_ANALOG_PINS)
            .map(|n| format!("ua[{n}]"))
            .chain(["VGND", "VDPWR", "VAPWR"].map(str::to_owned))
        {
            assert!(
                cell.labels.iter().any(|l| l.text == name),
                "{name} has a label"
            );
            assert!(
                cell.labels.iter().all(|l| l.layer == MET4_LABEL),
                "labels are on the met4 label purpose"
            );
        }
    }
}

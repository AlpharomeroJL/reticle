//! Shared fixtures for the device tests: SKY130 layer constants, small
//! shape/label builders, and the hand-built minimal inverter used as the golden
//! layout. Kept under `tests/common/` so Cargo treats it as a helper module, not
//! a test binary of its own.

#![allow(dead_code)] // each test binary uses a different subset of these helpers.

use reticle_extract::NetLabel;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};

// SKY130 GDS (layer, datatype) pairs used by the fixtures.
pub const DIFF: LayerId = LayerId::new(65, 20);
pub const TAP: LayerId = LayerId::new(65, 44);
pub const POLY: LayerId = LayerId::new(66, 20);
pub const LICON1: LayerId = LayerId::new(66, 44);
pub const LI1: LayerId = LayerId::new(67, 20);
pub const NWELL: LayerId = LayerId::new(64, 20);
pub const NSDM: LayerId = LayerId::new(93, 44);
pub const PSDM: LayerId = LayerId::new(94, 20);

/// A rectangle draw-shape on `layer` spanning `[(x0,y0), (x1,y1)]`.
pub fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// A net-naming label at `(x, y)` on `layer`.
pub fn label(name: &str, layer: LayerId, x: i32, y: i32) -> NetLabel {
    NetLabel::new(name, Point::new(x, y), layer)
}

/// Builds a single-cell document named `top` from a shape list.
pub fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

/// The shapes and labels of a hand-built minimal SKY130 inverter.
///
/// A shared vertical poly gate (input `A`) crosses an n+ diffusion (NMOS, bottom)
/// and a p+ diffusion in an n-well (PMOS, top). `li1` rails tie: NMOS source and
/// its p-tap to `VGND`, PMOS source and its n-tap to `VPWR`, and both drains to
/// the output `Y`. This is the golden fixture: the expected device list (1 NMOS +
/// 1 PMOS with the terminal nets asserted in the tests) is hand-verified from this
/// geometry. See tests/fixtures/inverter.md for the annotated coordinate table.
pub fn inverter() -> (Document, Vec<NetLabel>) {
    let shapes = vec![
        // --- NMOS (bottom, n+ in substrate) ---
        rect(DIFF, 0, 0, 100, 40),     // 0: NMOS active
        rect(NSDM, -5, -5, 105, 45),   // 1: n+ select over NMOS active
        rect(TAP, 0, -40, 100, -15),   // 2: p-tap (VGND body tie)
        rect(PSDM, -5, -45, 105, -10), // 3: p+ select over the p-tap
        // --- PMOS (top, p+ in n-well) ---
        rect(NWELL, -10, 90, 110, 210), // 4: n-well
        rect(DIFF, 0, 100, 100, 140),   // 5: PMOS active
        rect(PSDM, -5, 95, 105, 145),   // 6: p+ select over PMOS active
        rect(TAP, 0, 165, 100, 190),    // 7: n-tap (VPWR body tie)
        rect(NSDM, -5, 160, 105, 195),  // 8: n+ select over the n-tap
        // --- Shared poly gate (input A) ---
        rect(POLY, 40, -10, 60, 150), // 9: vertical gate stripe crossing both diffs
        // --- VGND rail: NMOS source lobe + p-tap ---
        rect(LICON1, 12, 12, 28, 28),   // 10: contact on NMOS source
        rect(LICON1, 12, -32, 28, -20), // 11: contact on p-tap
        rect(LI1, 8, -35, 32, 32),      // 12: li1 VGND rail
        // --- Y rail: NMOS drain lobe + PMOS drain lobe ---
        rect(LICON1, 72, 12, 88, 28),   // 13: contact on NMOS drain
        rect(LICON1, 72, 112, 88, 128), // 14: contact on PMOS drain
        rect(LI1, 68, 8, 92, 132),      // 15: li1 Y output strap
        // --- VPWR rail: PMOS source lobe + n-tap ---
        rect(LICON1, 12, 112, 28, 128), // 16: contact on PMOS source
        rect(LICON1, 12, 172, 28, 185), // 17: contact on n-tap
        rect(LI1, 8, 108, 32, 188),     // 18: li1 VPWR rail
    ];
    let labels = vec![
        label("A", POLY, 50, 70),
        label("VGND", LI1, 20, 20),
        label("Y", LI1, 80, 70),
        label("VPWR", LI1, 20, 120),
    ];
    (doc_with(shapes), labels)
}

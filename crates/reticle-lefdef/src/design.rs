//! The imported-run contract: [`LefDefDesign`] and its companion types.
//!
//! This module defines the value that LEF/DEF import produces and that lane 5B (the
//! run viewer) consumes. Everything here is plain owned data with public fields and
//! `serde`-free value semantics, so the viewer can hold it, diff it, and overlay it
//! without calling back into the parser.
//!
//! # Why the layout and the run metadata are separate
//!
//! [`LefDefDesign::document`] is the lowered [`Document`]: cells, instances, and
//! routed shapes on mapped layers, everything the renderer already knows how to
//! draw. The remaining fields are *run-level metadata* the viewer overlays on top
//! of that geometry: the die outline, the placement rows and sites, the net list
//! with per-net routed segments, and the external pins. They are kept beside the
//! document rather than encoded into it because a viewer treats them differently
//! from drawn geometry: rows and the die area are chrome, nets are selectable and
//! highlightable as logical objects, and the [`ReportOverlays`] slots are filled in
//! later from report files rather than from the layout itself.
//!
//! # Coordinates
//!
//! Every coordinate in this module is an integer database unit (DBU) on the grid
//! set by [`Document::technology`]'s `dbu_per_micron`, matching the rest of the
//! Reticle model. DEF `UNITS DISTANCE MICRONS` establishes the resolution; LEF
//! micron dimensions are converted to DBU on the same grid during lowering.

use reticle_geometry::{Dbu, LayerId, Orientation, Point, Rect};
use reticle_model::{Document, PinDirection};

/// The result of importing a LEF/DEF pair or a run directory.
///
/// Produced by [`import_lef_def`](crate::import_lef_def) and
/// [`import_run_dir`](crate::import_run_dir). The lowered layout lives in
/// [`document`](LefDefDesign::document); the rest is run-level metadata a viewer
/// overlays. This is the frozen contract lane 5B builds against, so its shape is
/// deliberately flat and owned.
#[derive(Clone, Debug, Default)]
pub struct LefDefDesign {
    /// The lowered layout: one [`Cell`](reticle_model::Cell) per LEF `MACRO`, plus a
    /// top cell named [`design_name`](LefDefDesign::design_name) holding the placed
    /// component [`Instance`](reticle_model::Instance)s and the routed net shapes.
    /// The document's [`Technology`](reticle_model::Technology) carries the layer
    /// table derived from the LEF layers and the DEF database resolution.
    pub document: Document,
    /// The design name from the DEF `DESIGN` statement, and the name of the top
    /// cell in [`document`](LefDefDesign::document). Empty when no DEF was imported
    /// (LEF-only import).
    pub design_name: String,
    /// The die outline from DEF `DIEAREA`, in DBU. `None` when the DEF declared none.
    pub die_area: Option<Rect>,
    /// Placement site definitions from LEF `SITE` blocks, keyed by name from
    /// [`Site::name`]. Rows reference these by name.
    pub sites: Vec<Site>,
    /// Placement rows from DEF `ROW` statements, in declaration order.
    pub rows: Vec<Row>,
    /// The routed net list from DEF `NETS`, in declaration order. Each net keeps its
    /// routed [`segments`](Net::segments); the same geometry is also lowered into
    /// the top cell so it renders, but the net list is what a viewer highlights and
    /// selects by name.
    pub nets: Vec<Net>,
    /// External design pins (I/O ports) from DEF `PINS`, in declaration order.
    pub pins: Vec<DesignPin>,
    /// Slots for report-derived overlays (congestion, utilization, timing). Empty
    /// after a plain LEF/DEF import; [`import_run_dir`](crate::import_run_dir) and
    /// lane 5B fill these from the run's report files.
    pub overlays: ReportOverlays,
    /// Non-fatal problems encountered during import (skipped keywords, dropped
    /// degenerate shapes, unresolved references). Empty on a clean import.
    pub warnings: Vec<crate::LefDefWarning>,
}

impl LefDefDesign {
    /// The top cell name, i.e. [`design_name`](LefDefDesign::design_name).
    #[must_use]
    pub fn top_cell(&self) -> &str {
        &self.design_name
    }
}

/// A placement site: the unit cell of the placement grid, from LEF `SITE`.
///
/// A standard-cell row is tiled with copies of a site; [`Row`] references one by
/// [`name`](Site::name). Dimensions are in DBU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The site name (referenced by [`Row::site`]).
    pub name: String,
    /// The `CLASS` of the site (typically `CORE` or `PAD`), verbatim, or empty if
    /// the LEF declared none.
    pub class: String,
    /// Site width in DBU.
    pub width: Dbu,
    /// Site height in DBU (the row height).
    pub height: Dbu,
}

/// A placement row from DEF `ROW`: a repeated site along one axis.
///
/// A DEF row places `count_x * count_y` sites starting at [`origin`](Row::origin),
/// stepping by [`step_x`](Row::step_x)/[`step_y`](Row::step_y) DBU. One of the two
/// counts is normally 1 (a row runs along a single axis).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The row name (`ROW <name> ...`).
    pub name: String,
    /// The site tiled along this row, by [`Site::name`]. May name a site absent from
    /// [`LefDefDesign::sites`] if the LEF did not define it.
    pub site: String,
    /// The row origin in DBU.
    pub origin: Point,
    /// The site orientation applied to every tile in the row.
    pub orientation: Orientation,
    /// Number of sites along x (`DO <count_x>`).
    pub count_x: u32,
    /// Number of sites along y (`BY <count_y>`).
    pub count_y: u32,
    /// The x step between sites in DBU (`STEP <step_x>`).
    pub step_x: Dbu,
    /// The y step between sites in DBU (`<step_y>`).
    pub step_y: Dbu,
}

/// A routed net from DEF `NETS`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Net {
    /// The net name.
    pub name: String,
    /// The DEF `+ USE` classification (`SIGNAL`, `POWER`, `GROUND`, `CLOCK`, ...),
    /// verbatim, or `None` if the net declared none.
    pub use_kind: Option<String>,
    /// The routed geometry: wire segments and vias in declaration order. Empty for
    /// an unrouted net (connectivity only).
    pub segments: Vec<NetSegment>,
}

/// One piece of a net's routing: a wire run or a via drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetSegment {
    /// A routed wire: a polyline on one layer with a width in DBU. A point run of
    /// length 1 is a point contact (a via landing recorded as a wire is normalized
    /// to a [`Via`](NetSegment::Via) instead).
    Wire {
        /// The routing layer.
        layer: LayerId,
        /// The center-line points of the wire, in DBU.
        points: Vec<Point>,
        /// The wire width in DBU (the layer default when the DEF gave none).
        width: Dbu,
    },
    /// A via drop at a point: a named via master placed on the route.
    Via {
        /// The location of the via, in DBU.
        at: Point,
        /// The via master name from the DEF (for example `via1`), verbatim.
        via: String,
    },
}

/// An external design pin (an I/O port) from DEF `PINS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignPin {
    /// The pin (port) name.
    pub name: String,
    /// The signal direction, mapped from DEF `+ DIRECTION`.
    pub direction: PinDirection,
    /// The net this pin connects to (`+ NET`), or empty if unspecified.
    pub net: String,
    /// The pin's shape layer, when the DEF placed a `+ LAYER ... ( ) ( )` rectangle.
    pub layer: Option<LayerId>,
    /// The pin's placed region in DBU, when the DEF placed the pin.
    pub region: Option<Rect>,
}

/// Report-derived overlays a viewer draws over the layout.
///
/// These fields are owned by the design so lane 5B has a stable place to attach
/// report data, but LEF/DEF alone do not populate them: they stay at their defaults
/// after [`import_lef_def`](crate::import_lef_def). The report parsing that fills
/// them (reading `OpenROAD` congestion maps, utilization summaries, and timing
/// reports) lands in lane 5B, which owns these slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportOverlays {
    /// Core utilization as a fraction in `[0, 1]`, from a placement report. `None`
    /// until a report supplies it.
    pub utilization: Option<f64>,
    /// Per-region routing congestion cells, from a global-route congestion map.
    /// Empty until a report supplies them.
    pub congestion: Vec<CongestionCell>,
    /// Timing-critical nets ordered worst-slack first, from a timing report. Empty
    /// until a report supplies them.
    pub timing_critical_nets: Vec<CriticalNet>,
}

/// One routing-congestion cell of the global-route grid (a lane 5B overlay slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CongestionCell {
    /// The grid-cell region in DBU.
    pub region: Rect,
    /// Demand minus capacity for the cell; positive means overflow (congested).
    pub overflow: i32,
}

/// One timing-critical net with its worst slack (a lane 5B overlay slot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CriticalNet {
    /// The net name (matches a [`Net::name`] in [`LefDefDesign::nets`]).
    pub net: String,
    /// Worst-path slack in picoseconds; negative is a timing violation.
    pub slack_ps: i64,
}

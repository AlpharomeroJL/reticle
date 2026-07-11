//! The F3 trace-query contract: the serializable result records the trace UI consumes.
//!
//! The trace-api lane (Phase 2) adds read-only spatial queries over an already-extracted
//! [`Netlist`]: net-at-point, net-extent, and a shorts/opens report. Those
//! queries are cached per document revision, so every result record carries a `revision`
//! envelope: it is the cache key, and a UI knows a result is stale when the document's
//! revision has moved past it. The records here are the query RESULTS (what the UI renders),
//! distinct from the internal [`Net`](crate::Net) / [`Short`](crate::Short) /
//! [`Open`](crate::Open) types the extractor works with.
//!
//! Coordinates are `i64` DBU in a standalone [`RectRecord`] because
//! `reticle_geometry::Rect` is not serde-serializable; the records are byte-stable so a
//! cached result hashes and diffs exactly. Fixture-first: the trace-ui lane builds against
//! the committed canned responses (`tests/fixtures/contracts/f3_trace.json`) before the
//! query API exists.

use reticle_geometry::{Point, Rect, Shape as _};
use reticle_model::DrawShape;
use serde::{Deserialize, Serialize};

use crate::connectivity::shape_covers_point;
use crate::intent::IntentReport;
use crate::netlist::Netlist;

/// A rectangle in DBU. Standalone (not `reticle_geometry::Rect`) so a query result
/// serializes; byte-stable.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RectRecord {
    /// Minimum x in DBU.
    pub min_x: i64,
    /// Minimum y in DBU.
    pub min_y: i64,
    /// Maximum x in DBU.
    pub max_x: i64,
    /// Maximum y in DBU.
    pub max_y: i64,
}

/// A net reference: the net name and the indices of the shapes on it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetRef {
    /// The net name (a label or the synthesized `net_<n>`).
    pub name: String,
    /// The indices (into the queried document's flattened shapes) that belong to the net.
    pub shape_indices: Vec<usize>,
}

/// The result of a net-at-point query at a document revision.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetAtPoint {
    /// The document revision this result was computed against (the cache key).
    pub revision: u64,
    /// The net covering the queried point, or `None` if the point is on no net.
    pub net: Option<NetRef>,
}

/// The result of a net-extent query: a net's bounding box and shape count.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetExtent {
    /// The document revision this result was computed against (the cache key).
    pub revision: u64,
    /// The net the extent is for.
    pub net: String,
    /// The net's bounding box in DBU.
    pub bbox: RectRecord,
    /// How many shapes the net spans.
    pub shape_count: usize,
}

/// A short: two distinct nets that are electrically connected but should not be.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ShortRecord {
    /// One shorted net.
    pub net_a: String,
    /// The other shorted net.
    pub net_b: String,
    /// A location where the two nets touch, for the navigable list to zoom to.
    pub at: RectRecord,
}

/// An open: a net that should be a single piece but is split into `pieces` pieces.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OpenRecord {
    /// The net that is broken.
    pub net: String,
    /// How many disconnected pieces the net is in (at least 2 for an open).
    pub pieces: usize,
}

/// A shorts/opens report at a document revision: the trace UI's navigable list.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct ShortsOpensReport {
    /// The document revision this report was computed against (the cache key).
    pub revision: u64,
    /// The shorts found.
    pub shorts: Vec<ShortRecord>,
    /// The opens found.
    pub opens: Vec<OpenRecord>,
}

impl ShortsOpensReport {
    /// Whether the report is clean: no shorts and no opens.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.shorts.is_empty() && self.opens.is_empty()
    }

    /// The total number of flagged items (shorts plus opens).
    #[must_use]
    pub fn len(&self) -> usize {
        self.shorts.len() + self.opens.len()
    }

    /// Whether the report has no flagged items (the same condition as [`is_clean`](Self::is_clean)).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.is_clean()
    }
}

/// Converts a geometry [`Rect`] (DBU as `i32`) to the serializable [`RectRecord`]
/// (DBU widened to `i64`, matching the widening [`Rect::width`] and friends use).
fn rect_record(r: Rect) -> RectRecord {
    RectRecord {
        min_x: i64::from(r.min.x),
        min_y: i64::from(r.min.y),
        max_x: i64::from(r.max.x),
        max_y: i64::from(r.max.y),
    }
}

/// Finds the net covering `point`, if any.
///
/// Scans `shapes` in order for the first one whose footprint covers `point`
/// ([`shape_covers_point`]), then resolves its net via [`Netlist::net_of`]. A
/// point that covers no shape yields `net: None`. `shapes` and `netlist` must be
/// the pair an [`Extractor`](crate::Extractor) produced together (the netlist's
/// member indices index into `shapes`); the result carries `revision`, the
/// document revision that pair was extracted at.
#[must_use]
pub fn net_at_point(
    shapes: &[DrawShape],
    netlist: &Netlist,
    point: Point,
    revision: u64,
) -> NetAtPoint {
    let net = shapes
        .iter()
        .position(|shape| shape_covers_point(shape, point))
        .and_then(|index| netlist.net_of(index))
        .map(|net| NetRef {
            name: net.name.clone(),
            shape_indices: net.shapes.clone(),
        });
    NetAtPoint { revision, net }
}

/// Computes the bounding box and shape count of the net named `name`.
///
/// Looks up the net's member shape indices with [`Netlist::shapes_of`] and unions
/// their bounding boxes. Returns `None` if `netlist` has no net named `name`
/// (mirroring `shapes_of`'s `None`); a member index outside `shapes` is skipped
/// rather than panicking (defends against a `netlist`/`shapes` pair that did not
/// come from the same extraction). The result carries `revision`, the document
/// revision `netlist` was extracted at.
#[must_use]
pub fn net_extent(
    shapes: &[DrawShape],
    netlist: &Netlist,
    name: &str,
    revision: u64,
) -> Option<NetExtent> {
    let indices = netlist.shapes_of(name)?;
    let mut bbox: Option<Rect> = None;
    for &i in indices {
        let Some(shape) = shapes.get(i) else {
            continue;
        };
        let b = shape.bounding_box();
        bbox = Some(match bbox {
            Some(acc) => acc.union(&b),
            None => b,
        });
    }
    let bbox = bbox.unwrap_or(Rect {
        min: Point::ORIGIN,
        max: Point::ORIGIN,
    });
    Some(NetExtent {
        revision,
        net: name.to_owned(),
        bbox: rect_record(bbox),
        shape_count: indices.len(),
    })
}

/// The minimum piece count an [`Open`](crate::Open) implies.
///
/// [`check_intent`](crate::check_intent) stops at the first terminal that
/// disagrees with the net's other terminals (see its `net_open` helper), so an
/// `Open` records only that a net is split, not how many pieces it is split into.
/// [`shorts_opens`] reports this documented floor rather than a fabricated exact
/// count; computing an exact count needs `intent::Open` to carry one, which is
/// outside this lane's owned paths (`query.rs` only).
const OPEN_PIECES_FLOOR: usize = 2;

/// Maps an [`IntentReport`]'s shorts and opens into the serializable
/// [`ShortsOpensReport`] the trace UI's navigable list renders.
///
/// Each [`Short`](crate::Short) becomes a [`ShortRecord`] (the two net names plus
/// the touching location); each [`Open`](crate::Open) becomes an [`OpenRecord`]
/// (the net name plus a piece count; see `OPEN_PIECES_FLOOR`'s doc comment in this
/// module for why the count is a floor rather than a measurement). The result
/// carries `revision`, shared with the other F3 records computed at the same
/// document revision.
#[must_use]
pub fn shorts_opens(report: &IntentReport, revision: u64) -> ShortsOpensReport {
    ShortsOpensReport {
        revision,
        shorts: report
            .shorts
            .iter()
            .map(|short| ShortRecord {
                net_a: short.net_a.clone(),
                net_b: short.net_b.clone(),
                at: rect_record(short.at),
            })
            .collect(),
        opens: report
            .opens
            .iter()
            .map(|open| OpenRecord {
                net: open.net.clone(),
                pieces: OPEN_PIECES_FLOOR,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extractor;
    use crate::intent::{Open, Short};
    use reticle_geometry::LayerId;
    use reticle_model::ShapeKind;

    /// Two overlapping rects (one net, shapes 0 and 1) plus a far-away isolated
    /// rect (its own net, shape 2). No labels, so names are the synthesized
    /// `net_0` / `net_1`, assigned in first-seen order (root of shape 0 first).
    fn two_net_shapes() -> Vec<DrawShape> {
        let m = LayerId::new(1, 0);
        let rect = |x0, y0, x1, y1| {
            DrawShape::new(
                m,
                ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
            )
        };
        vec![
            rect(0, 0, 10, 10),
            rect(5, 5, 15, 15),
            rect(1000, 1000, 1010, 1010),
        ]
    }

    #[test]
    fn net_at_point_finds_the_covering_net() {
        let shapes = two_net_shapes();
        let netlist = Extractor::new().extract_shapes(&shapes);

        let result = net_at_point(&shapes, &netlist, Point::new(7, 7), 7);
        assert_eq!(result.revision, 7);
        let net = result.net.expect("point (7,7) is covered by shape 0");
        assert_eq!(net.name, "net_0");
        assert_eq!(net.shape_indices, vec![0, 1]);
    }

    #[test]
    fn net_at_point_off_net_is_none() {
        let shapes = two_net_shapes();
        let netlist = Extractor::new().extract_shapes(&shapes);

        let result = net_at_point(&shapes, &netlist, Point::new(500, 500), 7);
        assert_eq!(result.revision, 7);
        assert!(result.net.is_none(), "point covers no shape");
    }

    #[test]
    fn net_extent_reports_bbox_and_shape_count() {
        let shapes = two_net_shapes();
        let netlist = Extractor::new().extract_shapes(&shapes);

        let extent = net_extent(&shapes, &netlist, "net_0", 7).expect("net_0 exists");
        assert_eq!(extent.revision, 7);
        assert_eq!(extent.net, "net_0");
        assert_eq!(extent.shape_count, 2);
        // Union of rect(0,0,10,10) and rect(5,5,15,15).
        assert_eq!(
            extent.bbox,
            RectRecord {
                min_x: 0,
                min_y: 0,
                max_x: 15,
                max_y: 15,
            }
        );

        let single = net_extent(&shapes, &netlist, "net_1", 7).expect("net_1 exists");
        assert_eq!(single.shape_count, 1);
        assert_eq!(
            single.bbox,
            RectRecord {
                min_x: 1000,
                min_y: 1000,
                max_x: 1010,
                max_y: 1010,
            }
        );
    }

    #[test]
    fn net_extent_unknown_name_is_none() {
        let shapes = two_net_shapes();
        let netlist = Extractor::new().extract_shapes(&shapes);
        assert!(net_extent(&shapes, &netlist, "no_such_net", 7).is_none());
    }

    #[test]
    fn shorts_opens_maps_a_seeded_short_and_open() {
        let report = IntentReport {
            shorts: vec![Short {
                net_a: "VDD".to_owned(),
                net_b: "GND".to_owned(),
                at: Rect::new(Point::new(4000, 3000), Point::new(4200, 3200)),
            }],
            opens: vec![Open {
                net: "CLK".to_owned(),
                at: Rect::new(Point::new(100, 100), Point::new(200, 200)),
                detail: "terminal 'clk_in' is on a different component from the rest".to_owned(),
            }],
        };

        let out = shorts_opens(&report, 7);
        assert_eq!(out.revision, 7);
        assert_eq!(out.shorts.len(), 1);
        assert_eq!(out.shorts[0].net_a, "VDD");
        assert_eq!(out.shorts[0].net_b, "GND");
        assert_eq!(
            out.shorts[0].at,
            RectRecord {
                min_x: 4000,
                min_y: 3000,
                max_x: 4200,
                max_y: 3200,
            }
        );
        assert_eq!(out.opens.len(), 1);
        assert_eq!(out.opens[0].net, "CLK");
        assert_eq!(out.opens[0].pieces, 2);
        assert!(!out.is_clean());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn shorts_opens_empty_report_is_clean() {
        let out = shorts_opens(&IntentReport::default(), 42);
        assert_eq!(out.revision, 42);
        assert!(out.is_clean());
    }
}

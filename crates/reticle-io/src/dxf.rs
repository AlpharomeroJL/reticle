//! DXF (Drawing Exchange Format) reader: the layout-relevant 2D subset.
//!
//! # Scope
//!
//! DXF is a tagged ASCII format: alternating lines of a group code (an integer)
//! and its value. A file is a sequence of sections (`HEADER`, `CLASSES`,
//! `TABLES`, `BLOCKS`, `ENTITIES`, `OBJECTS`), each opened by `0 SECTION` / `2
//! <name>` and closed by `0 ENDSEC`. This reader only interprets the
//! `ENTITIES` section; every other section is skipped structurally without
//! being parsed, and `BLOCKS`/`INSERT` (block references) are out of scope, so
//! only entities drawn directly in `ENTITIES` are imported.
//!
//! Within `ENTITIES`, the following entity types are read:
//!
//! * `LINE`: a two-point [`ShapeKind::Path`] (zero width).
//! * `LWPOLYLINE`: a lightweight polyline. Closed (bit 0 of group 70) becomes a
//!   [`ShapeKind::Polygon`]; open becomes a [`ShapeKind::Path`].
//! * `POLYLINE` / `VERTEX` / `SEQEND`: the classic (pre-`LWPOLYLINE`) polyline,
//!   spanning multiple entity records. Converted the same way as `LWPOLYLINE`.
//! * `CIRCLE`: polygonized into a closed [`ShapeKind::Polygon`] ring.
//! * `ARC`: polygonized into an open [`ShapeKind::Path`] (zero width).
//! * `HATCH`: only the common **polyline boundary path** case (group 92 bit
//!   `0x2` set) is read, each such loop becoming its own [`ShapeKind::Polygon`].
//!
//! Every other entity type (`TEXT`, `INSERT`, `SPLINE`, `ELLIPSE`, `DIMENSION`,
//! `3DFACE`, ...) is recognized structurally (so it cannot desync the parser)
//! but skipped with a deduped [`WarningKind::UnsupportedFeature`] warning.
//!
//! # Honest gaps
//!
//! * **Bulge is ignored.** `LWPOLYLINE`/`POLYLINE`/`VERTEX` and `HATCH`
//!   polyline-boundary vertices may carry a group-42 bulge factor that turns a
//!   segment into an arc; this reader always draws a straight chord between
//!   consecutive vertices. A bulged polyline therefore imports as its
//!   straight-edged silhouette, not its true rounded outline.
//! * **`HATCH` edge-type boundaries are not read.** A boundary path whose
//!   group-92 flag does not have bit `0x2` set is built from `LINE`/`ARC`/
//!   `ELLIPSE`/`SPLINE` edge records instead of a flat vertex list; this reader
//!   does not parse edge geometry and skips that boundary loop with a
//!   [`WarningKind::UnsupportedFeature`] warning rather than guessing at it.
//! * **No 3D.** Every Z coordinate (group 30/31/etc.) and extrusion direction
//!   (group 210/220/230) is ignored; geometry is read as if drawn in the world
//!   XY plane. This matches the brief: a DXF **2D** reader.
//! * **No units.** DXF carries no intrinsic database-units-per-micron scale (a
//!   real drawing's unit is declared in the `HEADER` section's `$INSUNITS`
//!   variable, which this reader does not parse). Coordinates are taken as-is:
//!   one DXF drawing unit equals one [`Dbu`], rounded to the nearest integer.
//!   [`Technology::dbu_per_micron`] is set to a conventional default
//!   ([`DEFAULT_DBU_PER_MICRON`]) purely for downstream consistency; no
//!   conversion is performed.
//!
//! # Layers
//!
//! DXF layer names (group code 8, defaulting to `"0"` when absent) are not
//! resolved against the `TABLES` section's `LAYER` table (not parsed; a
//! `LAYER` table entry only carries display metadata this reader does not
//! model). Instead, each distinct name seen on an entity is assigned a fresh
//! [`LayerId::new(k, 0)`](LayerId::new) in first-seen order and recorded in
//! [`Technology::layers`], so the caller gets the layer mapping **as data** (a
//! list of DXF layer names in encounter order) rather than a hard-coded
//! assignment. This mirrors [`crate::cif`]'s layer handling exactly.
//!
//! # Polygonization tolerance
//!
//! `CIRCLE` and `ARC` entities are polygonized to a bounded chord: the segment
//! count is chosen so the sagitta (the maximum perpendicular gap between a
//! chord and the true arc) never exceeds [`ARC_SAGITTA_TOLERANCE_DBU`], with a
//! floor of [`MIN_ARC_SEGMENTS`] (so a tiny circle still reads as round) and a
//! hard ceiling of [`MAX_ARC_SEGMENTS`] (so a huge radius cannot produce an
//! unbounded vertex list).
//!
//! # Untrusted-input discipline
//!
//! Every count-driven allocation is capped against the remaining input, never
//! against a claimed count (a `LWPOLYLINE`'s group-90 vertex count, a
//! `HATCH`'s group-93 vertex count, and so on, are all ignored as anything
//! other than a hint; a vertex list is actually grown one push at a time and
//! capped as it grows):
//!
//! * [`MAX_INPUT_BYTES`] (256 MiB) rejects an oversized input before any
//!   parsing begins, mirroring [`crate::gds::MAX_INPUT_BYTES`].
//! * [`MAX_PAIRS`] caps the number of group-code/value line pairs tokenized
//!   from the input, so a file that is small in bytes but pathologically
//!   line-dense cannot grow the pair table without bound.
//! * [`MAX_SHAPE_VERTICES`] (200,000) caps a single polyline, `HATCH`
//!   boundary loop, or classic `POLYLINE`'s point list; a shape that runs past
//!   it is dropped whole with a [`WarningKind::LimitExceeded`] warning rather
//!   than truncated-and-kept, mirroring [`crate::cif::MAX_SHAPE_VERTICES`].
//! * [`MAX_SHAPES`] caps the total number of shapes materialized into the
//!   document; further entities parse (so the file stays in sync) but are not
//!   inserted, with one deduped warning.
//! * [`MAX_LAYERS`] caps the number of distinct layer names tracked; further
//!   names fold onto a fixed overflow [`LayerId`], with one deduped warning.
//! * A layer name longer than [`MAX_TOKEN_LEN`] is truncated (at a valid UTF-8
//!   boundary) rather than kept unbounded, with one deduped warning.
//!
//! Malformed input (an odd number of lines, a non-numeric group code or
//! coordinate, a `POLYLINE` never closed by a `SEQEND`, a count past a cap) is
//! rejected with a structured [`IoError`], never a panic or an unbounded
//! allocation. A single degenerate entity (a zero/negative radius, too few
//! vertices) is recoverable: it is skipped with a warning and the rest of the
//! file still imports, matching [`crate::cif`]'s "one bad element does not
//! sink an otherwise-good file" philosophy.

use crate::IoError;
use crate::error::{ImportWarning, WarningKind};
use reticle_geometry::{Dbu, Endcap, LayerId, Path, Point, Polygon};
use reticle_model::{
    Cell, Document, DrawShape, Importer, LayerInfo, ModelError, Result, ShapeKind, Technology,
};
use std::collections::HashMap;

/// DXF import (2D `ENTITIES`-section subset; see the [module docs](self)).
///
/// Implements [`Importer`] (bytes to [`Document`]). There is no exporter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Dxf;

/// The largest DXF input this importer will attempt to parse, in bytes (256
/// MiB). A stream at or under this bound parses within a bounded allocation; a
/// larger one is refused with a clear [`IoError`] rather than risking an
/// out-of-memory abort on a hostile or truncated-huge input.
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

/// The largest number of group-code/value line pairs this importer will
/// tokenize from the input. DXF places no explicit limit on line count, so
/// this is the defense-in-depth ceiling against a file that is small in bytes
/// but pathologically line-dense (many short lines); past it the input is
/// refused outright, mirroring how [`MAX_INPUT_BYTES`] guards raw size.
pub const MAX_PAIRS: usize = 4_000_000;

/// The largest number of vertices a single polyline (`LWPOLYLINE`, classic
/// `POLYLINE`) or `HATCH` boundary loop is allowed to carry into the model.
/// Past it the whole shape is dropped with a [`WarningKind::LimitExceeded`]
/// warning rather than materialized, mirroring [`crate::gds::MAX_SHAPE_VERTICES`].
pub const MAX_SHAPE_VERTICES: usize = 200_000;

/// The largest number of shapes this importer will materialize into the
/// document. Past this cap, further entities are still parsed (so the rest of
/// the file stays in sync) but are not inserted, with one deduped
/// [`WarningKind::LimitExceeded`] warning. A real hand-drawn or CAM-exported
/// DXF layout is nowhere near this size.
pub const MAX_SHAPES: usize = 500_000;

/// The largest number of distinct layer (group code 8) names this importer
/// will track. Past this cap, further distinct names resolve to a single
/// fixed overflow [`LayerId`] rather than growing the name table without
/// bound, with one deduped [`WarningKind::LimitExceeded`] warning.
pub const MAX_LAYERS: usize = 4_096;

/// The longest a single layer name is kept before being truncated (at a valid
/// UTF-8 boundary), with one deduped [`WarningKind::LimitExceeded`] warning.
pub const MAX_TOKEN_LEN: usize = 4_096;

/// Maximum sagitta (the perpendicular gap between a chord and the true arc)
/// tolerated when polygonizing a `CIRCLE` or `ARC`, in [`Dbu`]. See the
/// [module docs](self) for how this drives the segment count.
pub const ARC_SAGITTA_TOLERANCE_DBU: f64 = 10.0;

/// Floor on the number of segments a full-circle polygonization uses,
/// regardless of tolerance, so a tiny-radius `CIRCLE` still reads as round
/// rather than degenerating toward a triangle.
pub const MIN_ARC_SEGMENTS: usize = 12;

/// Hard ceiling on the number of segments a single `CIRCLE`/`ARC`
/// polygonizes to, independent of the tolerance computation, so a huge radius
/// cannot produce an unbounded vertex list. Well under [`MAX_SHAPE_VERTICES`].
pub const MAX_ARC_SEGMENTS: usize = 2_048;

/// The database resolution recorded for every DXF import. DXF carries no
/// intrinsic scale (see the [module docs](self)); this is a conventional
/// default, not a conversion.
pub const DEFAULT_DBU_PER_MICRON: i64 = 1_000;

/// The fixed [`LayerId`] every layer name past [`MAX_LAYERS`] folds onto.
/// Chosen at the top of the `u16` range so it can never collide with a
/// sequentially assigned real layer (which stops at `MAX_LAYERS - 1 < 4096`).
const LAYER_OVERFLOW: LayerId = LayerId::new(0xFFFF, 0xFFFF);

/// The result of a DXF import that kept its non-fatal warnings.
///
/// Returned by [`Dxf::import_with_warnings`]. The [`document`](DxfImport::document)
/// is always well-formed and safe to use; [`warnings`](DxfImport::warnings) lists
/// every recoverable problem that was skipped, capped, or ignored during the
/// import (empty for a clean file). The frozen [`Importer::import`] path
/// discards the warnings and returns only the document.
#[derive(Debug, Clone)]
pub struct DxfImport {
    /// The imported document. Always valid, even when warnings are present.
    pub document: Document,
    /// Recoverable problems found during import, in encounter order.
    pub warnings: Vec<ImportWarning>,
}

impl Importer for Dxf {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        Ok(self.import_with_warnings(bytes)?.document)
    }
}

impl Dxf {
    /// Imports DXF `bytes` into a [`Document`], keeping every non-fatal warning.
    ///
    /// This is the hardened import entry point; see the [module docs](self) for
    /// the entity subset and untrusted-input discipline. It never panics and
    /// never allocates unboundedly on any input.
    ///
    /// # Errors
    ///
    /// Returns a [`reticle_model::ModelError`] (via [`IoError`]) when the input
    /// is too large, is not valid UTF-8, or is malformed DXF (an odd number of
    /// lines, a non-numeric group code or coordinate, a `POLYLINE` never closed
    /// by a `SEQEND`, or a structural count past a cap).
    pub fn import_with_warnings(&self, bytes: &[u8]) -> Result<DxfImport> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(malformed(
                "DXF input exceeds the maximum accepted size (256 MiB)",
            ));
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| malformed("DXF input is not valid UTF-8"))?;
        let pairs = tokenize(text)?;

        let mut state = ParseState::new();
        state.run(&pairs)?;
        let (document, warnings) = state.finalize();
        Ok(DxfImport { document, warnings })
    }
}

/// Tokenizes `text` into `(group code, value)` pairs: alternating lines, the
/// first of each pair an integer group code and the second its value, both
/// trimmed of surrounding whitespace.
///
/// # Errors
///
/// Returns a [`ModelError`] if a group code is not a valid integer, if the
/// input ends with a group code that has no matching value line, or if the
/// pair count would exceed [`MAX_PAIRS`].
fn tokenize(text: &str) -> Result<Vec<(i32, &str)>> {
    let mut lines = text.lines();
    let mut pairs = Vec::new();
    loop {
        let Some(code_line) = lines.next() else {
            break;
        };
        let Some(value_line) = lines.next() else {
            return Err(malformed(
                "DXF input ends with a group code that has no matching value line",
            ));
        };
        let code: i32 = code_line
            .trim()
            .parse()
            .map_err(|_| malformed("DXF group code is not a valid integer"))?;
        if pairs.len() >= MAX_PAIRS {
            return Err(malformed(
                "DXF input exceeds the maximum accepted number of group-code/value pairs",
            ));
        }
        pairs.push((code, value_line.trim()));
    }
    Ok(pairs)
}

/// An in-progress classic `POLYLINE`, accumulated across its `VERTEX` records
/// until a matching `SEQEND`.
struct PolylineAccum {
    layer: LayerId,
    closed: bool,
    points: Vec<Point>,
    truncated: bool,
}

/// The parser's running state across the whole file.
struct ParseState {
    layer_ids: HashMap<String, LayerId>,
    layers: Vec<LayerInfo>,
    shapes: Vec<DrawShape>,
    current_polyline: Option<PolylineAccum>,
    warnings: Warnings,
}

impl ParseState {
    fn new() -> Self {
        Self {
            layer_ids: HashMap::new(),
            layers: Vec::new(),
            shapes: Vec::new(),
            current_polyline: None,
            warnings: Warnings::new(),
        }
    }

    /// Walks every `(group code, value)` pair, tracking which section is
    /// active and dispatching entities found inside `ENTITIES`.
    ///
    /// Each `0 <name>` pair opens a record (a section marker, an entity, or
    /// `ENDSEC`/`EOF`) whose data is every following pair up to (not
    /// including) the next `0`-coded pair; finding that span structurally,
    /// before any attempt to interpret it, is what keeps one entity's parsing
    /// from ever desynchronizing the rest of the file.
    fn run(&mut self, pairs: &[(i32, &str)]) -> Result<()> {
        let mut in_entities = false;
        let mut i = 0usize;
        while i < pairs.len() {
            let (code, value) = pairs[i];
            if code != 0 {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < pairs.len() && pairs[end].0 != 0 {
                end += 1;
            }
            let data = &pairs[start..end];

            if self.current_polyline.is_some() && value != "VERTEX" && value != "SEQEND" {
                return Err(malformed(
                    "DXF POLYLINE is never closed by a matching SEQEND",
                ));
            }
            match value {
                "EOF" => break,
                "SECTION" => in_entities = find_str(data, 2) == Some("ENTITIES"),
                "ENDSEC" => in_entities = false,
                _ if in_entities => self.handle_entity(value, data)?,
                _ => {}
            }
            i = end;
        }
        if self.current_polyline.is_some() {
            return Err(malformed(
                "DXF input ends with an unterminated POLYLINE (missing SEQEND)",
            ));
        }
        Ok(())
    }

    fn handle_entity(&mut self, kind: &str, data: &[(i32, &str)]) -> Result<()> {
        match kind {
            "LINE" => self.add_line(data),
            "CIRCLE" => self.add_circle(data),
            "ARC" => self.add_arc(data),
            "LWPOLYLINE" => self.add_lwpolyline(data),
            "POLYLINE" => self.begin_polyline(data),
            "VERTEX" => self.add_vertex(data),
            "SEQEND" => {
                self.end_polyline();
                Ok(())
            }
            "HATCH" => self.add_hatch(data),
            _ => {
                self.warn(
                    WarningKind::UnsupportedFeature,
                    "DXF entity type not supported",
                    format!("entity type `{kind}` is not read by this importer; skipped"),
                );
                Ok(())
            }
        }
    }

    fn add_line(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let layer = self.layer_for(data);
        let x1 = require_f64(data, 10, "DXF LINE is missing its start X (group code 10)")?;
        let y1 = require_f64(data, 20, "DXF LINE is missing its start Y (group code 20)")?;
        let x2 = require_f64(data, 11, "DXF LINE is missing its end X (group code 11)")?;
        let y2 = require_f64(data, 21, "DXF LINE is missing its end Y (group code 21)")?;
        let points = vec![
            Point::new(to_dbu(x1), to_dbu(y1)),
            Point::new(to_dbu(x2), to_dbu(y2)),
        ];
        self.push_shape(DrawShape::new(
            layer,
            ShapeKind::Path(Path::new(points, 0, Endcap::Flat)),
        ));
        Ok(())
    }

    fn add_circle(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let layer = self.layer_for(data);
        let cx = require_f64(
            data,
            10,
            "DXF CIRCLE is missing its center X (group code 10)",
        )?;
        let cy = require_f64(
            data,
            20,
            "DXF CIRCLE is missing its center Y (group code 20)",
        )?;
        let r = require_f64(data, 40, "DXF CIRCLE is missing its radius (group code 40)")?;
        if r <= 0.0 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "circle skipped: non-positive radius",
                format!("a DXF CIRCLE had radius {r}; skipped"),
            );
            return Ok(());
        }
        let center = Point::new(to_dbu(cx), to_dbu(cy));
        let points = polygonize_arc(center, r, 0.0, 360.0, true);
        self.push_shape(DrawShape::new(
            layer,
            ShapeKind::Polygon(Polygon::new(points)),
        ));
        Ok(())
    }

    fn add_arc(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let layer = self.layer_for(data);
        let cx = require_f64(data, 10, "DXF ARC is missing its center X (group code 10)")?;
        let cy = require_f64(data, 20, "DXF ARC is missing its center Y (group code 20)")?;
        let r = require_f64(data, 40, "DXF ARC is missing its radius (group code 40)")?;
        let start = require_f64(
            data,
            50,
            "DXF ARC is missing its start angle (group code 50)",
        )?;
        let end = require_f64(data, 51, "DXF ARC is missing its end angle (group code 51)")?;
        if r <= 0.0 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "arc skipped: non-positive radius",
                format!("a DXF ARC had radius {r}; skipped"),
            );
            return Ok(());
        }
        // DXF arcs always sweep counter-clockwise from the start angle to the
        // end angle; a start/end pair that is numerically equal (mod 360) is
        // degenerate here (a true full circle is a CIRCLE entity, not an ARC).
        let sweep = (end - start).rem_euclid(360.0);
        if sweep <= 0.0 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "arc skipped: zero sweep angle",
                format!("a DXF ARC had start angle {start} equal to end angle {end}; skipped"),
            );
            return Ok(());
        }
        let center = Point::new(to_dbu(cx), to_dbu(cy));
        let points = polygonize_arc(center, r, start, sweep, false);
        self.push_shape(DrawShape::new(
            layer,
            ShapeKind::Path(Path::new(points, 0, Endcap::Flat)),
        ));
        Ok(())
    }

    fn add_lwpolyline(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let layer = self.layer_for(data);
        let closed = flag_bit0_set(data, 70)?;
        let (points, truncated) = collect_vertices(data)?;
        if truncated {
            self.warn(
                WarningKind::LimitExceeded,
                "LWPOLYLINE skipped: too many vertices",
                format!("a DXF LWPOLYLINE had more than {MAX_SHAPE_VERTICES} vertices; skipped"),
            );
            return Ok(());
        }
        self.finish_ring_or_path(layer, closed, points, "LWPOLYLINE");
        Ok(())
    }

    fn begin_polyline(&mut self, data: &[(i32, &str)]) -> Result<()> {
        if self.current_polyline.is_some() {
            return Err(malformed(
                "DXF POLYLINE begins while a previous POLYLINE is still open",
            ));
        }
        let layer = self.layer_for(data);
        let closed = flag_bit0_set(data, 70)?;
        self.current_polyline = Some(PolylineAccum {
            layer,
            closed,
            points: Vec::new(),
            truncated: false,
        });
        Ok(())
    }

    fn add_vertex(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let Some(accum) = &mut self.current_polyline else {
            self.warn(
                WarningKind::UnsupportedFeature,
                "DXF VERTEX with no open POLYLINE",
                "a VERTEX entity appeared outside any POLYLINE/SEQEND block; ignored",
            );
            return Ok(());
        };
        let x = require_f64(data, 10, "DXF VERTEX is missing its X (group code 10)")?;
        let y = require_f64(data, 20, "DXF VERTEX is missing its Y (group code 20)")?;
        if accum.points.len() < MAX_SHAPE_VERTICES {
            accum.points.push(Point::new(to_dbu(x), to_dbu(y)));
        } else {
            accum.truncated = true;
        }
        Ok(())
    }

    fn end_polyline(&mut self) {
        let Some(accum) = self.current_polyline.take() else {
            self.warn(
                WarningKind::UnsupportedFeature,
                "DXF SEQEND with no open POLYLINE",
                "a SEQEND entity appeared outside any POLYLINE/VERTEX block; ignored",
            );
            return;
        };
        if accum.truncated {
            self.warn(
                WarningKind::LimitExceeded,
                "POLYLINE skipped: too many vertices",
                format!("a DXF POLYLINE had more than {MAX_SHAPE_VERTICES} vertices; skipped"),
            );
            return;
        }
        self.finish_ring_or_path(accum.layer, accum.closed, accum.points, "POLYLINE");
    }

    /// Reads a `HATCH` entity's boundary paths, keeping only the common
    /// **polyline boundary path** loops (see the [module docs](self)).
    fn add_hatch(&mut self, data: &[(i32, &str)]) -> Result<()> {
        let layer = self.layer_for(data);
        let mut collecting = false;
        let mut expected = 0usize;
        let mut got: Vec<Point> = Vec::new();
        let mut pending_x: Option<Dbu> = None;
        let mut truncated = false;
        let mut edge_boundary_seen = false;

        for &(code, value) in data {
            match code {
                92 => {
                    if collecting {
                        self.finish_hatch_loop(layer, std::mem::take(&mut got), truncated);
                    }
                    truncated = false;
                    pending_x = None;
                    let flags = parse_i64(value)?;
                    collecting = flags & 2 != 0;
                    expected = 0;
                    if !collecting {
                        edge_boundary_seen = true;
                    }
                }
                93 if collecting => expected = usize_from_i64(parse_i64(value)?),
                10 if collecting => pending_x = Some(to_dbu(parse_f64(value)?)),
                20 if collecting => {
                    if let Some(x) = pending_x.take() {
                        let y = to_dbu(parse_f64(value)?);
                        if got.len() < MAX_SHAPE_VERTICES {
                            got.push(Point::new(x, y));
                        } else {
                            truncated = true;
                        }
                        if got.len() >= expected {
                            self.finish_hatch_loop(layer, std::mem::take(&mut got), truncated);
                            collecting = false;
                            truncated = false;
                        }
                    }
                }
                _ => {}
            }
        }
        if collecting {
            self.finish_hatch_loop(layer, got, truncated);
        }
        if edge_boundary_seen {
            self.warn(
                WarningKind::UnsupportedFeature,
                "HATCH edge-type boundary path skipped",
                "a HATCH boundary path was built from line/arc/ellipse/spline edges rather \
                 than a flat vertex list; this reader only extracts polyline boundary paths, \
                 so the loop was skipped"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn finish_hatch_loop(&mut self, layer: LayerId, points: Vec<Point>, truncated: bool) {
        if truncated {
            self.warn(
                WarningKind::LimitExceeded,
                "HATCH boundary loop skipped: too many vertices",
                format!(
                    "a DXF HATCH polyline boundary path had more than {MAX_SHAPE_VERTICES} \
                     vertices; skipped"
                ),
            );
            return;
        }
        if points.len() < 3 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "HATCH boundary loop skipped: fewer than 3 vertices",
                format!(
                    "a DXF HATCH polyline boundary path had {} vertices; skipped",
                    points.len()
                ),
            );
            return;
        }
        self.push_shape(DrawShape::new(
            layer,
            ShapeKind::Polygon(Polygon::new(points)),
        ));
    }

    /// Resolves an entity's layer (group code 8, defaulting to `"0"` per DXF
    /// convention), applying the token-length cap.
    fn layer_for(&mut self, data: &[(i32, &str)]) -> LayerId {
        let name = find_str(data, 8).unwrap_or("0");
        let name = self.cap_token(name);
        self.resolve_layer(&name)
    }

    fn finish_ring_or_path(
        &mut self,
        layer: LayerId,
        closed: bool,
        points: Vec<Point>,
        what: &'static str,
    ) {
        if closed {
            if points.len() < 3 {
                self.warn(
                    WarningKind::DegenerateGeometry,
                    "polyline skipped: fewer than 3 vertices",
                    format!("a closed DXF {what} had {} vertices; skipped", points.len()),
                );
                return;
            }
            self.push_shape(DrawShape::new(
                layer,
                ShapeKind::Polygon(Polygon::new(points)),
            ));
        } else {
            if points.len() < 2 {
                self.warn(
                    WarningKind::DegenerateGeometry,
                    "polyline skipped: fewer than 2 vertices",
                    format!("an open DXF {what} had {} vertices; skipped", points.len()),
                );
                return;
            }
            self.push_shape(DrawShape::new(
                layer,
                ShapeKind::Path(Path::new(points, 0, Endcap::Flat)),
            ));
        }
    }

    fn push_shape(&mut self, shape: DrawShape) {
        if self.shapes.len() >= MAX_SHAPES {
            self.warn(
                WarningKind::LimitExceeded,
                "DXF shape count exceeds cap",
                format!("more than {MAX_SHAPES} shapes were read; further entities are dropped"),
            );
            return;
        }
        self.shapes.push(shape);
    }

    /// Resolves a layer name to its [`LayerId`], assigning a fresh one in
    /// first-seen order (capped by [`MAX_LAYERS`]; see the [module docs](self)).
    fn resolve_layer(&mut self, name: &str) -> LayerId {
        if let Some(id) = self.layer_ids.get(name) {
            return *id;
        }
        if self.layer_ids.len() >= MAX_LAYERS {
            self.warn(
                WarningKind::LimitExceeded,
                "DXF distinct layer count exceeds cap",
                format!(
                    "more than {MAX_LAYERS} distinct layer (group code 8) names were seen; \
                     further ones share a fallback layer id"
                ),
            );
            return LAYER_OVERFLOW;
        }
        let id = LayerId::new(self.layer_ids.len() as u16, 0);
        self.layer_ids.insert(name.to_string(), id);
        self.layers.push(LayerInfo {
            id,
            name: name.to_string(),
            color_rgba: 0xFFFF_FFFF,
            visible: true,
        });
        id
    }

    /// Truncates `s` to [`MAX_TOKEN_LEN`] bytes (at a valid UTF-8 boundary),
    /// warning once if truncation occurred.
    fn cap_token(&mut self, s: &str) -> String {
        if s.len() <= MAX_TOKEN_LEN {
            return s.to_string();
        }
        self.warn(
            WarningKind::LimitExceeded,
            "DXF layer name truncated: exceeds length cap",
            format!("a layer name longer than {MAX_TOKEN_LEN} bytes was truncated"),
        );
        let mut end = MAX_TOKEN_LEN;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }

    fn warn(&mut self, kind: WarningKind, summary: impl Into<String>, detail: impl Into<String>) {
        self.warnings
            .push(ImportWarning::new(kind, summary, detail));
    }

    /// Consumes the parser state, assembling the final [`Document`]. Every
    /// shape lives flat in a single synthetic `"TOP"` cell: this reader's
    /// scope (the bare `ENTITIES` section) has no block/insert hierarchy.
    fn finalize(self) -> (Document, Vec<ImportWarning>) {
        let mut doc = Document::new();
        let mut top_cells = Vec::new();
        if !self.shapes.is_empty() {
            let mut cell = Cell::new("TOP");
            cell.shapes = self.shapes;
            doc.insert_cell(cell);
            top_cells.push("TOP".to_string());
        }
        doc.set_top_cells(top_cells);
        let tech = Technology {
            dbu_per_micron: DEFAULT_DBU_PER_MICRON,
            layers: self.layers,
            ..Technology::default()
        };
        doc.set_technology(tech);
        (doc, self.warnings.into_vec())
    }
}

/// Collects a vertex list from an `LWPOLYLINE`'s data: every group-10 value
/// opens a new vertex, completed by the next group-20 value. Every other code
/// (width, bulge, elevation, ...) is ignored (see the [module docs](self) for
/// the honest gaps this implies). Returns the points plus whether the list ran
/// past [`MAX_SHAPE_VERTICES`] (in which case the caller drops the shape).
fn collect_vertices(data: &[(i32, &str)]) -> Result<(Vec<Point>, bool)> {
    let mut points = Vec::new();
    let mut pending_x: Option<Dbu> = None;
    let mut truncated = false;
    for &(code, value) in data {
        match code {
            10 => pending_x = Some(to_dbu(parse_f64(value)?)),
            20 => {
                if let Some(x) = pending_x.take() {
                    let y = to_dbu(parse_f64(value)?);
                    if points.len() < MAX_SHAPE_VERTICES {
                        points.push(Point::new(x, y));
                    } else {
                        truncated = true;
                    }
                }
            }
            _ => {}
        }
    }
    Ok((points, truncated))
}

/// Reads a flags field (group `code`, defaulting to 0 when absent) and
/// reports whether bit 0 is set (the `LWPOLYLINE`/`POLYLINE` "closed" flag).
fn flag_bit0_set(data: &[(i32, &str)], code: i32) -> Result<bool> {
    let flags = match find_str(data, code) {
        Some(tok) => parse_i64(tok)?,
        None => 0,
    };
    Ok(flags & 1 != 0)
}

/// Polygonizes a circular arc of `radius_dbu` centered at `center`, sweeping
/// `sweep_degrees` counter-clockwise from `start_degrees`. When `full_circle`
/// is `true` the closing duplicate vertex is omitted (the ring is implicitly
/// closed by [`Polygon`]); otherwise both true endpoints are included exactly.
/// See [`arc_segment_count`] for how the segment count is chosen.
fn polygonize_arc(
    center: Point,
    radius_dbu: f64,
    start_degrees: f64,
    sweep_degrees: f64,
    full_circle: bool,
) -> Vec<Point> {
    let segments = arc_segment_count(radius_dbu, sweep_degrees);
    if segments == 0 {
        return Vec::new();
    }
    let count = if full_circle { segments } else { segments + 1 };
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let t = start_degrees + sweep_degrees * (f64_from_usize(i) / f64_from_usize(segments));
        let rad = t.to_radians();
        let x = f64::from(center.x) + radius_dbu * rad.cos();
        let y = f64::from(center.y) + radius_dbu * rad.sin();
        points.push(Point::new(to_dbu(x), to_dbu(y)));
    }
    points
}

/// Chooses how many segments approximate a `radius_dbu` arc sweeping
/// `sweep_degrees` so that the sagitta (the chord-to-arc gap) never exceeds
/// [`ARC_SAGITTA_TOLERANCE_DBU`], clamped to between [`MIN_ARC_SEGMENTS`] and
/// [`MAX_ARC_SEGMENTS`]. Returns 0 for a non-positive radius or sweep.
fn arc_segment_count(radius_dbu: f64, sweep_degrees: f64) -> usize {
    if radius_dbu <= 0.0 || sweep_degrees <= 0.0 {
        return 0;
    }
    let tolerance = ARC_SAGITTA_TOLERANCE_DBU.min(radius_dbu * 0.999);
    // sagitta = r * (1 - cos(theta / 2))  =>  theta = 2 * acos(1 - sagitta / r).
    let ratio = (1.0 - tolerance / radius_dbu).clamp(-1.0, 1.0);
    let max_theta = 2.0 * ratio.acos();
    let sweep_rad = sweep_degrees.to_radians();
    let needed = if max_theta > 1e-9 {
        (sweep_rad / max_theta).ceil()
    } else {
        MAX_ARC_SEGMENTS as f64
    };
    let floor = if sweep_degrees >= 359.999 {
        MIN_ARC_SEGMENTS
    } else {
        2
    };
    usize_from_f64(needed).clamp(floor, MAX_ARC_SEGMENTS)
}

fn find_str<'p>(data: &[(i32, &'p str)], code: i32) -> Option<&'p str> {
    data.iter().find(|(c, _)| *c == code).map(|&(_, v)| v)
}

/// Reads a required floating-point field, failing with `msg` if the group
/// `code` is absent or its value is not a finite number.
fn require_f64(data: &[(i32, &str)], code: i32, msg: &'static str) -> Result<f64> {
    let tok = find_str(data, code).ok_or_else(|| malformed(msg))?;
    parse_f64(tok)
}

/// Parses a DXF floating-point value token.
///
/// # Errors
///
/// Returns a [`ModelError`] if `tok` is not a valid, finite number.
fn parse_f64(tok: &str) -> Result<f64> {
    let v: f64 = tok
        .parse()
        .map_err(|_| malformed("DXF value is not a valid number"))?;
    if !v.is_finite() {
        return Err(malformed("DXF value is not a finite number"));
    }
    Ok(v)
}

/// Parses a DXF integer value token (a flags or count field).
///
/// # Errors
///
/// Returns a [`ModelError`] if `tok` is not a valid base-10 integer.
fn parse_i64(tok: &str) -> Result<i64> {
    tok.parse()
        .map_err(|_| malformed("DXF value is not a valid integer"))
}

/// Rounds a DXF coordinate or length to the nearest [`Dbu`], saturating into
/// range (see the [module docs](self): one DXF drawing unit equals one `Dbu`).
fn to_dbu(v: f64) -> Dbu {
    v.round().clamp(f64::from(Dbu::MIN), f64::from(Dbu::MAX)) as Dbu
}

/// Widens a small loop index to `f64` for the polygonization angle formula.
fn f64_from_usize(v: usize) -> f64 {
    u32::try_from(v).map_or(f64::from(u32::MAX), f64::from)
}

/// Narrows a non-negative `f64` segment count to `usize`, saturating rather
/// than wrapping on an out-of-range value.
fn usize_from_f64(v: f64) -> usize {
    if v <= 0.0 {
        0
    } else if v >= f64::from(u32::MAX) {
        u32::MAX as usize
    } else {
        v as u32 as usize
    }
}

/// Narrows a `HATCH` group-93 count to `usize`, saturating a negative or
/// out-of-range claim rather than trusting it; the caller never preallocates
/// from this value, only uses it to decide when a vertex list is complete.
fn usize_from_i64(v: i64) -> usize {
    usize::try_from(v).unwrap_or(0)
}

/// Builds a lowered [`ModelError`] from a static description, the DXF
/// reader's sole error constructor (see [`IoError::Malformed`]).
fn malformed(msg: &'static str) -> ModelError {
    IoError::Malformed(msg).into()
}

/// A bounded accumulator for [`ImportWarning`]s raised during an import.
///
/// Deduplicates by category so a pathological file yields one representative
/// warning per category with a running count, not one warning per occurrence.
/// Mirrors `gds.rs`/`cif.rs`'s identically shaped helper; each format module
/// keeps its own copy so the modules stay independently buildable.
struct Warnings {
    seen: Vec<(WarningKind, ImportWarning, usize)>,
}

impl Warnings {
    fn new() -> Self {
        Self { seen: Vec::new() }
    }

    fn push(&mut self, w: ImportWarning) {
        if let Some(entry) = self.seen.iter_mut().find(|(k, ..)| *k == w.kind) {
            entry.2 += 1;
        } else {
            let kind = w.kind;
            self.seen.push((kind, w, 1));
        }
    }

    fn into_vec(self) -> Vec<ImportWarning> {
        self.seen
            .into_iter()
            .map(|(_, mut w, count)| {
                if count > 1 {
                    w.detail = format!(
                        "{} ({} occurrences of this kind; first shown)",
                        w.detail, count
                    );
                }
                w
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{arc_segment_count, parse_f64, parse_i64, to_dbu, tokenize};

    #[test]
    fn tokenize_pairs_lines() {
        let pairs = tokenize("0\nLINE\n8\nA\n").expect("well-formed pairs tokenize");
        assert_eq!(pairs, vec![(0, "LINE"), (8, "A")]);
    }

    #[test]
    fn tokenize_rejects_odd_line_count() {
        assert!(tokenize("0\nLINE\n8\n").is_err());
    }

    #[test]
    fn tokenize_rejects_non_numeric_group_code() {
        assert!(tokenize("x\nLINE\n").is_err());
    }

    #[test]
    fn parse_f64_rejects_non_numeric_and_non_finite() {
        assert!(parse_f64("abc").is_err());
        assert!(parse_f64("nan").is_err());
        assert!(parse_f64("inf").is_err());
        assert!((parse_f64("12.5").unwrap() - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_i64_rejects_non_numeric() {
        assert!(parse_i64("1.5").is_err());
        assert_eq!(parse_i64("42").unwrap(), 42);
    }

    #[test]
    fn to_dbu_rounds_and_saturates() {
        assert_eq!(to_dbu(1.4), 1);
        assert_eq!(to_dbu(1.5), 2);
        assert_eq!(to_dbu(f64::MAX), i32::MAX);
        assert_eq!(to_dbu(f64::MIN), i32::MIN);
    }

    #[test]
    fn arc_segment_count_is_bounded() {
        assert_eq!(arc_segment_count(0.0, 360.0), 0);
        assert_eq!(arc_segment_count(100.0, 0.0), 0);
        let full = arc_segment_count(100.0, 360.0);
        assert!((super::MIN_ARC_SEGMENTS..=super::MAX_ARC_SEGMENTS).contains(&full));
        // A huge radius must still clamp to the hard ceiling.
        let huge = arc_segment_count(1.0e12, 360.0);
        assert_eq!(huge, super::MAX_ARC_SEGMENTS);
    }
}

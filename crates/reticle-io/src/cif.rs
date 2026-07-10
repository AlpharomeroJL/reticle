//! CIF (Caltech Intermediate Format) reader: the classic subset.
//!
//! # Scope
//!
//! This module reads the "classic" CIF 2.0 primitive subset used by MOSIS-era
//! mask layouts: symbol definitions (`DS`/`DF`), an optional symbol-name comment
//! (`9`), layers (`L`), boxes (`B`), polygons (`P`), wires (`W`), symbol calls
//! with transforms (`C`), and the end marker (`E`). Parenthesized `(...)`
//! comments are recognized and stripped anywhere in the file. This is import
//! only; there is no exporter (out of scope, symmetric to
//! [`OasisStd`](crate::OasisStd), which is export-only).
//!
//! Not implemented (an honest gap, not silently dropped): CIF text/label
//! extensions (for example the `94`/`95`/`96` conventions some tools emit),
//! rounded-flash geometry, and any vendor-specific resolution pragma carried in
//! a comment (a comment's text is always discarded, never interpreted). An
//! unrecognized top-level command is skipped with a warning rather than
//! rejected, so one private extension statement does not sink an otherwise
//! good file.
//!
//! # Grammar (this subset)
//!
//! ```text
//! DS <n> [<a> <b>];      begin symbol n; a/b scales this symbol's own
//!                        coordinates into file units (default 1/1)
//! 9 <name>;              names the innermost open symbol (optional)
//! L <name>;              sets the current layer for subsequent geometry
//! B <w> <h> <cx> <cy> [<dx> <dy>];    axis-aligned box, w by h, centered at
//!                        (cx,cy), optionally rotated toward (dx,dy)
//! P <x1> <y1> ... <xn> <yn>;          polygon (3 or more points)
//! W <width> <x1> <y1> ... <xn> <yn>;  wire (2 or more points), round caps
//! C <n> [T <tx> <ty> | R <dx> <dy> | M X | M Y]*;  place symbol n, applying
//!                        zero or more transforms left to right
//! DF;                    end the current symbol
//! E                      end of file; anything after is ignored
//! ( comment text )       stripped anywhere; comments do not nest
//! ```
//!
//! Geometry outside any `DS`/`DF` pair is collected into a synthetic top-level
//! cell named `TOP` (present in the document only when non-empty), matching how
//! many real CIF files place their principal structure directly at the top
//! level and use `DS`/`DF` only for reusable sub-cells. A symbol `C`-referenced
//! by number but never `DS`-defined resolves to its default name anyway (a
//! dangling reference), tolerated rather than rejected, matching how the rest
//! of Reticle's document model already treats an instance whose cell is
//! missing: [`Document::flatten`](reticle_model::Document::flatten) and
//! [`Document::cell_bbox`](reticle_model::Document::cell_bbox) simply skip it.
//!
//! # Units, scale, and layers
//!
//! CIF carries no `UNITS` record, so every import sets
//! [`Technology::dbu_per_micron`] to the classic default of 100 (1/100 micron,
//! a centimicron grid). A `9` statement at the top level (outside any `DS`)
//! sets [`Technology::name`] instead of naming a symbol, since Reticle's
//! document model has no other place for a library-level name.
//!
//! CIF layer names are short mnemonic strings, not the `(layer, datatype)` pair
//! [`LayerId`] uses. This reader assigns each distinct name seen a fresh
//! `LayerId::new(k, 0)` in first-seen order (`k` = 0, 1, 2, ...) and records the
//! mapping in [`Technology::layers`] so the original name is never lost. This is
//! a simple, deterministic, file-local scheme, not a standards mapping: two CIF
//! files using the same mnemonic for different physical layers get different
//! numbers here unless their `L` statements happen to appear in the same order.
//!
//! # Rotation
//!
//! Reticle's placement model supports only the eight Manhattan [`Orientation`]s.
//! `B`'s optional direction vector and `C`'s `R` transform are both snapped to
//! the nearest multiple of 90 degrees (matching how the GDSII importer handles
//! a non-Manhattan `STRANS` angle), with a [`WarningKind::ValueClamped`] warning
//! when the input was not already exactly axis-aligned. `C`'s `M X`/`M Y`
//! mirror the symbol about the Y axis / X axis respectively, matching classic
//! CIF semantics exactly (no snapping needed: a mirror is always exact).
//!
//! # Untrusted-input discipline
//!
//! Every count-driven allocation is capped against the remaining input, never
//! against a claimed count (CIF has no count-prefixed fields; a point list is
//! simply read until the statement's terminating `;`):
//!
//! * [`MAX_INPUT_BYTES`] (256 MiB) rejects an oversized input before any
//!   parsing begins, mirroring [`crate::gds::MAX_INPUT_BYTES`].
//! * [`MAX_SHAPE_VERTICES`] (200,000) caps a single `P`/`W` point list; a
//!   statement that runs past it is still parsed structurally (so the rest of
//!   the file stays in sync) but the oversized shape is dropped with a
//!   [`WarningKind::LimitExceeded`] warning rather than materialized, mirroring
//!   [`crate::gds::MAX_SHAPE_VERTICES`].
//! * [`MAX_CELLS`] caps the number of distinct `DS` symbols materialized as
//!   cells; further definitions parse but are discarded, with one deduped
//!   warning.
//! * [`MAX_LAYERS`] caps the number of distinct `L` layer names tracked;
//!   further names fold onto a fixed overflow [`LayerId`], with one deduped
//!   warning.
//! * A `9` name or `L` layer name longer than [`MAX_TOKEN_LEN`] is truncated
//!   (at a valid UTF-8 boundary) rather than kept unbounded, with one deduped
//!   warning.
//!
//! Malformed input (a truncated statement, a non-numeric token where a number
//! is required, an unterminated comment, invalid UTF-8, a count past a cap)
//! is rejected with a structured [`IoError`], never a panic or an unbounded
//! allocation. A single degenerate shape (too few vertices, non-positive box
//! dimensions, negative wire width) is recoverable: it is skipped with a
//! warning and the rest of the file still imports, matching
//! [`Gds::import_with_warnings`](crate::Gds::import_with_warnings)'s
//! philosophy of "one bad element does not sink an otherwise-good file".

use crate::IoError;
use crate::error::{ImportWarning, WarningKind};
use reticle_geometry::{Dbu, Endcap, LayerId, Orientation, Path, Point, Polygon, Rect, Transform};
use reticle_model::{
    Cell, Document, DrawShape, Importer, Instance, LayerInfo, ModelError, Result, ShapeKind,
    Technology,
};
use std::collections::{HashMap, HashSet};

/// CIF import (classic subset; see the [module docs](self)).
///
/// Implements [`Importer`] (bytes to [`Document`]). There is no exporter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cif;

/// The largest CIF input this importer will attempt to parse, in bytes
/// (256 MiB). A stream at or under this bound parses within a bounded
/// allocation; a larger one is refused with a clear [`IoError`] rather than
/// risking an out-of-memory abort on a hostile or truncated-huge input. Hand
/// and tool-written CIF for a single design is typically kilobytes to a few
/// megabytes, far under this ceiling.
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

/// The largest number of vertices a single `P` (polygon) or `W` (wire) point
/// list is allowed to carry into the model. CIF places no explicit limit on a
/// point list's length (it simply reads numbers up to the terminating `;`), so
/// this is the defense-in-depth ceiling: past it the shape is skipped with a
/// [`WarningKind::LimitExceeded`] warning rather than materialized. A real
/// hand-drawn or tool-generated CIF shape is nowhere near this size.
pub const MAX_SHAPE_VERTICES: usize = 200_000;

/// The largest number of distinct `DS` symbols this importer will materialize
/// as cells. Past this cap, further symbol definitions are still parsed (so
/// the rest of the file stays in sync) but are not inserted into the document;
/// one deduped [`WarningKind::LimitExceeded`] warning records the cap being
/// hit. Real CIF designs (MOSIS-era student and small-team chip projects) are
/// nowhere near this many distinct symbols.
pub const MAX_CELLS: usize = 50_000;

/// The largest number of distinct `L` layer names this importer will track.
/// Past this cap, further distinct names resolve to a single fixed overflow
/// [`LayerId`] rather than growing the name table without bound; one deduped
/// [`WarningKind::LimitExceeded`] warning records the cap being hit. Real
/// process technologies use well under a hundred layers.
pub const MAX_LAYERS: usize = 4_096;

/// The longest a single `9` (symbol name) or `L` (layer name) token is kept
/// before being truncated (at a valid UTF-8 boundary), with one deduped
/// [`WarningKind::LimitExceeded`] warning. Defense in depth against a hostile
/// single-token name; real names are a handful of characters.
pub const MAX_TOKEN_LEN: usize = 4_096;

/// The database resolution assumed for every CIF import: 100 units per micron
/// (a centimicron grid), the classic CIF default. CIF carries no `UNITS`
/// record and this reader does not interpret any vendor resolution pragma (see
/// the [module docs](self)), so this value is used unconditionally.
const DEFAULT_DBU_PER_MICRON: i64 = 100;

/// The fixed [`LayerId`] every layer name past [`MAX_LAYERS`] folds onto.
/// Chosen at the top of the `u16` range so it can never collide with a
/// sequentially assigned real layer (which stops at `MAX_LAYERS - 1 < 4096`).
const LAYER_OVERFLOW: LayerId = LayerId::new(0xFFFF, 0xFFFF);

/// The result of a CIF import that kept its non-fatal warnings.
///
/// Returned by [`Cif::import_with_warnings`]. The [`document`](CifImport::document)
/// is always well-formed and safe to use; [`warnings`](CifImport::warnings) lists
/// every recoverable problem that was skipped, clamped, or defaulted during the
/// import (empty for a clean file). The frozen [`Importer::import`] path
/// discards the warnings and returns only the document.
#[derive(Debug, Clone)]
pub struct CifImport {
    /// The imported document. Always valid, even when warnings are present.
    pub document: Document,
    /// Recoverable problems found during import, in encounter order.
    pub warnings: Vec<ImportWarning>,
}

impl Importer for Cif {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        Ok(self.import_with_warnings(bytes)?.document)
    }
}

impl Cif {
    /// Imports CIF `bytes` into a [`Document`], keeping every non-fatal warning.
    ///
    /// This is the hardened import entry point; see the [module docs](self) for
    /// the full grammar and untrusted-input discipline. It never panics and
    /// never allocates unboundedly on any input.
    ///
    /// # Errors
    ///
    /// Returns a [`reticle_model::ModelError`] (via [`IoError`]) when the input
    /// is too large, is not valid UTF-8, or is malformed CIF (an unterminated
    /// comment or statement, a non-numeric token where a number is required, a
    /// structurally invalid statement).
    pub fn import_with_warnings(&self, bytes: &[u8]) -> Result<CifImport> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(malformed(
                "CIF input exceeds the maximum accepted size (256 MiB)",
            ));
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| malformed("CIF input is not valid UTF-8"))?;
        let stripped = strip_comments(text)?;

        // Split off the trailing content after the last `;` so it can be
        // validated separately: it must be empty (ordinary trailing whitespace)
        // or exactly a lenient, semicolon-less final `E`. Anything else is an
        // incomplete final statement (truncated input).
        let (body, leftover) = match stripped.rfind(';') {
            Some(idx) => (&stripped[..=idx], &stripped[idx + 1..]),
            None => ("", stripped.as_str()),
        };
        let leftover_trimmed = leftover.trim();
        if !leftover_trimmed.is_empty() && leftover_trimmed != "E" {
            return Err(malformed(
                "CIF input ends with an incomplete statement (missing ';')",
            ));
        }

        let mut state = ParseState::new();
        for stmt in body.split(';') {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            if state.handle_statement(stmt)? {
                break; // saw `E`
            }
        }
        if state.current_symbol.is_some() {
            return Err(malformed(
                "CIF input ends with an unterminated symbol definition (DS without a matching DF)",
            ));
        }

        let (document, warnings) = state.finalize();
        Ok(CifImport { document, warnings })
    }
}

/// One symbol (`DS`/`DF` block) or the synthetic top-level content being
/// accumulated during parsing, before symbol numbers are resolved to names.
struct RawCell {
    name: String,
    shapes: Vec<DrawShape>,
    /// Pending placements: the called symbol's raw number plus the resolved
    /// transform. Resolved to a [`reticle_model::Instance`] only once the whole
    /// file has been parsed, so a call can name a symbol defined earlier or
    /// later in the file (or never).
    calls: Vec<(u32, Transform)>,
}

impl RawCell {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            shapes: Vec::new(),
            calls: Vec::new(),
        }
    }
}

/// State for a `DS`/`DF` block currently open.
struct CurrentSymbol {
    number: u32,
    /// The `a, b` scale factor (`a > 0`, `b > 0`); coordinates parsed within
    /// this block are multiplied by `a/b` on the way into the model.
    scale: (i64, i64),
    cell: RawCell,
    /// `true` once [`MAX_CELLS`] has already been reached, so this block's
    /// content is parsed (to stay in sync with the file) but not kept.
    discarded: bool,
}

/// The parser's running state across the whole file.
struct ParseState {
    current_layer: Option<LayerId>,
    layer_ids: HashMap<String, LayerId>,
    layers: Vec<LayerInfo>,
    top: RawCell,
    /// Symbol numbers formally opened with `DS` (redefinition guard).
    defined_symbols: HashSet<u32>,
    /// Symbol number to its current resolved name (default or `9`-given),
    /// consulted when a `C` call is finally resolved to an instance.
    symbol_name: HashMap<u32, String>,
    /// Materialized (non-discarded) symbols, in definition order.
    cells: Vec<(u32, RawCell)>,
    /// How many symbols have been accepted so far (gates [`MAX_CELLS`]).
    accepted_cells: usize,
    current_symbol: Option<CurrentSymbol>,
    /// Set by a top-level (outside any `DS`) `9` statement.
    tech_name: Option<String>,
    warnings: Warnings,
}

impl ParseState {
    fn new() -> Self {
        Self {
            current_layer: None,
            layer_ids: HashMap::new(),
            layers: Vec::new(),
            top: RawCell::new("TOP"),
            defined_symbols: HashSet::new(),
            symbol_name: HashMap::new(),
            cells: Vec::new(),
            accepted_cells: 0,
            current_symbol: None,
            tech_name: None,
            warnings: Warnings::new(),
        }
    }

    /// Dispatches one `;`-delimited, comment-stripped, non-empty statement.
    /// Returns `Ok(true)` when the statement was `E` (end of file): the caller
    /// stops processing further statements.
    fn handle_statement(&mut self, stmt: &str) -> Result<bool> {
        let toks: Vec<&str> = stmt.split_whitespace().collect();
        let Some((cmd, rest)) = toks.split_first() else {
            return Ok(false);
        };
        match *cmd {
            "DS" => self.begin_symbol(rest)?,
            "DF" => self.end_symbol(rest)?,
            "9" => self.set_name(rest)?,
            "L" => self.set_layer(rest)?,
            "B" => self.add_box(rest)?,
            "P" => self.add_polygon(rest)?,
            "W" => self.add_wire(rest)?,
            "C" => self.add_call(rest)?,
            "E" => return Ok(true),
            _ => self.warn(
                WarningKind::UnsupportedFeature,
                "CIF unknown command skipped",
                format!(
                    "statement `{stmt}` uses a command this reader does not recognize; skipped"
                ),
            ),
        }
        Ok(false)
    }

    fn begin_symbol(&mut self, rest: &[&str]) -> Result<()> {
        if self.current_symbol.is_some() {
            return Err(malformed(
                "CIF DS while a symbol definition is already open (nesting is not supported)",
            ));
        }
        let (number_tok, scale) = match rest {
            [n] => (*n, (1i64, 1i64)),
            [n, a, b] => {
                let a = parse_int(a)?;
                let b = parse_int(b)?;
                if a <= 0 || b <= 0 {
                    return Err(malformed(
                        "CIF DS scale factor must be positive (a > 0 and b > 0)",
                    ));
                }
                (*n, (a, b))
            }
            _ => {
                return Err(malformed(
                    "CIF DS expects a symbol number, or a symbol number and a/b scale",
                ));
            }
        };
        let number = parse_u32(number_tok)?;
        if !self.defined_symbols.insert(number) {
            return Err(malformed(
                "CIF DS redefines an already-defined symbol number",
            ));
        }

        let discarded = self.accepted_cells >= MAX_CELLS;
        if discarded {
            self.warn(
                WarningKind::LimitExceeded,
                "CIF symbol count exceeds cap; further definitions dropped",
                format!(
                    "more than {MAX_CELLS} distinct DS symbols were defined; this and later ones are parsed but not materialized"
                ),
            );
        } else {
            self.accepted_cells += 1;
        }

        let name = default_cell_name(number);
        self.symbol_name.insert(number, name.clone());
        self.current_symbol = Some(CurrentSymbol {
            number,
            scale,
            cell: RawCell::new(name),
            discarded,
        });
        Ok(())
    }

    fn end_symbol(&mut self, rest: &[&str]) -> Result<()> {
        if !rest.is_empty() {
            return Err(malformed("CIF DF takes no arguments"));
        }
        let Some(sym) = self.current_symbol.take() else {
            return Err(malformed("CIF DF with no matching open DS"));
        };
        if !sym.discarded {
            self.cells.push((sym.number, sym.cell));
        }
        Ok(())
    }

    fn set_name(&mut self, rest: &[&str]) -> Result<()> {
        let [name] = rest else {
            return Err(malformed(
                "CIF 9 (symbol name) expects exactly one name token",
            ));
        };
        let resolved = self.cap_token((*name).to_string());
        if let Some(sym) = &mut self.current_symbol {
            sym.cell.name.clone_from(&resolved);
            self.symbol_name.insert(sym.number, resolved);
        } else {
            self.tech_name = Some(resolved);
        }
        Ok(())
    }

    fn set_layer(&mut self, rest: &[&str]) -> Result<()> {
        let [name] = rest else {
            return Err(malformed("CIF L (layer) expects exactly one layer name"));
        };
        let name = self.cap_token((*name).to_string());
        self.current_layer = Some(self.resolve_layer_id(name));
        Ok(())
    }

    fn add_box(&mut self, rest: &[&str]) -> Result<()> {
        if rest.len() != 4 && rest.len() != 6 {
            return Err(malformed(
                "CIF B (box) expects 4 or 6 numbers: w h cx cy [dx dy]",
            ));
        }
        let Some(layer) = self.current_layer else {
            return Err(malformed(
                "CIF shape statement appears before any L (layer) statement",
            ));
        };
        let mut nums = Vec::with_capacity(rest.len());
        for t in rest {
            nums.push(parse_int(t)?);
        }
        let (a, b) = self.active_scale();
        let w = scale_to_dbu(nums[0], a, b);
        let h = scale_to_dbu(nums[1], a, b);
        let cx = scale_to_dbu(nums[2], a, b);
        let cy = scale_to_dbu(nums[3], a, b);
        if w <= 0 || h <= 0 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "box skipped: non-positive width or height",
                format!("a CIF B box had width {w} height {h} (both must be positive); skipped"),
            );
            return Ok(());
        }
        let (dx, dy) = if nums.len() == 6 {
            (nums[4], nums[5])
        } else {
            (1, 0)
        };
        let (orientation, exact) = quadrant_from_vector(dx, dy);
        if !exact {
            self.warn(
                WarningKind::ValueClamped,
                "box rotation snapped to nearest 90 degrees",
                format!(
                    "a CIF B box direction ({dx}, {dy}) is not axis-aligned; Reticle only \
                     places on the four Manhattan orientations, so it was snapped to the \
                     nearest one"
                ),
            );
        }
        let (ex, ey) = if matches!(orientation, Orientation::R90 | Orientation::R270) {
            (h, w)
        } else {
            (w, h)
        };
        let min_x = cx.saturating_sub(ex / 2);
        let min_y = cy.saturating_sub(ey / 2);
        let rect = Rect::new(
            Point::new(min_x, min_y),
            Point::new(min_x.saturating_add(ex), min_y.saturating_add(ey)),
        );
        self.push_shape(DrawShape::new(layer, ShapeKind::Rect(rect)));
        Ok(())
    }

    fn add_polygon(&mut self, rest: &[&str]) -> Result<()> {
        if !rest.len().is_multiple_of(2) {
            return Err(malformed("CIF P has an odd number of coordinates"));
        }
        let Some(layer) = self.current_layer else {
            return Err(malformed(
                "CIF shape statement appears before any L (layer) statement",
            ));
        };
        let (a, b) = self.active_scale();
        let mut pts: Vec<Point> = Vec::new();
        let mut truncated = false;
        for pair in rest.chunks_exact(2) {
            let x = scale_to_dbu(parse_int(pair[0])?, a, b);
            let y = scale_to_dbu(parse_int(pair[1])?, a, b);
            if pts.len() < MAX_SHAPE_VERTICES {
                pts.push(Point::new(x, y));
            } else {
                truncated = true;
            }
        }
        if truncated {
            self.warn(
                WarningKind::LimitExceeded,
                "polygon skipped: too many vertices",
                format!("a CIF P polygon had more than {MAX_SHAPE_VERTICES} vertices; skipped"),
            );
            return Ok(());
        }
        if pts.len() < 3 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "polygon skipped: fewer than 3 vertices",
                format!("a CIF P polygon had {} vertices; skipped", pts.len()),
            );
            return Ok(());
        }
        self.push_shape(DrawShape::new(layer, ShapeKind::Polygon(Polygon::new(pts))));
        Ok(())
    }

    fn add_wire(&mut self, rest: &[&str]) -> Result<()> {
        let [width_tok, coords @ ..] = rest else {
            return Err(malformed("CIF W (wire) missing width"));
        };
        if !coords.len().is_multiple_of(2) {
            return Err(malformed("CIF W has an odd number of coordinates"));
        }
        let Some(layer) = self.current_layer else {
            return Err(malformed(
                "CIF shape statement appears before any L (layer) statement",
            ));
        };
        let (a, b) = self.active_scale();
        let width = scale_to_dbu(parse_int(width_tok)?, a, b);
        let mut pts: Vec<Point> = Vec::new();
        let mut truncated = false;
        for pair in coords.chunks_exact(2) {
            let x = scale_to_dbu(parse_int(pair[0])?, a, b);
            let y = scale_to_dbu(parse_int(pair[1])?, a, b);
            if pts.len() < MAX_SHAPE_VERTICES {
                pts.push(Point::new(x, y));
            } else {
                truncated = true;
            }
        }
        if truncated {
            self.warn(
                WarningKind::LimitExceeded,
                "wire skipped: too many vertices",
                format!("a CIF W wire had more than {MAX_SHAPE_VERTICES} points; skipped"),
            );
            return Ok(());
        }
        if width < 0 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "wire skipped: negative width",
                format!("a CIF W wire had width {width}; skipped"),
            );
            return Ok(());
        }
        if pts.len() < 2 {
            self.warn(
                WarningKind::DegenerateGeometry,
                "wire skipped: fewer than 2 points",
                format!("a CIF W wire had {} point(s); skipped", pts.len()),
            );
            return Ok(());
        }
        self.push_shape(DrawShape::new(
            layer,
            ShapeKind::Path(Path::new(pts, width, Endcap::Round)),
        ));
        Ok(())
    }

    fn add_call(&mut self, rest: &[&str]) -> Result<()> {
        let [number_tok, transform_toks @ ..] = rest else {
            return Err(malformed("CIF C (call) expects a symbol number"));
        };
        let number = parse_u32(number_tok)?;
        let mut transform = Transform::IDENTITY;
        let mut i = 0;
        while i < transform_toks.len() {
            match transform_toks[i] {
                "T" => {
                    if i + 2 >= transform_toks.len() {
                        return Err(malformed("CIF C transform T expects two numbers (tx ty)"));
                    }
                    let (a, b) = self.active_scale();
                    let tx = scale_to_dbu(parse_int(transform_toks[i + 1])?, a, b);
                    let ty = scale_to_dbu(parse_int(transform_toks[i + 2])?, a, b);
                    transform = transform.then(&Transform::translate(tx, ty));
                    i += 3;
                }
                "R" => {
                    if i + 2 >= transform_toks.len() {
                        return Err(malformed("CIF C transform R expects two numbers (dx dy)"));
                    }
                    let dx = parse_int(transform_toks[i + 1])?;
                    let dy = parse_int(transform_toks[i + 2])?;
                    let (orientation, exact) = quadrant_from_vector(dx, dy);
                    if !exact {
                        self.warn(
                            WarningKind::ValueClamped,
                            "call rotation snapped to nearest 90 degrees",
                            format!(
                                "a CIF C call direction ({dx}, {dy}) is not axis-aligned; \
                                 snapped to the nearest Manhattan orientation"
                            ),
                        );
                    }
                    transform = transform.then(&Transform {
                        orientation,
                        ..Transform::IDENTITY
                    });
                    i += 3;
                }
                "M" => {
                    if i + 1 >= transform_toks.len() {
                        return Err(malformed(
                            "CIF C transform M expects exactly one operand, X or Y",
                        ));
                    }
                    let orientation = match transform_toks[i + 1] {
                        "X" => Orientation::MirrorX180, // mirror about the Y axis: negate x
                        "Y" => Orientation::MirrorX,    // mirror about the X axis: negate y
                        _ => {
                            return Err(malformed(
                                "CIF C transform M expects exactly one operand, X or Y",
                            ));
                        }
                    };
                    transform = transform.then(&Transform {
                        orientation,
                        ..Transform::IDENTITY
                    });
                    i += 2;
                }
                _ => {
                    return Err(malformed(
                        "CIF C has an unrecognized transform op (expected T, R, or M)",
                    ));
                }
            }
        }
        self.push_call(number, transform);
        Ok(())
    }

    /// The scale factor in effect for the innermost open symbol, or `1/1` at
    /// the top level (top-level content has no symbol-local grid to convert).
    fn active_scale(&self) -> (i64, i64) {
        self.current_symbol.as_ref().map_or((1, 1), |s| s.scale)
    }

    /// Whether the innermost open symbol's content is being discarded because
    /// [`MAX_CELLS`] was already reached (`false` at the top level, which is
    /// never discarded).
    fn active_discarded(&self) -> bool {
        self.current_symbol.as_ref().is_some_and(|s| s.discarded)
    }

    fn active_cell_mut(&mut self) -> &mut RawCell {
        match &mut self.current_symbol {
            Some(sym) => &mut sym.cell,
            None => &mut self.top,
        }
    }

    fn push_shape(&mut self, shape: DrawShape) {
        if !self.active_discarded() {
            self.active_cell_mut().shapes.push(shape);
        }
    }

    fn push_call(&mut self, number: u32, transform: Transform) {
        if !self.active_discarded() {
            self.active_cell_mut().calls.push((number, transform));
        }
    }

    /// Resolves a layer name to its [`LayerId`], assigning a fresh one in
    /// first-seen order (capped by [`MAX_LAYERS`]; see the [module docs](self)).
    fn resolve_layer_id(&mut self, name: String) -> LayerId {
        if let Some(id) = self.layer_ids.get(&name) {
            return *id;
        }
        if self.layer_ids.len() >= MAX_LAYERS {
            self.warn(
                WarningKind::LimitExceeded,
                "CIF distinct layer count exceeds cap",
                format!(
                    "more than {MAX_LAYERS} distinct L layer names were seen; further ones \
                     share a fallback layer id"
                ),
            );
            return LAYER_OVERFLOW;
        }
        let id = LayerId::new(self.layer_ids.len() as u16, 0);
        self.layer_ids.insert(name.clone(), id);
        self.layers.push(LayerInfo {
            id,
            name,
            color_rgba: 0xFFFF_FFFF,
            visible: true,
        });
        id
    }

    /// Truncates `s` to [`MAX_TOKEN_LEN`] bytes (at a valid UTF-8 boundary),
    /// warning once if truncation occurred.
    fn cap_token(&mut self, s: String) -> String {
        if s.len() <= MAX_TOKEN_LEN {
            return s;
        }
        self.warn(
            WarningKind::LimitExceeded,
            "CIF name truncated: exceeds length cap",
            format!("a name longer than {MAX_TOKEN_LEN} bytes was truncated"),
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

    /// Consumes the parser state, resolving every pending call to a
    /// [`reticle_model::Instance`] and assembling the final [`Document`].
    fn finalize(mut self) -> (Document, Vec<ImportWarning>) {
        let mut doc = Document::new();
        let mut used_names: HashSet<String> = HashSet::new();
        used_names.insert("TOP".to_string());

        // Assign final, collision-free names to every DS-defined cell, keeping
        // `symbol_name` in sync so `C` resolution below sees the final name
        // regardless of whether a call to it appeared before or after its `9`.
        let raw_cells = std::mem::take(&mut self.cells);
        let mut finalized: Vec<(u32, RawCell)> = Vec::with_capacity(raw_cells.len());
        for (number, mut raw) in raw_cells {
            let final_name = uniquify(raw.name, &mut used_names, &mut self.warnings);
            self.symbol_name.insert(number, final_name.clone());
            raw.name = final_name;
            finalized.push((number, raw));
        }

        // Every symbol number ever called, from any cell or the top level.
        let mut referenced: HashSet<u32> = HashSet::new();
        for (_, raw) in &finalized {
            for (n, _) in &raw.calls {
                referenced.insert(*n);
            }
        }
        for (n, _) in &self.top.calls {
            referenced.insert(*n);
        }

        let has_top_content = !self.top.shapes.is_empty() || !self.top.calls.is_empty();
        let mut top_cells: Vec<String> = Vec::new();
        if has_top_content {
            top_cells.push("TOP".to_string());
        }

        for (number, raw) in finalized {
            let name = raw.name;
            let mut cell = Cell::new(name.clone());
            cell.shapes = raw.shapes;
            for (n, transform) in raw.calls {
                let target = self
                    .symbol_name
                    .get(&n)
                    .cloned()
                    .unwrap_or_else(|| default_cell_name(n));
                cell.instances.push(Instance {
                    cell: target,
                    transform,
                });
            }
            if !referenced.contains(&number) {
                top_cells.push(name);
            }
            doc.insert_cell(cell);
        }

        if has_top_content {
            let mut cell = Cell::new("TOP");
            cell.shapes = self.top.shapes;
            for (n, transform) in self.top.calls {
                let target = self
                    .symbol_name
                    .get(&n)
                    .cloned()
                    .unwrap_or_else(|| default_cell_name(n));
                cell.instances.push(Instance {
                    cell: target,
                    transform,
                });
            }
            doc.insert_cell(cell);
        }

        let tech = Technology {
            name: self.tech_name.unwrap_or_default(),
            dbu_per_micron: DEFAULT_DBU_PER_MICRON,
            layers: self.layers,
            ..Technology::default()
        };
        doc.set_technology(tech);
        doc.set_top_cells(top_cells);
        (doc, self.warnings.into_vec())
    }
}

/// Ensures `base` is not already in `used`, disambiguating with a numeric
/// suffix (and a [`WarningKind::ValueClamped`] warning) when it collides. Used
/// so two symbols that both received the same `9`-given name still both
/// survive in the document rather than one silently overwriting the other.
fn uniquify(base: String, used: &mut HashSet<String>, warnings: &mut Warnings) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    warnings.push(ImportWarning::new(
        WarningKind::ValueClamped,
        "duplicate CIF symbol name",
        format!("the name `{base}` is used by more than one symbol; a later one was disambiguated"),
    ));
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}_{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// The default cell name for a `DS` symbol number that was never given a `9`
/// name (or, for a dangling `C` reference, one that was never `DS`-defined at
/// all).
fn default_cell_name(number: u32) -> String {
    format!("cif_{number}")
}

/// Strips CIF `(...)` comments (non-nested, matching the CIF spec) from
/// `text`, replacing each with a single space so tokens on either side stay
/// separated. Scans by [`char`] (not byte) so a multi-byte UTF-8 character
/// inside or around a comment is never split.
///
/// # Errors
///
/// Returns a [`ModelError`] if a `(` is never closed.
fn strip_comments(text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '(' {
            let mut closed = false;
            for c2 in chars.by_ref() {
                if c2 == ')' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err(malformed("CIF comment is never closed"));
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Parses a CIF integer token.
///
/// # Errors
///
/// Returns a [`ModelError`] if `tok` is not a valid base-10 integer.
fn parse_int(tok: &str) -> Result<i64> {
    tok.parse::<i64>()
        .map_err(|_| malformed("CIF token is not a valid integer"))
}

/// Parses a CIF symbol number (a non-negative integer that fits in `u32`).
///
/// # Errors
///
/// Returns a [`ModelError`] if `tok` is not such a number.
fn parse_u32(tok: &str) -> Result<u32> {
    tok.parse::<u32>()
        .map_err(|_| malformed("CIF symbol number is not a valid non-negative integer"))
}

/// Converts a raw CIF coordinate or length `raw` from the innermost open
/// symbol's local units to file units via its `a/b` scale (`a > 0`, `b > 0`,
/// default `1/1`), rounding half away from zero and saturating into the
/// [`Dbu`] range. Widened to [`i128`] so an adversarial scale factor cannot
/// overflow the intermediate product (a debug-mode overflow would panic; see
/// the [module docs](self)).
fn scale_to_dbu(raw: i64, a: i64, b: i64) -> Dbu {
    if a == b {
        return raw.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu;
    }
    let num = i128::from(raw) * i128::from(a);
    let den = i128::from(b);
    let half = den / 2;
    let rounded = if num >= 0 {
        (num + half) / den
    } else {
        (num - half) / den
    };
    rounded.clamp(i128::from(Dbu::MIN), i128::from(Dbu::MAX)) as Dbu
}

/// Maps a CIF direction vector `(dx, dy)` to the nearest of the four Manhattan
/// rotations, plus whether the input was already exactly axis-aligned (one
/// component exactly zero, the other nonzero). A zero vector is degenerate and
/// maps to `R0`, reported as inexact so the caller can warn.
fn quadrant_from_vector(dx: i64, dy: i64) -> (Orientation, bool) {
    if dx == 0 && dy == 0 {
        return (Orientation::R0, false);
    }
    let exact = dx == 0 || dy == 0;
    let degrees = (dy as f64).atan2(dx as f64).to_degrees().rem_euclid(360.0);
    let quadrant = (degrees / 90.0).round() as i64 % 4;
    let orientation = match quadrant {
        0 => Orientation::R0,
        1 => Orientation::R90,
        2 => Orientation::R180,
        _ => Orientation::R270,
    };
    (orientation, exact)
}

/// Builds a lowered [`ModelError`] from a static description, the CIF
/// reader's sole error constructor (see [`IoError::Malformed`]).
fn malformed(msg: &'static str) -> ModelError {
    IoError::Malformed(msg).into()
}

/// A bounded accumulator for [`ImportWarning`]s raised during an import.
///
/// Deduplicates by category so a pathological file (say, ten million
/// degenerate polygons) yields one representative warning per category with a
/// running count, not ten million warnings that would themselves be a memory
/// hazard. Mirrors `gds.rs`'s identically named, identically shaped helper;
/// each format module keeps its own copy so the modules stay independently
/// buildable.
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
    use super::{parse_int, quadrant_from_vector, scale_to_dbu, strip_comments};
    use reticle_geometry::Orientation;

    #[test]
    fn strip_comments_removes_parenthesized_text() {
        let out = strip_comments("DS 1 1 1;(a comment)L M1;(another)B 1 1 0 0;")
            .expect("balanced comments strip cleanly");
        assert_eq!(out, "DS 1 1 1; L M1; B 1 1 0 0;");
    }

    #[test]
    fn strip_comments_rejects_unterminated_comment() {
        assert!(strip_comments("DS 1 1 1;(unterminated").is_err());
    }

    #[test]
    fn quadrant_from_vector_is_exact_on_axes() {
        assert_eq!(quadrant_from_vector(1, 0), (Orientation::R0, true));
        assert_eq!(quadrant_from_vector(0, 1), (Orientation::R90, true));
        assert_eq!(quadrant_from_vector(-1, 0), (Orientation::R180, true));
        assert_eq!(quadrant_from_vector(0, -1), (Orientation::R270, true));
    }

    #[test]
    fn quadrant_from_vector_snaps_non_manhattan() {
        // 45 degrees: exactly ambiguous between R0 and R90, rounds to R90.
        let (orientation, exact) = quadrant_from_vector(1, 1);
        assert!(!exact);
        assert_eq!(orientation, Orientation::R90);
        // A shallow angle snaps to the nearer axis.
        let (orientation, exact) = quadrant_from_vector(10, 1);
        assert!(!exact);
        assert_eq!(orientation, Orientation::R0);
    }

    #[test]
    fn quadrant_from_vector_zero_defaults_to_r0_inexact() {
        assert_eq!(quadrant_from_vector(0, 0), (Orientation::R0, false));
    }

    #[test]
    fn scale_to_dbu_identity_and_scaled() {
        assert_eq!(scale_to_dbu(1000, 1, 1), 1000);
        assert_eq!(scale_to_dbu(1000, 1, 2), 500);
        assert_eq!(scale_to_dbu(-1000, 1, 2), -500);
        // Rounds half away from zero.
        assert_eq!(scale_to_dbu(5, 1, 2), 3);
        assert_eq!(scale_to_dbu(-5, 1, 2), -3);
    }

    #[test]
    fn scale_to_dbu_never_overflows_on_extreme_input() {
        // An adversarial huge scale factor must saturate, not panic.
        let v = scale_to_dbu(i64::MAX, i64::MAX, 1);
        assert!(v == i32::MAX || v == i32::MIN);
    }

    #[test]
    fn parse_int_rejects_non_numeric() {
        assert!(parse_int("abc").is_err());
        assert!(parse_int("").is_err());
        assert!(parse_int("12.5").is_err());
        assert_eq!(parse_int("-42").unwrap(), -42);
    }
}

//! Productivity editing: clipboard, duplicate, array, move-by-delta, and via stacks.
//!
//! This module holds the window-free logic behind the productivity side panel, so
//! every geometric transform is unit-tested without a GPU or an egui context. The
//! panel itself (`App::productivity_panel`) is thin glue that reads these helpers and
//! feeds the resulting [`reticle_model::Edit`]s through the undo history.
//!
//! The pieces are:
//!
//! * [`Clipboard`], an in-app store of copied [`DrawShape`]s in world coordinates.
//! * [`translate_shape`] / [`translate_shapes`], the shared offset used by paste,
//!   duplicate, and move-by-delta.
//! * [`array_offsets`] / [`array_shapes`], the rows x columns x pitch expansion the
//!   array tool previews and commits.
//! * [`enclosure_of`] and [`via_stack_shapes`], which read the technology enclosure
//!   rules and build a via cut plus its two layer enclosures sized to satisfy them.
//! * [`ProductivityState`], the panel's field state (array parameters, move deltas,
//!   via-stack layer choices, and live-preview toggles).
//!
//! Nothing here mutates a document directly: the helpers return owned [`DrawShape`]
//! lists and the panel wraps each in an `AddShape` (or `AddArray`) edit so the whole
//! feature is undo-integrated by construction.

use reticle_geometry::{Dbu, LayerId, Path, Point, Polygon, Rect};
use reticle_model::{DrawShape, Rule, RuleKind, ShapeKind, Technology};

/// The largest number of array elements the tool will expand at once.
///
/// A rows x columns product beyond this is refused rather than materialized: the
/// array tool is for tractable, previewable repeats, and huge counts belong in a
/// hierarchical [`reticle_model::ArrayInstance`] placement instead. The panel shows
/// the cap and disables commit past it.
pub const MAX_ARRAY_ELEMENTS: u64 = 100_000;

/// An in-app clipboard of shapes, held in world (top-cell) coordinates.
///
/// Copy and cut both snapshot the selected shapes into here; paste stamps them back
/// with an offset (see [`translate_shapes`]). Storing resolved [`DrawShape`]s (not
/// selection indices) means the clipboard survives edits, undo, and selection
/// changes, matching how a layout editor's clipboard behaves.
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    shapes: Vec<DrawShape>,
}

impl Clipboard {
    /// Creates an empty clipboard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the clipboard contents with `shapes` (a copy or cut).
    pub fn set(&mut self, shapes: Vec<DrawShape>) {
        self.shapes = shapes;
    }

    /// The clipboard's shapes, in the order they were copied.
    #[must_use]
    pub fn shapes(&self) -> &[DrawShape] {
        &self.shapes
    }

    /// The number of shapes on the clipboard.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// Whether the clipboard holds no shapes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Clears the clipboard.
    pub fn clear(&mut self) {
        self.shapes.clear();
    }
}

/// Translates a drawable shape by `(dx, dy)` DBU, preserving its layer and kind.
///
/// This is the one offset primitive behind paste, duplicate, and move-by-delta.
/// Every coordinate goes through [`Point::translate`], which saturates at the DBU
/// range, so a delta that would overflow clamps rather than wraps.
#[must_use]
pub fn translate_shape(shape: &DrawShape, dx: Dbu, dy: Dbu) -> DrawShape {
    let kind = match &shape.kind {
        ShapeKind::Rect(rect) => ShapeKind::Rect(Rect::new(
            rect.min.translate(dx, dy),
            rect.max.translate(dx, dy),
        )),
        ShapeKind::Polygon(poly) => ShapeKind::Polygon(Polygon::new(
            poly.vertices()
                .iter()
                .map(|p| p.translate(dx, dy))
                .collect(),
        )),
        ShapeKind::Path(path) => ShapeKind::Path(Path::new(
            path.points().iter().map(|p| p.translate(dx, dy)).collect(),
            path.width(),
            path.endcap(),
        )),
    };
    DrawShape::new(shape.layer, kind)
}

/// Translates every shape in `shapes` by `(dx, dy)` DBU.
#[must_use]
pub fn translate_shapes(shapes: &[DrawShape], dx: Dbu, dy: Dbu) -> Vec<DrawShape> {
    shapes.iter().map(|s| translate_shape(s, dx, dy)).collect()
}

/// The per-element `(dx, dy)` offsets for a `rows` x `columns` array with the given
/// pitches, in row-major order (all columns of row 0, then row 1, and so on).
///
/// Element `(0, 0)` sits at offset `(0, 0)`; column `c` is `c * column_pitch` in x
/// and row `r` is `r * row_pitch` in y. A zero row or column count yields no
/// offsets. Offsets are computed with widened arithmetic and saturated back into the
/// DBU range so a large pitch times a large index cannot overflow.
#[must_use]
pub fn array_offsets(
    rows: u32,
    columns: u32,
    row_pitch: Dbu,
    column_pitch: Dbu,
) -> Vec<(Dbu, Dbu)> {
    let mut offsets = Vec::new();
    for r in 0..rows {
        let dy = saturating_mul_index(row_pitch, r);
        for c in 0..columns {
            let dx = saturating_mul_index(column_pitch, c);
            offsets.push((dx, dy));
        }
    }
    offsets
}

/// The total number of elements a `rows` x `columns` array expands to, as a
/// [`u64`] so the [`MAX_ARRAY_ELEMENTS`] guard never itself overflows.
#[must_use]
pub fn array_element_count(rows: u32, columns: u32) -> u64 {
    u64::from(rows) * u64::from(columns)
}

/// Expands `shapes` into a `rows` x `columns` array with the given pitches.
///
/// Every source shape is stamped once per array element at that element's offset
/// (see [`array_offsets`]), so the result has `shapes.len() * rows * columns`
/// entries in element-major then shape order. Element `(0, 0)` reproduces the input
/// unchanged. Returns an empty vector when either count is zero.
#[must_use]
pub fn array_shapes(
    shapes: &[DrawShape],
    rows: u32,
    columns: u32,
    row_pitch: Dbu,
    column_pitch: Dbu,
) -> Vec<DrawShape> {
    let mut out = Vec::new();
    for (dx, dy) in array_offsets(rows, columns, row_pitch, column_pitch) {
        for shape in shapes {
            out.push(translate_shape(shape, dx, dy));
        }
    }
    out
}

/// The center point of a rectangle, rounding toward zero on an odd span.
#[must_use]
pub fn rect_center(rect: Rect) -> Point {
    let cx = i64::midpoint(i64::from(rect.min.x), i64::from(rect.max.x));
    let cy = i64::midpoint(i64::from(rect.min.y), i64::from(rect.max.y));
    Point::new(clamp_dbu(cx), clamp_dbu(cy))
}

/// The minimum enclosure of `inner_layer` by `outer_layer` required by `rules`, in
/// DBU, or `None` if the technology declares no such enclosure rule.
///
/// The match is an [`RuleKind::Enclosure`] rule whose primary `layer` is
/// `outer_layer` (the enclosing layer) and whose `other_layer` is `inner_layer` (the
/// enclosed layer), matching the rule vocabulary in [`reticle_model::Rule`]. When
/// several such rules exist the largest value wins, so the built geometry satisfies
/// all of them at once.
#[must_use]
pub fn enclosure_of(rules: &[Rule], outer_layer: LayerId, inner_layer: LayerId) -> Option<Dbu> {
    rules
        .iter()
        .filter(|rule| {
            rule.kind == RuleKind::Enclosure
                && rule.layer == outer_layer
                && rule.other_layer == Some(inner_layer)
        })
        .map(|rule| clamp_dbu(rule.value))
        .max()
}

/// A resolved via-stack recipe: the cut rectangle plus the two enclosure rectangles
/// that will be committed, all in world coordinates.
///
/// Building the shapes into an owned struct (rather than mutating a document) keeps
/// [`via_stack_shapes`] pure and unit-testable; the panel turns each rectangle into
/// an `AddShape` edit so the whole placement lands as undoable history.
///
/// [`DrawShape`] is only [`PartialEq`], not [`Eq`] (its geometry compares
/// structurally), so this derives `PartialEq` alone.
#[derive(Clone, PartialEq, Debug)]
pub struct ViaStack {
    /// The cut/via square on the cut layer.
    pub cut: DrawShape,
    /// The enclosure rectangle on the lower picked layer.
    pub lower_enclosure: DrawShape,
    /// The enclosure rectangle on the upper picked layer.
    pub upper_enclosure: DrawShape,
}

impl ViaStack {
    /// The three shapes in commit order: cut, lower enclosure, upper enclosure.
    #[must_use]
    pub fn shapes(&self) -> [&DrawShape; 3] {
        [&self.cut, &self.lower_enclosure, &self.upper_enclosure]
    }

    /// The three shapes as an owned vector, for feeding into edits.
    #[must_use]
    pub fn into_shapes(self) -> Vec<DrawShape> {
        vec![self.cut, self.lower_enclosure, self.upper_enclosure]
    }
}

/// Builds a via stack centered at `center`: a square cut of side `cut_size` on
/// `cut_layer`, enclosed on `lower_layer` and `upper_layer` by the enclosure margins
/// the technology requires.
///
/// The enclosure margin for each picked layer is looked up with [`enclosure_of`]
/// (that layer enclosing the cut layer); a layer with no matching rule falls back to
/// `default_enclosure`, so the tool still produces a sane stack against a technology
/// that omits the rule. Each enclosure rectangle is the cut expanded outward by its
/// margin, so it overlaps the cut on every side by at least the required amount.
///
/// Returns `None` when `cut_size` is not positive: a non-existent cut has no
/// meaningful enclosure.
#[must_use]
pub fn via_stack_shapes(
    tech: &Technology,
    lower_layer: LayerId,
    upper_layer: LayerId,
    cut_layer: LayerId,
    center: Point,
    cut_size: Dbu,
    default_enclosure: Dbu,
) -> Option<ViaStack> {
    if cut_size <= 0 {
        return None;
    }
    let half = cut_size / 2;
    // Center the cut on `center`; an odd size rounds the far corner up by one DBU so
    // the drawn size is never smaller than requested.
    let cut_rect = Rect::new(
        Point::new(center.x.saturating_sub(half), center.y.saturating_sub(half)),
        Point::new(
            center.x.saturating_add(cut_size - half),
            center.y.saturating_add(cut_size - half),
        ),
    );

    let lower_margin =
        enclosure_of(&tech.rules, lower_layer, cut_layer).unwrap_or(default_enclosure);
    let upper_margin =
        enclosure_of(&tech.rules, upper_layer, cut_layer).unwrap_or(default_enclosure);

    let cut = DrawShape::new(cut_layer, ShapeKind::Rect(cut_rect));
    let lower_enclosure = DrawShape::new(
        lower_layer,
        ShapeKind::Rect(cut_rect.expanded(lower_margin.max(0))),
    );
    let upper_enclosure = DrawShape::new(
        upper_layer,
        ShapeKind::Rect(cut_rect.expanded(upper_margin.max(0))),
    );
    Some(ViaStack {
        cut,
        lower_enclosure,
        upper_enclosure,
    })
}

/// Multiplies a pitch by a zero-based index with widened arithmetic, saturating into
/// the DBU range so a large array never overflows a coordinate.
fn saturating_mul_index(pitch: Dbu, index: u32) -> Dbu {
    let product = i64::from(pitch) * i64::from(index);
    clamp_dbu(product)
}

/// Clamps a widened value into the [`Dbu`] range.
fn clamp_dbu(v: i64) -> Dbu {
    v.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// The panel's editable field state: array parameters, move deltas, and via-stack
/// choices, plus the live-preview toggles.
///
/// The fields are plain values the egui panel binds its widgets to; the panel reads
/// them, calls the pure helpers above, and applies the resulting edits. Defaults are
/// a small, immediately-usable 2 x 2 array and a zero move, so the panel is
/// functional the moment it opens.
#[derive(Clone, Debug)]
pub struct ProductivityState {
    /// The in-app clipboard (copy/cut source for paste).
    pub clipboard: Clipboard,

    /// Paste/duplicate offset in x (DBU).
    pub paste_dx: Dbu,
    /// Paste/duplicate offset in y (DBU).
    pub paste_dy: Dbu,

    /// Array row count (y repetitions).
    pub array_rows: u32,
    /// Array column count (x repetitions).
    pub array_cols: u32,
    /// Array row pitch (DBU).
    pub array_row_pitch: Dbu,
    /// Array column pitch (DBU).
    pub array_col_pitch: Dbu,
    /// Whether the array live preview is drawn before commit.
    pub array_preview: bool,

    /// Move-by-delta x component (DBU), applied to the selection.
    pub move_dx: Dbu,
    /// Move-by-delta y component (DBU), applied to the selection.
    pub move_dy: Dbu,

    /// The lower picked layer for the via-stack builder.
    pub via_lower: LayerId,
    /// The upper picked layer for the via-stack builder.
    pub via_upper: LayerId,
    /// The cut/via layer for the via-stack builder.
    pub via_cut: LayerId,
    /// The cut square side length (DBU).
    pub via_cut_size: Dbu,
    /// Fallback enclosure margin (DBU) when the technology declares no rule.
    pub via_default_enclosure: Dbu,
    /// Via-stack center x (DBU).
    pub via_center_x: Dbu,
    /// Via-stack center y (DBU).
    pub via_center_y: Dbu,
}

impl Default for ProductivityState {
    fn default() -> Self {
        Self {
            clipboard: Clipboard::new(),
            paste_dx: 200,
            paste_dy: 200,
            array_rows: 2,
            array_cols: 2,
            array_row_pitch: 1000,
            array_col_pitch: 1000,
            array_preview: true,
            move_dx: 0,
            move_dy: 0,
            via_lower: LayerId::new(4, 0),
            via_upper: LayerId::new(5, 0),
            via_cut: LayerId::new(7, 0),
            via_cut_size: 200,
            via_default_enclosure: 50,
            via_center_x: 0,
            via_center_y: 0,
        }
    }
}

impl ProductivityState {
    /// Creates the default panel state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of elements the current array parameters would expand to.
    #[must_use]
    pub fn array_count(&self) -> u64 {
        array_element_count(self.array_rows, self.array_cols)
    }

    /// Whether the current array parameters are within the [`MAX_ARRAY_ELEMENTS`]
    /// cap and produce at least one element.
    #[must_use]
    pub fn array_is_committable(&self) -> bool {
        let count = self.array_count();
        count > 0 && count <= MAX_ARRAY_ELEMENTS
    }

    /// Expands `shapes` per the current array parameters, for preview or commit.
    #[must_use]
    pub fn array_expand(&self, shapes: &[DrawShape]) -> Vec<DrawShape> {
        array_shapes(
            shapes,
            self.array_rows,
            self.array_cols,
            self.array_row_pitch,
            self.array_col_pitch,
        )
    }

    /// Builds the via stack for the current picks and center, or `None` if the cut
    /// size is not positive.
    #[must_use]
    pub fn build_via_stack(&self, tech: &Technology) -> Option<ViaStack> {
        via_stack_shapes(
            tech,
            self.via_lower,
            self.via_upper,
            self.via_cut,
            Point::new(self.via_center_x, self.via_center_y),
            self.via_cut_size,
            self.via_default_enclosure,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Endcap;
    use reticle_model::LayerInfo;

    const M1: LayerId = LayerId::new(4, 0);
    const M2: LayerId = LayerId::new(5, 0);
    const CUT: LayerId = LayerId::new(7, 0);

    fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn rect_of(shape: &DrawShape) -> Rect {
        match &shape.kind {
            ShapeKind::Rect(r) => *r,
            other => panic!("expected a rect, got {other:?}"),
        }
    }

    #[test]
    fn translate_shape_moves_rect_corners() {
        let s = rect_shape(M1, 0, 0, 100, 50);
        let moved = translate_shape(&s, 30, -20);
        let r = rect_of(&moved);
        assert_eq!(r.min, Point::new(30, -20));
        assert_eq!(r.max, Point::new(130, 30));
        assert_eq!(moved.layer, M1);
    }

    #[test]
    fn translate_shape_preserves_path_width_and_cap() {
        let path = DrawShape::new(
            M2,
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(100, 0)],
                40,
                Endcap::Square,
            )),
        );
        let moved = translate_shape(&path, 5, 7);
        match &moved.kind {
            ShapeKind::Path(p) => {
                assert_eq!(p.points()[0], Point::new(5, 7));
                assert_eq!(p.points()[1], Point::new(105, 7));
                assert_eq!(p.width(), 40);
                assert_eq!(p.endcap(), Endcap::Square);
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn translate_shape_moves_polygon_vertices() {
        let poly = DrawShape::new(
            M1,
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(10, 0),
                Point::new(10, 10),
            ])),
        );
        let moved = translate_shape(&poly, -3, 4);
        match &moved.kind {
            ShapeKind::Polygon(p) => {
                assert_eq!(
                    p.vertices(),
                    &[Point::new(-3, 4), Point::new(7, 4), Point::new(7, 14)]
                );
            }
            other => panic!("expected a polygon, got {other:?}"),
        }
    }

    #[test]
    fn paste_offset_shifts_every_shape_by_the_same_delta() {
        let shapes = vec![
            rect_shape(M1, 0, 0, 10, 10),
            rect_shape(M2, 100, 100, 120, 140),
        ];
        let pasted = translate_shapes(&shapes, 200, 200);
        assert_eq!(pasted.len(), 2);
        assert_eq!(rect_of(&pasted[0]).min, Point::new(200, 200));
        assert_eq!(rect_of(&pasted[0]).max, Point::new(210, 210));
        assert_eq!(rect_of(&pasted[1]).min, Point::new(300, 300));
        assert_eq!(rect_of(&pasted[1]).max, Point::new(320, 340));
        // Layers are preserved through the paste.
        assert_eq!(pasted[0].layer, M1);
        assert_eq!(pasted[1].layer, M2);
    }

    #[test]
    fn clipboard_round_trips_shapes() {
        let mut clip = Clipboard::new();
        assert!(clip.is_empty());
        clip.set(vec![rect_shape(M1, 0, 0, 10, 10)]);
        assert_eq!(clip.len(), 1);
        assert!(!clip.is_empty());
        clip.clear();
        assert!(clip.is_empty());
    }

    #[test]
    fn array_offsets_are_row_major_with_pitch() {
        let offsets = array_offsets(2, 3, 1000, 500);
        assert_eq!(
            offsets,
            vec![
                (0, 0),
                (500, 0),
                (1000, 0),
                (0, 1000),
                (500, 1000),
                (1000, 1000),
            ]
        );
    }

    #[test]
    fn array_offsets_empty_when_a_dimension_is_zero() {
        assert!(array_offsets(0, 5, 100, 100).is_empty());
        assert!(array_offsets(5, 0, 100, 100).is_empty());
    }

    #[test]
    fn array_shapes_count_is_shapes_times_rows_times_cols() {
        let src = vec![rect_shape(M1, 0, 0, 100, 100), rect_shape(M2, 0, 0, 50, 50)];
        let rows = 3;
        let cols = 4;
        let arrayed = array_shapes(&src, rows, cols, 2000, 1500);
        assert_eq!(arrayed.len(), src.len() * rows as usize * cols as usize);
    }

    #[test]
    fn array_shapes_place_elements_at_expected_positions() {
        let src = vec![rect_shape(M1, 0, 0, 100, 100)];
        // 2 rows x 2 cols, pitch 1000 x 1000: four rects at the grid corners.
        let arrayed = array_shapes(&src, 2, 2, 1000, 1000);
        let mins: Vec<Point> = arrayed.iter().map(|s| rect_of(s).min).collect();
        assert_eq!(
            mins,
            vec![
                Point::new(0, 0),
                Point::new(1000, 0),
                Point::new(0, 1000),
                Point::new(1000, 1000),
            ]
        );
    }

    #[test]
    fn array_element_count_and_guard() {
        assert_eq!(array_element_count(10, 10), 100);
        let mut state = ProductivityState::new();
        state.array_rows = 400;
        state.array_cols = 400; // 160_000 > MAX_ARRAY_ELEMENTS
        assert_eq!(state.array_count(), 160_000);
        assert!(!state.array_is_committable());
        state.array_rows = 10;
        state.array_cols = 10;
        assert!(state.array_is_committable());
        state.array_rows = 0;
        assert!(!state.array_is_committable());
    }

    #[test]
    fn array_offsets_saturate_instead_of_overflowing() {
        // A large pitch times a large index would overflow i32 if multiplied
        // narrow; the widened, clamped math must saturate at Dbu::MAX.
        let offsets = array_offsets(1, 3, Dbu::MAX, 0);
        // Row pitch is 0, so all dy are 0; column pitch drives dx here instead.
        assert_eq!(offsets.len(), 3);
        let offsets = array_offsets(3, 1, Dbu::MAX, 0);
        assert_eq!(offsets[2].1, Dbu::MAX); // 2 * Dbu::MAX clamps to Dbu::MAX
    }

    fn tech_with_enclosure(outer: LayerId, inner: LayerId, value: i64) -> Technology {
        Technology {
            name: "test".to_owned(),
            dbu_per_micron: 1000,
            layers: vec![
                LayerInfo {
                    id: outer,
                    name: "outer".to_owned(),
                    color_rgba: 0,
                    visible: true,
                },
                LayerInfo {
                    id: inner,
                    name: "inner".to_owned(),
                    color_rgba: 0,
                    visible: true,
                },
            ],
            rules: vec![Rule {
                name: "via.enc".to_owned(),
                kind: RuleKind::Enclosure,
                layer: outer,
                other_layer: Some(inner),
                value,
            }],
            stack: Vec::new(),
        }
    }

    #[test]
    fn enclosure_of_finds_matching_rule() {
        let tech = tech_with_enclosure(M1, CUT, 60);
        assert_eq!(enclosure_of(&tech.rules, M1, CUT), Some(60));
        // Wrong direction (inner/outer swapped) does not match.
        assert_eq!(enclosure_of(&tech.rules, CUT, M1), None);
        // A layer with no rule returns None.
        assert_eq!(enclosure_of(&tech.rules, M2, CUT), None);
    }

    #[test]
    fn enclosure_of_takes_the_largest_when_several_apply() {
        let mut tech = tech_with_enclosure(M1, CUT, 40);
        tech.rules.push(Rule {
            name: "via.enc.wide".to_owned(),
            kind: RuleKind::Enclosure,
            layer: M1,
            other_layer: Some(CUT),
            value: 90,
        });
        assert_eq!(enclosure_of(&tech.rules, M1, CUT), Some(90));
    }

    #[test]
    fn via_stack_sizes_enclosures_from_rules() {
        // METAL1 encloses the cut by 60, METAL2 by 100.
        let mut tech = tech_with_enclosure(M1, CUT, 60);
        tech.rules.push(Rule {
            name: "m2.enc".to_owned(),
            kind: RuleKind::Enclosure,
            layer: M2,
            other_layer: Some(CUT),
            value: 100,
        });
        // Even-sized cut centered at the origin: 200 x 200 spanning [-100, 100].
        let stack = via_stack_shapes(&tech, M1, M2, CUT, Point::ORIGIN, 200, 10)
            .expect("positive cut size yields a stack");

        let cut = rect_of(&stack.cut);
        assert_eq!(cut.min, Point::new(-100, -100));
        assert_eq!(cut.max, Point::new(100, 100));
        assert_eq!(stack.cut.layer, CUT);

        // Lower enclosure: cut expanded by 60 on every side.
        let lower = rect_of(&stack.lower_enclosure);
        assert_eq!(lower.min, Point::new(-160, -160));
        assert_eq!(lower.max, Point::new(160, 160));
        assert_eq!(stack.lower_enclosure.layer, M1);

        // Upper enclosure: cut expanded by 100 on every side.
        let upper = rect_of(&stack.upper_enclosure);
        assert_eq!(upper.min, Point::new(-200, -200));
        assert_eq!(upper.max, Point::new(200, 200));
        assert_eq!(stack.upper_enclosure.layer, M2);
    }

    #[test]
    fn via_stack_falls_back_to_default_enclosure_without_a_rule() {
        // A technology with no enclosure rules at all.
        let tech = Technology {
            name: "bare".to_owned(),
            dbu_per_micron: 1000,
            layers: Vec::new(),
            rules: Vec::new(),
            stack: Vec::new(),
        };
        let stack = via_stack_shapes(&tech, M1, M2, CUT, Point::ORIGIN, 100, 25)
            .expect("positive cut size yields a stack");
        let cut = rect_of(&stack.cut);
        let lower = rect_of(&stack.lower_enclosure);
        // Both enclosures use the 25 DBU fallback margin.
        assert_eq!(lower.min.x, cut.min.x - 25);
        assert_eq!(lower.max.x, cut.max.x + 25);
        assert_eq!(rect_of(&stack.upper_enclosure).max.y, cut.max.y + 25);
    }

    #[test]
    fn via_stack_rejects_nonpositive_cut() {
        let tech = tech_with_enclosure(M1, CUT, 60);
        assert!(via_stack_shapes(&tech, M1, M2, CUT, Point::ORIGIN, 0, 10).is_none());
        assert!(via_stack_shapes(&tech, M1, M2, CUT, Point::ORIGIN, -50, 10).is_none());
    }

    #[test]
    fn via_stack_into_shapes_is_cut_then_enclosures() {
        let tech = tech_with_enclosure(M1, CUT, 60);
        let stack = via_stack_shapes(&tech, M1, M2, CUT, Point::ORIGIN, 200, 10).unwrap();
        let shapes = stack.into_shapes();
        assert_eq!(shapes.len(), 3);
        assert_eq!(shapes[0].layer, CUT);
        assert_eq!(shapes[1].layer, M1);
        assert_eq!(shapes[2].layer, M2);
    }

    #[test]
    fn rect_center_is_the_midpoint() {
        assert_eq!(
            rect_center(Rect::new(Point::new(0, 0), Point::new(100, 200))),
            Point::new(50, 100)
        );
        // Odd span rounds toward zero.
        assert_eq!(
            rect_center(Rect::new(Point::new(0, 0), Point::new(5, 5))),
            Point::new(2, 2)
        );
    }

    #[test]
    fn move_delta_uses_the_shared_translate() {
        // Move-by-delta is exactly translate_shapes with the panel's dx/dy.
        let sel = vec![rect_shape(M1, 10, 10, 20, 20)];
        let moved = translate_shapes(&sel, -5, 15);
        assert_eq!(rect_of(&moved[0]).min, Point::new(5, 25));
        assert_eq!(rect_of(&moved[0]).max, Point::new(15, 35));
    }

    #[test]
    fn default_state_is_a_committable_small_array() {
        let state = ProductivityState::new();
        assert_eq!(state.array_rows, 2);
        assert_eq!(state.array_cols, 2);
        assert!(state.array_is_committable());
        assert!(state.clipboard.is_empty());
    }
}

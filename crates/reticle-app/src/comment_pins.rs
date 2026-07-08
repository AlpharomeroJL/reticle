//! Anchored comment pins: the panel state and its window-free logic.
//!
//! A [`CommentPins`] holds the layout's [`Comment`]s so the
//! side panel can list them and the canvas can drop a numbered pin at each one's
//! anchor. As in [`crate::drc_panel`], everything interesting here, resolving an
//! anchor to a world point and formatting a comment for the list, is a plain
//! function tested without an egui context; the app module owns only the thin
//! painting and click wiring.
//!
//! Comments are persisted through the schema-V2 `Document.comments` field
//! (ADR 0080); this module is the in-app view over that set. An `anchor_ref` binds
//! a comment to a cell (or a `cell/element-id` path); the pin lands at the centre
//! of that cell's own geometry.

use reticle_geometry::Point;
use reticle_model::Document;
use reticle_sync::Comment;

/// The comment-pins panel state: the anchored comments and the current selection.
///
/// The list is empty until [`CommentPins::add`] is called. A `selected` index
/// tracks which comment the user last clicked so the canvas can emphasize its pin.
#[derive(Clone, Debug, Default)]
pub struct CommentPins {
    /// The layout's comments, in insertion order (which is also list order).
    comments: Vec<Comment>,
    /// The index of the comment the user last selected, if any.
    selected: Option<usize>,
}

impl CommentPins {
    /// Creates an empty pin set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the stored comments (for example after loading a V2 document),
    /// dropping any selection since indices into the old list are stale.
    pub fn set_comments(&mut self, comments: Vec<Comment>) {
        self.comments = comments;
        self.selected = None;
    }

    /// Appends a comment, returning the index it was stored at.
    pub fn add(&mut self, comment: Comment) -> usize {
        self.comments.push(comment);
        self.comments.len() - 1
    }

    /// The stored comments.
    #[must_use]
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The number of stored comments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.comments.len()
    }

    /// Whether there are no stored comments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.comments.is_empty()
    }

    /// Clears the stored comments and the selection.
    pub fn clear(&mut self) {
        self.comments.clear();
        self.selected = None;
    }

    /// The index of the currently selected comment, if any.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Records `index` as the selected comment, ignoring an out-of-range index so
    /// a stale click after the list shrank cannot select a missing comment.
    pub fn select(&mut self, index: usize) {
        if index < self.comments.len() {
            self.selected = Some(index);
        }
    }
}

/// The anchor cell name in an `anchor_ref`: everything before the first `/`, so
/// both a bare cell name (`TOP`) and a `cell/element-id` path (`TOP/shape-3`)
/// resolve to the same cell. An empty `anchor_ref` yields an empty name.
#[must_use]
pub fn anchor_cell(anchor_ref: &str) -> &str {
    anchor_ref.split('/').next().unwrap_or("")
}

/// Resolves a comment's `anchor_ref` to a world point for its pin: the centre of
/// the anchored cell's own (non-instanced) geometry.
///
/// Returns `None` if the anchor names no cell in `doc`, or that cell has no direct
/// shapes to centre on (an instance-only cell), so the caller can skip painting a
/// pin that would otherwise float at the origin.
#[must_use]
pub fn anchor_point(doc: &Document, anchor_ref: &str) -> Option<Point> {
    let cell = doc.cell(anchor_cell(anchor_ref))?;
    let bbox = cell.shapes_bbox()?;
    // `i32::midpoint` averages without the intermediate overflow of `(a + b) / 2`.
    Some(Point::new(
        i32::midpoint(bbox.min.x, bbox.max.x),
        i32::midpoint(bbox.min.y, bbox.max.y),
    ))
}

/// Formats one comment into a single side-panel line: author, anchor, and a
/// single-line preview of the body (newlines collapsed to spaces).
#[must_use]
pub fn format_comment_line(comment: &Comment) -> String {
    let body = comment.body.replace('\n', " ");
    format!("{} @{}  {}", comment.author, comment.anchor_ref, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    /// A document with a `TOP` cell holding a single 0..100 x 0..40 metal rect, so
    /// its geometry centre is a known point.
    fn doc_with_top() -> Document {
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            LayerId::new(4, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 40))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    fn comment(anchor: &str) -> Comment {
        Comment::root("c1", anchor, "alice", "note", 1)
    }

    #[test]
    fn anchor_cell_strips_the_element_path() {
        assert_eq!(anchor_cell("TOP"), "TOP");
        assert_eq!(anchor_cell("TOP/shape-3"), "TOP");
        assert_eq!(anchor_cell("A/b/c"), "A");
        assert_eq!(anchor_cell(""), "");
    }

    #[test]
    fn anchor_point_is_the_cell_geometry_centre() {
        let doc = doc_with_top();
        let p = anchor_point(&doc, "TOP/shape-3").expect("TOP has geometry");
        // Centre of (0,0)-(100,40).
        assert_eq!(p, Point::new(50, 20));
    }

    #[test]
    fn anchor_point_is_none_for_unknown_or_empty_cell() {
        let doc = doc_with_top();
        assert!(anchor_point(&doc, "MISSING").is_none());
        assert!(anchor_point(&doc, "").is_none());
    }

    #[test]
    fn anchor_point_is_none_for_a_cell_without_direct_geometry() {
        let mut doc = Document::new();
        doc.insert_cell(Cell::new("EMPTY"));
        assert!(anchor_point(&doc, "EMPTY").is_none());
    }

    #[test]
    fn add_and_select_track_the_comment_set() {
        let mut pins = CommentPins::new();
        assert!(pins.is_empty());
        let i = pins.add(comment("TOP"));
        assert_eq!(i, 0);
        assert_eq!(pins.len(), 1);

        pins.select(0);
        assert_eq!(pins.selected(), Some(0));
        // Out-of-range selection is ignored.
        pins.select(99);
        assert_eq!(pins.selected(), Some(0));
    }

    #[test]
    fn set_comments_replaces_and_drops_selection() {
        let mut pins = CommentPins::new();
        pins.add(comment("TOP"));
        pins.select(0);
        pins.set_comments(vec![comment("A"), comment("B")]);
        assert_eq!(pins.len(), 2);
        assert!(pins.selected().is_none(), "stale selection dropped");
    }

    #[test]
    fn clear_empties_the_set() {
        let mut pins = CommentPins::new();
        pins.add(comment("TOP"));
        pins.select(0);
        pins.clear();
        assert!(pins.is_empty());
        assert!(pins.selected().is_none());
    }

    #[test]
    fn format_line_carries_author_anchor_and_single_line_body() {
        let mut c = comment("TOP/shape-3");
        c.body = "line one\nline two".to_owned();
        let line = format_comment_line(&c);
        assert!(line.contains("alice"));
        assert!(line.contains("@TOP/shape-3"));
        assert!(line.contains("line one line two"), "newlines collapsed");
        assert!(!line.contains('\n'));
    }
}

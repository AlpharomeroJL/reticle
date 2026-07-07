//! The document host: an edited in-RAM document, or a read-only streamed scene.
//!
//! An open document in Reticle is one of two very different things: a small layout the
//! user *edits* (an in-RAM [`History`] with undo/redo), or a multi-gigabyte die
//! *streamed* over a [`TileSource`](reticle_index::TileSource) and only ever browsed
//! ([`StreamedScene`], ADR 0062). [`DocHost`] is the one type that holds either, and it
//! is the place the read-mostly scope line is drawn: structurally, not by a runtime
//! flag.
//!
//! # Editing a streamed document is a compile error
//!
//! Every mutating path in the app takes `&mut `[`History`]: [`History::apply`],
//! [`History::apply_group`], undo/redo. [`DocHost`] hands out a [`History`] **only** by
//! matching its [`Edited`](DocHost::Edited) arm: there is no total accessor that yields
//! a `&mut History` regardless of arm, and [`StreamedScene`] deliberately exposes no
//! mutation API at all. So editing code that does not first destructure `Edited` cannot
//! name a `History` to mutate, and code that calls a mutator on the result of
//! [`history_mut`](DocHost::history_mut) (an [`Option`]) does not compile until it
//! handles the streamed case. The read-mostly guarantee is therefore checked by the
//! type system at build time, exactly as ADR 0062 requires, rather than by a runtime
//! `if is_streamed { refuse }`.
//!
//! Browse, measure, query, and share work on both arms; they only read.

use crate::history::History;
use crate::streamed::StreamedScene;

/// The document currently open in the app: an edited in-RAM document, or a read-only
/// streamed scene.
///
/// See the [module docs](crate::dochost) for why mutation is only reachable through the
/// [`Edited`](DocHost::Edited) arm.
#[derive(Debug)]
pub enum DocHost {
    /// An in-RAM document the user edits, with its undo/redo [`History`]. The only arm
    /// from which a mutable [`History`] can be obtained.
    Edited(History),
    /// A document streamed from an `.rtla` archive, browsed but never edited.
    Streamed(StreamedScene),
}

impl Default for DocHost {
    /// A fresh, empty edited document: the app's starting state.
    fn default() -> Self {
        DocHost::Edited(History::default())
    }
}

impl DocHost {
    /// Wraps an editing [`History`] as an edited host.
    #[must_use]
    pub fn edited(history: History) -> Self {
        DocHost::Edited(history)
    }

    /// Wraps a [`StreamedScene`] as a read-only streamed host.
    #[must_use]
    pub fn streamed(scene: StreamedScene) -> Self {
        DocHost::Streamed(scene)
    }

    /// Whether the open document is a read-only streamed scene.
    #[must_use]
    pub fn is_streamed(&self) -> bool {
        matches!(self, DocHost::Streamed(_))
    }

    /// Whether the open document is an editable in-RAM document.
    #[must_use]
    pub fn is_edited(&self) -> bool {
        matches!(self, DocHost::Edited(_))
    }

    /// The editing [`History`] for reading, or `None` for a streamed document.
    ///
    /// Read-only borrow, safe on either arm; it just yields nothing for a streamed
    /// document, which has no `History`.
    #[must_use]
    pub fn history(&self) -> Option<&History> {
        match self {
            DocHost::Edited(history) => Some(history),
            DocHost::Streamed(_) => None,
        }
    }

    /// The editing [`History`] for mutation, or `None` for a streamed document.
    ///
    /// This is the *only* mutable-`History` accessor, and it is fallible: a caller that
    /// wants to edit must handle the streamed (`None`) case, so a mutating call on a
    /// streamed document is a compile-time obligation (the `Option` has no `apply`),
    /// never a silent no-op. There is intentionally no infallible
    /// `fn history_mut(&mut self) -> &mut History`.
    #[must_use]
    pub fn history_mut(&mut self) -> Option<&mut History> {
        match self {
            DocHost::Edited(history) => Some(history),
            DocHost::Streamed(_) => None,
        }
    }

    /// The streamed scene for reading, or `None` for an edited document.
    #[must_use]
    pub fn scene(&self) -> Option<&StreamedScene> {
        match self {
            DocHost::Streamed(scene) => Some(scene),
            DocHost::Edited(_) => None,
        }
    }

    /// The streamed scene for driving residency (fetch/insert/evict resident tiles), or
    /// `None` for an edited document.
    ///
    /// A streamed scene is mutable for *residency* (adopting fetched tiles, evicting
    /// old ones), which is not a document edit: it changes what is paged into RAM, never
    /// the silicon. The document stays read-only; see [`StreamedScene`], which has no
    /// edit API.
    #[must_use]
    pub fn scene_mut(&mut self) -> Option<&mut StreamedScene> {
        match self {
            DocHost::Streamed(scene) => Some(scene),
            DocHost::Edited(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_index::streaming::ArchivableRect;
    use reticle_index::{LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader};
    use reticle_model::{DrawShape, Edit, ShapeKind};

    fn streamed_host() -> DocHost {
        let header = RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(Rect::new(Point::new(0, 0), Point::new(1000, 1000))),
            dbu_per_micron: 1000,
            levels: vec![
                LevelDims { cols: 1, rows: 1 },
                LevelDims { cols: 2, rows: 2 },
            ],
        };
        DocHost::streamed(StreamedScene::new(header, 16).unwrap())
    }

    #[test]
    fn edited_arm_yields_a_mutable_history_and_applies_edits() {
        let mut host = DocHost::edited(History::new(crate::demo::demo_document()));
        assert!(host.is_edited());
        let history = host.history_mut().expect("edited host has a history");
        let before = history
            .document()
            .cell(crate::demo::TOP_CELL)
            .unwrap()
            .shapes
            .len();
        history
            .apply(Edit::AddShape {
                cell: crate::demo::TOP_CELL.to_owned(),
                shape: DrawShape::new(
                    LayerId::new(4, 0),
                    ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
                ),
            })
            .unwrap();
        assert_eq!(
            host.history()
                .unwrap()
                .document()
                .cell(crate::demo::TOP_CELL)
                .unwrap()
                .shapes
                .len(),
            before + 1
        );
    }

    #[test]
    fn streamed_arm_exposes_no_history_so_editing_cannot_be_expressed() {
        let mut host = streamed_host();
        assert!(host.is_streamed());
        // The only mutable-History accessor returns None: there is no `&mut History` to
        // hand an editing tool, which is the compile-time read-mostly guarantee at work.
        assert!(host.history_mut().is_none());
        assert!(host.history().is_none());
        // A streamed host does expose its scene for browsing and residency.
        assert!(host.scene().is_some());
        assert!(host.scene_mut().is_some());
    }

    #[test]
    fn edited_arm_exposes_no_scene() {
        let host = DocHost::default();
        assert!(host.is_edited());
        assert!(host.scene().is_none());
    }
}

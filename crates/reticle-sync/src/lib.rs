//! Real-time collaboration for Reticle.
//!
//! Wave 3 wraps a [`yrs`] document over the hierarchical model (ADR 0007), encodes
//! and decodes updates, and manages presence (cursor/selection/viewport) and
//! threaded comments over a WebSocket. Offline edits reconcile on reconnect.
//!
//! The Wave 0 contract is [`SyncDocument`], the local mirror that Wave 3 backs
//! with a CRDT.
//!
//! # How it works
//!
//! A [`SyncDocument`] owns a single [`yrs::Doc`]. The hierarchical model is mapped
//! onto `yrs` shared types (see the `mapping` module): cells live in a
//! root map keyed by name, and each cell's shapes, instances, and arrays live in
//! nested maps keyed by a globally-unique element id. Local edits mutate the
//! `yrs` doc inside a transaction, which produces a binary CRDT update; peers
//! exchange those updates with [`SyncDocument::encode_update`] /
//! [`SyncDocument::apply_update`] and converge to an identical [`Document`],
//! regardless of the order the updates arrive in.
//!
//! ```
//! use reticle_sync::SyncDocument;
//! use reticle_model::Cell;
//!
//! let mut a = SyncDocument::new("alice");
//! let mut b = SyncDocument::new("bob");
//!
//! // Concurrent, independent edits on two peers.
//! a.add_cell(&Cell::new("top"));
//! b.add_cell(&Cell::new("sub"));
//!
//! // Exchange updates both ways.
//! let from_a = a.encode_state_update();
//! let from_b = b.encode_state_update();
//! a.apply_update(&from_b).unwrap();
//! b.apply_update(&from_a).unwrap();
//!
//! // Both peers now see both cells.
//! assert!(a.document().cell("top").is_some());
//! assert!(a.document().cell("sub").is_some());
//! assert_eq!(a.document(), b.document());
//! ```

mod comment;
mod error;
mod frame;
mod mapping;
mod presence;

pub use comment::{Comment, CommentThread};
pub use error::{Result, SyncError};
pub use frame::{
    Frame, decode_frame, encode_presence_frame, encode_update_frame, encode_update_frame_for,
};
pub use presence::{Awareness, Presence};

// `StepEdit` is defined further down in this module and re-exported here so the one
// grouped-edit type has a stable path alongside the rest of the public surface.

use reticle_geometry::LayerId;
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, Instance};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

/// A collaboratively-edited document, backed by a [`yrs`] CRDT.
///
/// Local mutations go through the `add_*` / `remove_cell` methods, which write to
/// the underlying `yrs` document in a transaction. [`SyncDocument::document`]
/// returns a materialized [`Document`] view that is refreshed after every local
/// or remote change.
#[derive(Debug)]
pub struct SyncDocument {
    doc: Doc,
    actor: String,
    /// Monotonic counter making per-actor element ids unique.
    counter: u64,
    /// Cached materialization of the CRDT, refreshed on every change.
    cache: Document,
    /// In-memory presence map for remote collaborators.
    awareness: Awareness,
}

impl Default for SyncDocument {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl SyncDocument {
    /// Creates a sync document for the given actor id.
    ///
    /// The actor id seeds the underlying `yrs` client id (deterministically, by
    /// hashing), so a peer's tie-breaking order is stable across runs.
    #[must_use]
    pub fn new(actor: impl Into<String>) -> Self {
        let actor = actor.into();
        let doc = Doc::with_client_id(client_id_for(&actor));
        let mut this = Self {
            doc,
            actor,
            counter: 0,
            cache: Document::new(),
            awareness: Awareness::new(),
        };
        this.refresh_cache();
        this
    }

    /// Seeds a new sync document for `actor` from an existing [`Document`].
    ///
    /// Every cell, shape, instance, and array in `document` is written into the
    /// CRDT so it can immediately be shared with peers.
    #[must_use]
    pub fn from_document(actor: impl Into<String>, document: &Document) -> Self {
        let mut this = Self::new(actor);
        this.edit(|txn, next_id| {
            for cell in document.cells() {
                mapping::write_cell(txn, cell, next_id);
            }
            for top in document.top_cells() {
                mapping::set_top_cell(txn, top, true, next_id);
            }
        });
        this
    }

    /// The underlying model document (a materialized view of the CRDT).
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.cache
    }

    /// Materializes the CRDT into a fresh [`Document`].
    ///
    /// This is equivalent to cloning [`SyncDocument::document`]; it is provided to
    /// mirror [`SyncDocument::from_document`].
    #[must_use]
    pub fn to_document(&self) -> Document {
        self.cache.clone()
    }

    /// This peer's actor id.
    #[must_use]
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// The peer's in-memory awareness (remote presence) map.
    #[must_use]
    pub fn awareness(&self) -> &Awareness {
        &self.awareness
    }

    /// Mutable access to the peer's awareness map, for recording remote presence.
    pub fn awareness_mut(&mut self) -> &mut Awareness {
        &mut self.awareness
    }

    // -------------------------------------------------------------------------
    // Local edits
    // -------------------------------------------------------------------------

    /// Adds (or merges into) a cell, writing all of its geometry into the CRDT.
    ///
    /// If a cell with the same name already exists on the CRDT, this cell's
    /// shapes, instances, and arrays are added to it rather than replacing it, so
    /// concurrent edits to the same cell are preserved as a union.
    pub fn add_cell(&mut self, cell: &Cell) {
        self.edit(|txn, next_id| mapping::write_cell(txn, cell, next_id));
    }

    /// Creates an empty cell with the given name (if it does not already exist).
    pub fn add_empty_cell(&mut self, name: &str) {
        self.edit(|txn, next_id| mapping::ensure_cell(txn, name, next_id));
    }

    /// Removes a cell and all of its contents from the CRDT.
    pub fn remove_cell(&mut self, name: &str) {
        self.edit(|txn, _| mapping::remove_cell(txn, name));
    }

    /// Adds a shape to `cell` (creating the cell if needed), returning the unique
    /// element id assigned to it.
    pub fn add_shape(&mut self, cell: &str, shape: &DrawShape) -> String {
        self.edit(|txn, next_id| {
            let id = next_id();
            mapping::insert_shape(txn, cell, &id, shape, next_id);
            id
        })
    }

    /// Adds a rectangle shape on `layer` to `cell`; a convenience over
    /// [`SyncDocument::add_shape`].
    pub fn add_rect(&mut self, cell: &str, layer: LayerId, rect: reticle_geometry::Rect) -> String {
        self.add_shape(
            cell,
            &DrawShape::new(layer, reticle_model::ShapeKind::Rect(rect)),
        )
    }

    /// Adds a single instance placement to `cell` (creating the cell if needed),
    /// returning the unique element id assigned to it.
    pub fn add_instance(&mut self, cell: &str, instance: &Instance) -> String {
        self.edit(|txn, next_id| {
            let id = next_id();
            mapping::insert_instance(txn, cell, &id, instance, next_id);
            id
        })
    }

    /// Adds an array placement to `cell` (creating the cell if needed), returning
    /// the unique element id assigned to it.
    pub fn add_array(&mut self, cell: &str, array: &ArrayInstance) -> String {
        self.edit(|txn, next_id| {
            let id = next_id();
            mapping::insert_array(txn, cell, &id, array, next_id);
            id
        })
    }

    /// Marks (or, with `is_top = false`, clears) a cell as a top (root) cell.
    pub fn set_top_cell(&mut self, name: &str, is_top: bool) {
        self.edit(|txn, next_id| mapping::set_top_cell(txn, name, is_top, next_id));
    }

    /// Runs `f` as a single grouped, atomic CRDT transaction.
    ///
    /// The closure receives a [`StepEdit`] handle whose methods mirror the
    /// individual mutators ([`add_cell`](Self::add_cell),
    /// [`add_shape`](Self::add_shape), ...) but batch every operation into **one**
    /// underlying `yrs` transaction. The whole group commits together and produces a
    /// single update, so a peer never observes a partially-applied step (for example,
    /// one shape of a multi-shape placement landing before the rest).
    ///
    /// Returns whatever the closure returns, so ids allocated inside the step can be
    /// handed back:
    ///
    /// ```
    /// use reticle_sync::SyncDocument;
    /// use reticle_geometry::{LayerId, Point, Rect};
    ///
    /// let mut doc = SyncDocument::new("agent");
    /// let ids = doc.step(|edit| {
    ///     edit.add_empty_cell("top");
    ///     let a = edit.add_rect("top", LayerId::new(68, 20),
    ///         Rect::new(Point::new(0, 0), Point::new(10, 10)));
    ///     let b = edit.add_rect("top", LayerId::new(68, 20),
    ///         Rect::new(Point::new(20, 0), Point::new(30, 10)));
    ///     vec![a, b]
    /// });
    /// assert_eq!(ids.len(), 2);
    /// assert_eq!(doc.document().cell("top").unwrap().shapes.len(), 2);
    /// ```
    pub fn step<R>(&mut self, f: impl FnOnce(&mut StepEdit) -> R) -> R {
        self.edit(|txn, next_id| {
            let mut edit = StepEdit { txn, next_id };
            f(&mut edit)
        })
    }

    // -------------------------------------------------------------------------
    // Update exchange
    // -------------------------------------------------------------------------

    /// Encodes the CRDT changes this peer has that a peer at `state_vector` is
    /// missing, as a binary `yrs` v1 update.
    ///
    /// `state_vector` is a remote peer's [`SyncDocument::state_vector`] output.
    /// The result can be shipped to that peer and applied with
    /// [`SyncDocument::apply_update`].
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::DecodeStateVector`] if `state_vector` is not a valid
    /// encoded [`StateVector`].
    pub fn encode_update(&self, state_vector: &[u8]) -> Result<Vec<u8>> {
        let sv = StateVector::decode_v1(state_vector)
            .map_err(|e| SyncError::DecodeStateVector(e.to_string()))?;
        Ok(self.doc.transact().encode_diff_v1(&sv))
    }

    /// Encodes the peer's entire document state as a single `yrs` v1 update.
    ///
    /// This is [`SyncDocument::encode_update`] against an empty state vector: it
    /// carries everything, so a fresh peer can be brought fully up to date from
    /// it. Handy for the initial exchange and for tests.
    #[must_use]
    pub fn encode_state_update(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// Encodes a **full-state snapshot**: every change this peer has, as one `yrs`
    /// v1 update against the empty state vector.
    ///
    /// This is the resynchronization frame a reconnecting sharer publishes first,
    /// before resuming incremental updates (the live share transport, ADR 0063).
    /// Because it carries the *whole* document rather than a diff since some
    /// remembered point, it is self-contained: a receiver that missed arbitrary
    /// updates while the socket was down (or that never saw any) converges to this
    /// peer's exact document by applying it, and applying it again is a no-op (yrs
    /// updates are idempotent). It is deliberately equivalent to
    /// [`encode_state_update`](Self::encode_state_update); the distinct name marks
    /// the reconnect-resync contract at the call site (snapshot-on-reconnect, not a
    /// state-vector diff, since a reconnecting peer cannot trust any remembered
    /// remote state vector).
    #[must_use]
    pub fn encode_full_state(&self) -> Vec<u8> {
        self.encode_state_update()
    }

    /// This peer's [`StateVector`], encoded as `yrs` v1 bytes.
    ///
    /// Send this to a peer so it can compute (via [`SyncDocument::encode_update`])
    /// exactly the changes this peer is missing.
    #[must_use]
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Applies a binary `yrs` v1 update from a peer, merging its changes into the
    /// local CRDT and refreshing the materialized view.
    ///
    /// Updates are idempotent and commutative: applying the same update twice, or
    /// applying two peers' updates in either order, converges to the same state.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::DecodeUpdate`] if `update` is not a valid encoded
    /// update, or [`SyncError::ApplyUpdate`] if integration fails.
    pub fn apply_update(&mut self, update: &[u8]) -> Result<()> {
        let update =
            Update::decode_v1(update).map_err(|e| SyncError::DecodeUpdate(e.to_string()))?;
        {
            let mut txn = self.doc.transact_mut();
            txn.apply_update(update)
                .map_err(|e| SyncError::ApplyUpdate(e.to_string()))?;
        }
        self.refresh_cache();
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------------------

    /// Runs a write closure inside a single `yrs` transaction, supplying an id
    /// generator that yields globally-unique `actor:counter` element ids, then
    /// commits the transaction and refreshes the materialized view.
    ///
    /// Centralizing edits here keeps id allocation, transaction scoping, and cache
    /// invalidation consistent across every mutator.
    fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut yrs::TransactionMut, &mut dyn FnMut() -> String) -> R,
    ) -> R {
        let actor = self.actor.clone();
        let counter = std::cell::Cell::new(self.counter);
        let mut make_id = || {
            let n = counter.get();
            counter.set(n + 1);
            format!("{actor}:{n}")
        };
        let mut txn = self.doc.transact_mut();
        let result = f(&mut txn, &mut make_id);
        drop(txn);
        self.counter = counter.get();
        self.refresh_cache();
        result
    }

    /// Re-materializes the cached [`Document`] from the CRDT. On the (practically
    /// impossible) event that a stored value is malformed, the previous cache is
    /// retained.
    fn refresh_cache(&mut self) {
        let txn = self.doc.transact();
        let roots = mapping::Roots::resolve(&txn);
        let materialized = mapping::materialize(&txn, &roots);
        drop(txn);
        if let Ok(doc) = materialized {
            self.cache = doc;
        }
    }
}

/// A handle to a single grouped [`SyncDocument::step`] transaction.
///
/// Its methods mirror [`SyncDocument`]'s individual mutators but write into the one
/// shared transaction the step owns, so every operation performed through a single
/// `StepEdit` lands as one atomic CRDT update. The mutators that create an element
/// (`add_shape`, `add_instance`, `add_array`) return the globally-unique element id
/// assigned to it, exactly as their `SyncDocument` counterparts do.
///
/// A `StepEdit` cannot be constructed directly; obtain one from
/// [`SyncDocument::step`]. It borrows the live transaction, so it is scoped to that
/// call and is not `Send`.
pub struct StepEdit<'a, 'txn> {
    txn: &'a mut yrs::TransactionMut<'txn>,
    next_id: &'a mut dyn FnMut() -> String,
}

impl std::fmt::Debug for StepEdit<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepEdit").finish_non_exhaustive()
    }
}

impl StepEdit<'_, '_> {
    /// Adds (or merges into) a cell, writing all of its geometry, as part of the step.
    ///
    /// Mirrors [`SyncDocument::add_cell`]: an existing cell of the same name gains
    /// this cell's contents rather than being replaced.
    pub fn add_cell(&mut self, cell: &Cell) {
        mapping::write_cell(self.txn, cell, self.next_id);
    }

    /// Creates an empty cell (if absent) as part of the step.
    pub fn add_empty_cell(&mut self, name: &str) {
        mapping::ensure_cell(self.txn, name, self.next_id);
    }

    /// Removes a cell and all of its contents as part of the step.
    pub fn remove_cell(&mut self, name: &str) {
        mapping::remove_cell(self.txn, name);
    }

    /// Adds a shape to `cell` (creating the cell if needed) as part of the step,
    /// returning the unique element id assigned to it.
    pub fn add_shape(&mut self, cell: &str, shape: &DrawShape) -> String {
        let id = (self.next_id)();
        mapping::insert_shape(self.txn, cell, &id, shape, self.next_id);
        id
    }

    /// Adds a rectangle shape on `layer` to `cell` as part of the step; a convenience
    /// over [`StepEdit::add_shape`].
    pub fn add_rect(&mut self, cell: &str, layer: LayerId, rect: reticle_geometry::Rect) -> String {
        self.add_shape(
            cell,
            &DrawShape::new(layer, reticle_model::ShapeKind::Rect(rect)),
        )
    }

    /// Adds a single instance placement to `cell` as part of the step, returning the
    /// unique element id assigned to it.
    pub fn add_instance(&mut self, cell: &str, instance: &Instance) -> String {
        let id = (self.next_id)();
        mapping::insert_instance(self.txn, cell, &id, instance, self.next_id);
        id
    }

    /// Adds an array placement to `cell` as part of the step, returning the unique
    /// element id assigned to it.
    pub fn add_array(&mut self, cell: &str, array: &ArrayInstance) -> String {
        let id = (self.next_id)();
        mapping::insert_array(self.txn, cell, &id, array, self.next_id);
        id
    }

    /// Marks (or, with `is_top = false`, clears) a cell as a top (root) cell, as part
    /// of the step.
    pub fn set_top_cell(&mut self, name: &str, is_top: bool) {
        mapping::set_top_cell(self.txn, name, is_top, self.next_id);
    }

    /// Overwrites the shape stored under CRDT element `id` (owned by `cell`) with
    /// `shape`, in place, as part of the step.
    ///
    /// `id` must be an element id an earlier [`add_shape`](Self::add_shape) returned
    /// (or the `SyncDocument` equivalent): the record keeps its id and only its
    /// geometry changes, so a converged peer sees the same element *move* rather than
    /// a delete-and-recreate. This is the write a mirrored in-place transform makes;
    /// an `id` no shape is stored under simply creates one, so callers resolve the id
    /// before calling.
    pub fn set_shape(&mut self, cell: &str, id: &str, shape: &DrawShape) {
        mapping::overwrite_shape(self.txn, cell, id, shape);
    }

    /// Removes the shape stored under CRDT element `id`, as part of the step.
    ///
    /// Mirrors a delete of a single shape (by the element id an earlier
    /// [`add_shape`](Self::add_shape) returned); every other record, including a
    /// concurrent peer's, is untouched. Removing an absent id is a no-op.
    pub fn remove_shape(&mut self, id: &str) {
        mapping::remove_shape_by_id(self.txn, id);
    }
}

/// Derives a stable `yrs` client id from an actor id by hashing.
fn client_id_for(actor: &str) -> u64 {
    if actor.is_empty() {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    actor.hash(&mut hasher);
    // `yrs` client ids are compared for seniority; any non-zero value works.
    hasher.finish()
}

#[cfg(test)]
mod step_edit_tests {
    use super::SyncDocument;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{DrawShape, ShapeKind};

    /// A met1 rectangle from `(x0,y0)` to `(x1,y1)`.
    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    /// The single rectangle in `cell`, or a panic if the shape is not a rectangle.
    fn only_rect(doc: &SyncDocument, cell: &str) -> Rect {
        let c = doc.document().cell(cell).expect("cell present");
        assert_eq!(c.shapes.len(), 1, "exactly one shape");
        match c.shapes[0].kind {
            ShapeKind::Rect(r) => r,
            ref other => panic!("expected a rect, got {other:?}"),
        }
    }

    #[test]
    fn set_shape_moves_a_shape_in_place_keeping_its_id() {
        let mut doc = SyncDocument::new("agent");
        let id = doc.step(|edit| {
            edit.add_empty_cell("top");
            edit.add_rect(
                "top",
                LayerId::new(68, 20),
                Rect::new(Point::ORIGIN, Point::new(10, 10)),
            )
        });
        // Overwrite the shape at its id with moved geometry.
        doc.step(|edit| edit.set_shape("top", &id, &rect(100, 0, 110, 10)));
        assert_eq!(
            only_rect(&doc, "top"),
            Rect::new(Point::new(100, 0), Point::new(110, 10)),
            "the shape moved in place"
        );
    }

    #[test]
    fn remove_shape_deletes_only_that_shape() {
        let mut doc = SyncDocument::new("agent");
        let (a, _b) = doc.step(|edit| {
            edit.add_empty_cell("top");
            let a = edit.add_rect(
                "top",
                LayerId::new(68, 20),
                Rect::new(Point::ORIGIN, Point::new(4, 4)),
            );
            let b = edit.add_rect(
                "top",
                LayerId::new(68, 20),
                Rect::new(Point::new(8, 0), Point::new(12, 4)),
            );
            (a, b)
        });
        doc.step(|edit| edit.remove_shape(&a));
        assert_eq!(
            only_rect(&doc, "top"),
            Rect::new(Point::new(8, 0), Point::new(12, 4)),
            "the surviving shape is the one not removed"
        );
    }

    #[test]
    fn a_mirrored_transform_converges_with_a_concurrent_peer_edit() {
        // The agent overwrites its own shape in place while a human adds a shape to the
        // same cell concurrently; the two peers converge to the union with the move applied.
        let mut agent = SyncDocument::new("agent");
        let id = agent.step(|edit| {
            edit.add_empty_cell("shared");
            edit.add_rect(
                "shared",
                LayerId::new(68, 20),
                Rect::new(Point::ORIGIN, Point::new(10, 10)),
            )
        });
        let mut human = SyncDocument::new("human");
        human.add_empty_cell("shared");
        human.add_shape("shared", &rect(50, 50, 60, 60));

        // Concurrent: the agent moves its shape in place.
        agent.step(|edit| edit.set_shape("shared", &id, &rect(200, 0, 210, 10)));

        let sv_a = agent.state_vector();
        let sv_h = human.state_vector();
        let a_to_h = agent.encode_update(&sv_h).expect("a->h");
        let h_to_a = human.encode_update(&sv_a).expect("h->a");
        agent.apply_update(&h_to_a).expect("apply h->a");
        human.apply_update(&a_to_h).expect("apply a->h");

        assert_eq!(agent.document(), human.document(), "peers converge");
        let cell = agent.document().cell("shared").expect("shared cell");
        assert_eq!(
            cell.shapes.len(),
            2,
            "union of the moved agent shape and the human shape"
        );
    }
}

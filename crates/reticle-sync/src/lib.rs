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
mod mapping;
mod presence;

pub use comment::{Comment, CommentThread};
pub use error::{Result, SyncError};
pub use presence::{Awareness, Presence};

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

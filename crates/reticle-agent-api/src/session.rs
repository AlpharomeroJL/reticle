//! The command session: an editable document, a revision, a stable-id allocator,
//! and a transcript.
//!
//! A [`Session`] is the stateful target of [`AgentCommand`](crate::AgentCommand)s.
//! It owns an [`EditableDocument`], a monotonic [`revision`](Session::revision), a
//! [`Vec`] of [`CommandRecord`](crate::CommandRecord)s, and an [`Allocator`] that
//! maps each stable [`ElementId`] to the element it addresses. The apply loop lives
//! in the `apply` module; this module owns the state and the id bookkeeping.
//!
//! # Stable ids across removals
//!
//! The engine's [`Edit`](reticle_model::Edit) vocabulary addresses shapes by
//! positional index, and removing one shifts the indices of every later shape in
//! that cell's vector. The [`Allocator`] therefore reconciles its slot map on each
//! removal so a live [`ElementId`] keeps addressing the same element. Instances and
//! arrays have no positional-remove edit, so their slots never move.

use std::collections::HashMap;

use reticle_model::{Document, EditableDocument};

use crate::ElementId;

/// Which of a cell's parallel vectors an [`ElementId`] indexes.
///
/// Shapes are removable and their indices shift on removal (see the [module
/// docs](self)); instances and arrays are append-only in the edit vocabulary, so
/// their slots are permanent once assigned. Labels have no create command in the
/// frozen surface, so no id ever addresses one and they are not represented here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ElementKind {
    /// Indexes `Cell::shapes`.
    Shape,
    /// Indexes `Cell::instances`.
    Instance,
    /// Indexes `Cell::arrays`.
    Array,
}

/// Where a live [`ElementId`] currently points: a cell, which vector, and the slot.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ElementRef {
    /// The owning cell's name.
    pub cell: String,
    /// Which of the cell's vectors the slot indexes.
    pub kind: ElementKind,
    /// The current index into that vector.
    pub slot: usize,
}

/// Allocates stable [`ElementId`]s and keeps their slot mapping current across
/// index-shifting removals.
///
/// The allocator hands out ids monotonically (never reused within a session) and
/// records, per id, the `(cell, kind, slot)` it addresses. Removing a shape shifts
/// the vector, so [`remove`](Allocator::remove) decrements every live id in the
/// same cell and vector whose slot sat above the removed one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Allocator {
    next: u64,
    map: HashMap<ElementId, ElementRef>,
}

impl Allocator {
    /// A fresh allocator with no live ids; the first id handed out is `1`.
    pub(crate) fn new() -> Self {
        Self {
            next: 1,
            map: HashMap::new(),
        }
    }

    /// Allocates the next id for the element at `slot` of `cell`'s `kind` vector.
    pub(crate) fn allocate(&mut self, cell: &str, kind: ElementKind, slot: usize) -> ElementId {
        let id = ElementId(self.next);
        self.next += 1;
        self.map.insert(
            id,
            ElementRef {
                cell: cell.to_owned(),
                kind,
                slot,
            },
        );
        id
    }

    /// The element an id addresses, if it is live.
    pub(crate) fn resolve(&self, id: ElementId) -> Option<&ElementRef> {
        self.map.get(&id)
    }

    /// The live id addressing (`cell`, `kind`, `slot`), if any.
    pub(crate) fn id_for(&self, cell: &str, kind: ElementKind, slot: usize) -> Option<ElementId> {
        self.map
            .iter()
            .find(|(_, r)| r.cell == cell && r.kind == kind && r.slot == slot)
            .map(|(id, _)| *id)
    }

    /// Re-points an existing id at a new `(cell, kind, slot)`, keeping the id stable.
    ///
    /// Used by an in-place transform, which removes a shape and re-appends the
    /// transformed geometry: the same id must now address the appended slot.
    pub(crate) fn rebind(&mut self, id: ElementId, cell: &str, kind: ElementKind, slot: usize) {
        self.map.insert(
            id,
            ElementRef {
                cell: cell.to_owned(),
                kind,
                slot,
            },
        );
    }

    /// Removes the id for the element at (`cell`, `kind`, `slot`) and reconciles the
    /// slots of the remaining live ids in the same cell and vector: every id above
    /// the removed slot moves down by one, matching `Vec::remove`.
    ///
    /// Idempotent for an already-absent slot: if no id maps to it, only the shift is
    /// applied. Returns the id that addressed the removed slot, if one did.
    pub(crate) fn remove(
        &mut self,
        cell: &str,
        kind: ElementKind,
        slot: usize,
    ) -> Option<ElementId> {
        let removed = self
            .map
            .iter()
            .find(|(_, r)| r.cell == cell && r.kind == kind && r.slot == slot)
            .map(|(id, _)| *id);
        if let Some(id) = removed {
            self.map.remove(&id);
        }
        for r in self.map.values_mut() {
            if r.cell == cell && r.kind == kind && r.slot > slot {
                r.slot -= 1;
            }
        }
        removed
    }

    /// Drops every id owned by `cell`. Used when a whole cell is deleted, since its
    /// vectors vanish and no per-slot reconciliation applies.
    pub(crate) fn forget_cell(&mut self, cell: &str) {
        self.map.retain(|_, r| r.cell != cell);
    }
}

/// A stateful command session over the engine.
///
/// Owns the editable document, a monotonic revision that advances on every applied
/// mutation, the stable-id allocator, and the command transcript. Commands are
/// dispatched through [`Session::apply`].
#[derive(Debug, Default)]
pub struct Session {
    /// The editable document the commands mutate.
    pub(crate) doc: EditableDocument,
    /// Monotonic count of applied mutations; mirrors the document revision but is
    /// owned here so a session that swaps its document (import, load) keeps a
    /// continuous, non-decreasing revision.
    pub(crate) revision: u64,
    /// The stable-id allocator and its slot map.
    pub(crate) alloc: Allocator,
    /// The append-only command transcript.
    pub(crate) transcript: Vec<crate::CommandRecord>,
    /// Wall-clock origin for transcript timestamps, established on first use.
    pub(crate) started: Option<std::time::Instant>,
}

impl Session {
    /// Creates an empty session: an empty document, revision `0`, no ids, and an
    /// empty transcript.
    #[must_use]
    pub fn new() -> Self {
        Self {
            doc: EditableDocument::new(Document::new()),
            revision: 0,
            alloc: Allocator::new(),
            transcript: Vec::new(),
            started: None,
        }
    }

    /// Borrows the current document.
    #[must_use]
    pub fn document(&self) -> &Document {
        self.doc.document()
    }

    /// The current revision: `0` for a new session, incremented by one on each
    /// applied mutating command.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The command transcript recorded so far, in order.
    #[must_use]
    pub fn transcript(&self) -> &[crate::CommandRecord] {
        &self.transcript
    }

    /// Milliseconds elapsed since the session's clock origin, establishing the
    /// origin on the first call so a transcript's timestamps start near zero.
    ///
    /// On `wasm32-unknown-unknown` there is no monotonic clock (`Instant::now`
    /// panics), so timestamps degrade to zero rather than aborting the session.
    pub(crate) fn now_ms(&mut self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let origin = *self.started.get_or_insert_with(std::time::Instant::now);
            origin.elapsed().as_millis() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            // No monotonic clock on wasm; `started` stays `None` and this read keeps
            // the field live for the dead-code lint.
            let _ = &self.started;
            0
        }
    }

    /// A serializable snapshot of the session: its command transcript.
    ///
    /// The document and the id allocator are both reproducible from the recorded
    /// commands, so persisting the transcript is sufficient;
    /// [`Self::from_snapshot_str`] rebuilds an equivalent session by re-applying
    /// them. The snapshot carries the `document_hash` of the current document so a
    /// load can be verified.
    pub(crate) fn snapshot_json(&self) -> serde_json::Value {
        let transcript = crate::Transcript {
            records: self.transcript.clone(),
            final_hash: reticle_model::document_hash(self.document()),
            // A session snapshot persists the command history, not the agent harness's
            // per-iteration plan; the plan log is empty here.
            plan: Vec::new(),
        };
        serde_json::json!({ "transcript": transcript })
    }

    /// Rebuilds a session from a [`snapshot_json`](Self::snapshot_json) string by
    /// re-applying the recorded commands onto a fresh session.
    ///
    /// Because command application is deterministic, the rebuilt document, revision,
    /// and every [`ElementId`] match the saved session.
    pub(crate) fn from_snapshot_str(snapshot: &str) -> Result<Self, crate::AgentError> {
        use crate::{AgentError, ErrorCode};

        #[derive(serde::Deserialize)]
        struct Snapshot {
            transcript: crate::Transcript,
        }
        let parsed: Snapshot = serde_json::from_str(snapshot).map_err(|e| {
            AgentError::new(
                ErrorCode::InvalidArgument,
                format!("session snapshot parse: {e}"),
            )
        })?;
        let mut session = Session::new();
        for record in parsed.transcript.records {
            // Re-apply every recorded command; a command that failed originally will
            // fail again the same way, so the rebuilt transcript matches.
            let _ = session.apply(record.command);
        }
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::{Allocator, ElementKind};

    /// Ids are handed out monotonically starting at 1 and resolve to their slot.
    #[test]
    fn allocate_is_monotonic() {
        let mut a = Allocator::new();
        let e1 = a.allocate("top", ElementKind::Shape, 0);
        let e2 = a.allocate("top", ElementKind::Shape, 1);
        assert_eq!(e1.0, 1);
        assert_eq!(e2.0, 2);
        assert_eq!(a.resolve(e2).unwrap().slot, 1);
    }

    /// Removing a lower slot shifts the higher live ids down by one so they still
    /// address the same element.
    #[test]
    fn remove_reconciles_higher_slots() {
        let mut a = Allocator::new();
        let e0 = a.allocate("top", ElementKind::Shape, 0);
        let e1 = a.allocate("top", ElementKind::Shape, 1);
        let e2 = a.allocate("top", ElementKind::Shape, 2);
        // Remove slot 1 (e1): e2 must slide from slot 2 to slot 1; e0 is unchanged.
        let gone = a.remove("top", ElementKind::Shape, 1);
        assert_eq!(gone, Some(e1));
        assert!(a.resolve(e1).is_none());
        assert_eq!(a.resolve(e0).unwrap().slot, 0);
        assert_eq!(a.resolve(e2).unwrap().slot, 1);
    }

    /// Shifts are scoped to the same cell and element kind.
    #[test]
    fn remove_is_scoped_by_cell_and_kind() {
        let mut a = Allocator::new();
        let shape = a.allocate("top", ElementKind::Shape, 1);
        let inst = a.allocate("top", ElementKind::Instance, 1);
        let other = a.allocate("other", ElementKind::Shape, 1);
        a.remove("top", ElementKind::Shape, 0);
        // The instance (different kind) and the other cell's shape do not move.
        assert_eq!(a.resolve(shape).unwrap().slot, 0);
        assert_eq!(a.resolve(inst).unwrap().slot, 1);
        assert_eq!(a.resolve(other).unwrap().slot, 1);
    }

    /// Forgetting a cell drops exactly its ids.
    #[test]
    fn forget_cell_drops_only_that_cell() {
        let mut a = Allocator::new();
        let top = a.allocate("top", ElementKind::Shape, 0);
        let keep = a.allocate("keep", ElementKind::Shape, 0);
        a.forget_cell("top");
        assert!(a.resolve(top).is_none());
        assert!(a.resolve(keep).is_some());
    }
}

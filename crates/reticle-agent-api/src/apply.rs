//! Applying an [`AgentCommand`] to a [`Session`].
//!
//! [`Session::apply`] is the single dispatch point over the engine: it converts the
//! serde argument types to engine types, performs the operation against the editable
//! document (or the read-only engines for queries), records a [`CommandRecord`], and
//! returns a [`CommandResult`]. It never panics: every engine failure is mapped to
//! an [`AgentError`] with a fitting [`ErrorCode`].
//!
//! # Mutations and stable ids
//!
//! Mutating commands go through the [`Edit`] vocabulary so the document's undo
//! history and revision stay correct, and they update the session's stable-id
//! allocator so returned [`ElementId`]s keep addressing the elements they created.
//! Because the edit vocabulary can only *append* shapes, `TransformShapes` is
//! modelled as a remove-then-append that rebinds the same id to the shape's new
//! slot.

use reticle_geometry::{
    BooleanOp, Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Shape as _,
    Transform, polygon_boolean,
};
use reticle_model::{
    ArrayInstance, Cell, DrawShape, Edit, EditableDocument, Instance, Rule, RuleKind, ShapeKind,
    Violation,
};

use crate::args::{
    AlignArg, AxisArg, BooleanOpArg, EndcapArg, LayerArg, OrientationArg, PointArg, RectArg,
    TransformArg,
};
use crate::session::{ElementKind, ElementRef};
use crate::{AgentCommand, AgentError, AgentResponse, CommandResult, ElementId, ErrorCode};
use crate::{CommandRecord, Outcome, Session};

/// One resolved shape for a Wave 2 op: its id, its current slot in the cell's shape
/// vector, and a clone of its geometry. Produced by
/// [`Session::resolve_shapes_in_cell`].
type ResolvedShape = (ElementId, usize, DrawShape);

impl Session {
    /// Applies one command, returning its response or a structured error.
    ///
    /// Every call appends a [`CommandRecord`] to the transcript (whether it
    /// succeeded or failed). A mutating command that succeeds advances the session
    /// [`revision`](Session::revision) by one and returns the affected
    /// [`ElementId`]s; a read-only command returns [`AgentResponse::Data`] or
    /// [`AgentResponse::Blob`] at the current revision. Bad input and engine
    /// failures become an [`AgentError`] and never panic.
    pub fn apply(&mut self, cmd: AgentCommand) -> CommandResult {
        let seq = self.transcript.len() as u64;
        let revision_before = self.revision;
        let ts_start_ms = self.now_ms();

        let outcome = self.dispatch(cmd.clone());

        let ts_end_ms = self.now_ms();
        let record = CommandRecord {
            seq,
            command: cmd,
            revision_before,
            revision_after: self.revision,
            outcome: match &outcome {
                Ok(resp) => Outcome::Ok(resp.clone()),
                Err(err) => Outcome::Err(err.clone()),
            },
            ts_start_ms,
            ts_end_ms,
            tokens_in: None,
            tokens_out: None,
        };
        self.transcript.push(record);
        outcome
    }

    /// The command dispatch, without transcript bookkeeping (that lives in
    /// [`apply`](Self::apply)).
    fn dispatch(&mut self, cmd: AgentCommand) -> CommandResult {
        match cmd {
            AgentCommand::CreateCell { name } => self.create_cell(name),
            AgentCommand::DeleteCell { name } => self.delete_cell(&name),
            AgentCommand::AddRect { cell, layer, rect } => self.add_rect(&cell, layer, rect),
            AgentCommand::AddPolygon {
                cell,
                layer,
                points,
            } => self.add_polygon(&cell, layer, &points),
            AgentCommand::AddPath {
                cell,
                layer,
                width,
                points,
                endcap,
            } => self.add_path(&cell, layer, width, &points, endcap),
            AgentCommand::PlaceInstance {
                cell,
                child,
                transform,
            } => self.place_instance(&cell, child, transform),
            AgentCommand::PlaceArray {
                cell,
                child,
                transform,
                columns,
                rows,
                column_pitch,
                row_pitch,
            } => self.place_array(
                &cell,
                child,
                transform,
                columns,
                rows,
                column_pitch,
                row_pitch,
            ),
            AgentCommand::TransformShapes { ids, transform } => {
                self.transform_shapes(&ids, transform)
            }
            AgentCommand::DeleteShapes { ids } => self.delete_shapes(&ids),
            AgentCommand::QueryShapes {
                cell,
                layer,
                region,
            } => self.query_shapes(&cell, layer, region),
            AgentCommand::GetCellInfo { cell } => self.get_cell_info(&cell),
            AgentCommand::ListLayers => Ok(self.list_layers()),
            AgentCommand::SetTechnology { source } => self.set_technology(&source),
            AgentCommand::RunDrc { cell, region } => self.run_drc(&cell, region),
            AgentCommand::GetViolations => Ok(self.get_violations()),
            AgentCommand::RouteNet {
                cell,
                net,
                layer,
                terminals,
            } => self.route_net(&cell, net, layer, &terminals),
            AgentCommand::RunExtract { cell } => self.run_extract(&cell),
            AgentCommand::CheckIntent { cell, intent } => self.check_intent(&cell, &intent),
            AgentCommand::NetlistCompare { cell, expected } => {
                self.netlist_compare(&cell, &expected)
            }
            AgentCommand::ExportGds => self.export_gds(),
            AgentCommand::ExportOasis => self.export_oasis(),
            AgentCommand::ImportGds { bytes } => self.import_gds(&bytes),
            AgentCommand::RenderPng {
                region,
                width,
                height,
            } => self.render_png(region, width, height),
            AgentCommand::SaveSession => self.save_session(),
            AgentCommand::LoadSession { snapshot } => self.load_session(&snapshot),
            AgentCommand::BooleanCombine {
                cell,
                bool_op,
                ids,
                layer,
            } => self.boolean_combine(&cell, bool_op, &ids, layer),
            AgentCommand::AlignShapes { ids, align } => self.align_shapes(&ids, align),
            AgentCommand::DistributeShapes { ids, axis } => self.distribute_shapes(&ids, axis),
            AgentCommand::OffsetShapes { ids, delta } => self.offset_shapes(&ids, delta),
            AgentCommand::BuildViaStack {
                cell,
                lower_layer,
                upper_layer,
                cut_layer,
                center,
                cut_size,
                default_enclosure,
            } => self.build_via_stack(
                &cell,
                lower_layer,
                upper_layer,
                cut_layer,
                center,
                cut_size,
                default_enclosure,
            ),
        }
    }

    // ----- mutation helpers -------------------------------------------------

    /// Applies an edit through the editor and mirrors the revision, mapping a model
    /// error to an [`AgentError`].
    fn commit(&mut self, edit: Edit) -> Result<(), AgentError> {
        use reticle_model::DocumentStore;
        self.doc.apply(edit).map_err(map_model_err)?;
        self.revision += 1;
        Ok(())
    }

    /// A successful mutation response at the current revision.
    fn ok(&self, affected: Vec<ElementId>) -> AgentResponse {
        AgentResponse::Ok {
            revision: self.revision,
            affected,
        }
    }

    /// Structured read-only data at the current revision.
    fn data(&self, value: serde_json::Value) -> AgentResponse {
        AgentResponse::Data {
            revision: self.revision,
            value,
        }
    }

    /// A binary payload at the current revision.
    fn blob(&self, bytes: Vec<u8>) -> AgentResponse {
        AgentResponse::Blob {
            revision: self.revision,
            bytes,
        }
    }

    /// Requires that `cell` exists, or a [`ErrorCode::NoSuchCell`] error.
    fn require_cell(&self, cell: &str) -> Result<(), AgentError> {
        if self.document().cell(cell).is_some() {
            Ok(())
        } else {
            Err(AgentError::no_such_cell(cell))
        }
    }

    // ----- cells ------------------------------------------------------------

    fn create_cell(&mut self, name: String) -> CommandResult {
        if name.is_empty() {
            return Err(AgentError::invalid("cell name must be non-empty"));
        }
        self.commit(Edit::AddCell {
            cell: Cell::new(name),
        })?;
        Ok(self.ok(vec![]))
    }

    fn delete_cell(&mut self, name: &str) -> CommandResult {
        self.require_cell(name)?;
        self.commit(Edit::RemoveCell {
            name: name.to_owned(),
        })?;
        self.alloc.forget_cell(name);
        Ok(self.ok(vec![]))
    }

    // ----- shapes -----------------------------------------------------------

    fn add_rect(&mut self, cell: &str, layer: LayerArg, rect: RectArg) -> CommandResult {
        self.require_cell(cell)?;
        let shape = DrawShape::new(layer_id(layer), ShapeKind::Rect(to_rect(rect)));
        self.add_shape(cell, shape)
    }

    fn add_polygon(&mut self, cell: &str, layer: LayerArg, points: &[PointArg]) -> CommandResult {
        self.require_cell(cell)?;
        if points.len() < 3 {
            return Err(AgentError::invalid(
                "a polygon needs at least three vertices",
            ));
        }
        let poly = Polygon::new(points.iter().map(|p| to_point(*p)).collect());
        let shape = DrawShape::new(layer_id(layer), ShapeKind::Polygon(poly));
        self.add_shape(cell, shape)
    }

    fn add_path(
        &mut self,
        cell: &str,
        layer: LayerArg,
        width: i32,
        points: &[PointArg],
        endcap: Option<EndcapArg>,
    ) -> CommandResult {
        self.require_cell(cell)?;
        if points.len() < 2 {
            return Err(AgentError::invalid("a path needs at least two vertices"));
        }
        if width < 0 {
            return Err(AgentError::invalid("path width must be non-negative"));
        }
        let path = Path::new(
            points.iter().map(|p| to_point(*p)).collect(),
            width,
            to_endcap(endcap),
        );
        let shape = DrawShape::new(layer_id(layer), ShapeKind::Path(path));
        self.add_shape(cell, shape)
    }

    /// Appends `shape` to `cell` and allocates a stable id for its new slot.
    fn add_shape(&mut self, cell: &str, shape: DrawShape) -> CommandResult {
        let slot = self.document().cell(cell).map_or(0, |c| c.shapes.len());
        self.commit(Edit::AddShape {
            cell: cell.to_owned(),
            shape,
        })?;
        let id = self.alloc.allocate(cell, ElementKind::Shape, slot);
        Ok(self.ok(vec![id]))
    }

    // ----- placements -------------------------------------------------------

    fn place_instance(
        &mut self,
        cell: &str,
        child: String,
        transform: TransformArg,
    ) -> CommandResult {
        self.require_cell(cell)?;
        self.require_cell(&child)?;
        let instance = Instance {
            cell: child,
            transform: to_transform(transform)?,
        };
        let slot = self.document().cell(cell).map_or(0, |c| c.instances.len());
        self.commit(Edit::AddInstance {
            cell: cell.to_owned(),
            instance,
        })?;
        let id = self.alloc.allocate(cell, ElementKind::Instance, slot);
        Ok(self.ok(vec![id]))
    }

    #[allow(clippy::too_many_arguments)]
    fn place_array(
        &mut self,
        cell: &str,
        child: String,
        transform: TransformArg,
        columns: u32,
        rows: u32,
        column_pitch: i32,
        row_pitch: i32,
    ) -> CommandResult {
        self.require_cell(cell)?;
        self.require_cell(&child)?;
        if columns == 0 || rows == 0 {
            return Err(AgentError::invalid(
                "array columns and rows must be positive",
            ));
        }
        let array = ArrayInstance {
            cell: child,
            transform: to_transform(transform)?,
            columns,
            rows,
            column_pitch,
            row_pitch,
        };
        let slot = self.document().cell(cell).map_or(0, |c| c.arrays.len());
        self.commit(Edit::AddArray {
            cell: cell.to_owned(),
            array,
        })?;
        let id = self.alloc.allocate(cell, ElementKind::Array, slot);
        Ok(self.ok(vec![id]))
    }

    // ----- transform / delete existing shapes -------------------------------

    /// Transforms each addressed shape in place by rebinding its id.
    ///
    /// The edit vocabulary can only append, so each shape is removed at its current
    /// slot and the transformed geometry is appended; the same [`ElementId`] is then
    /// re-pointed at the new (end) slot. Ids are resolved one at a time so a batch
    /// touching several shapes in the same cell stays consistent as slots shift.
    fn transform_shapes(&mut self, ids: &[ElementId], transform: TransformArg) -> CommandResult {
        let xform = to_transform(transform)?;
        // Validate all ids up front so a bad id fails the whole batch atomically.
        for &id in ids {
            let r = self.alloc.resolve(id).ok_or_else(|| no_such_element(id))?;
            if r.kind != ElementKind::Shape {
                return Err(AgentError::invalid(format!(
                    "{id} is not a shape; only shapes can be transformed"
                )));
            }
        }
        for &id in ids {
            // Re-resolve every step: an earlier removal in this cell may have shifted
            // this id's slot.
            let ElementRef { cell, slot, .. } = self
                .alloc
                .resolve(id)
                .cloned()
                .ok_or_else(|| no_such_element(id))?;
            let shape = self
                .document()
                .cell(&cell)
                .and_then(|c| c.shapes.get(slot))
                .cloned()
                .ok_or_else(|| no_such_element(id))?;
            let moved = transform_shape(&xform, &shape);
            // Remove the old slot (shifts higher ids down) then append the new shape.
            self.commit(Edit::RemoveShape {
                cell: cell.clone(),
                index: slot,
            })?;
            self.alloc.remove(&cell, ElementKind::Shape, slot);
            let new_slot = self.document().cell(&cell).map_or(0, |c| c.shapes.len());
            self.commit(Edit::AddShape {
                cell: cell.clone(),
                shape: moved,
            })?;
            self.alloc.rebind(id, &cell, ElementKind::Shape, new_slot);
        }
        Ok(self.ok(ids.to_vec()))
    }

    /// Deletes each addressed shape, reconciling the id map so surviving ids keep
    /// addressing the same elements.
    ///
    /// Only shapes are removable through the edit vocabulary; an id naming an
    /// instance or array is rejected up front so the batch stays atomic.
    fn delete_shapes(&mut self, ids: &[ElementId]) -> CommandResult {
        for &id in ids {
            let r = self.alloc.resolve(id).ok_or_else(|| no_such_element(id))?;
            if r.kind != ElementKind::Shape {
                return Err(AgentError::invalid(format!(
                    "{id} is an instance or array; those cannot be deleted individually"
                )));
            }
        }
        for &id in ids {
            let Some(ElementRef { cell, slot, .. }) = self.alloc.resolve(id).cloned() else {
                // Already removed as part of this batch (a duplicate id); skip.
                continue;
            };
            self.commit(Edit::RemoveShape {
                cell: cell.clone(),
                index: slot,
            })?;
            self.alloc.remove(&cell, ElementKind::Shape, slot);
        }
        Ok(self.ok(ids.to_vec()))
    }

    // ----- Wave 2 editor ops as agent commands ------------------------------

    /// Resolves a set of ids to shapes in a single cell, cloning each shape.
    ///
    /// Every id must be live and name a shape (not an instance or array), and all
    /// must live in the same cell; a multi-cell selection, an unknown id, or a
    /// non-shape id is an [`ErrorCode::InvalidArgument`]/[`ErrorCode::NoSuchElement`]
    /// error so the operation stays atomic. Returns the shared cell name and a list
    /// of `(id, slot, shape)` in the order the ids were given.
    fn resolve_shapes_in_cell(
        &self,
        ids: &[ElementId],
    ) -> Result<(String, Vec<ResolvedShape>), AgentError> {
        let mut cell_name: Option<String> = None;
        let mut resolved = Vec::with_capacity(ids.len());
        for &id in ids {
            let r = self.alloc.resolve(id).ok_or_else(|| no_such_element(id))?;
            if r.kind != ElementKind::Shape {
                return Err(AgentError::invalid(format!(
                    "{id} is an instance or array; this operation only accepts shapes"
                )));
            }
            match &cell_name {
                None => cell_name = Some(r.cell.clone()),
                Some(name) if name != &r.cell => {
                    return Err(AgentError::invalid(
                        "all shapes must be in the same cell for this operation",
                    ));
                }
                Some(_) => {}
            }
            let (cell, slot) = (r.cell.clone(), r.slot);
            let shape = self
                .document()
                .cell(&cell)
                .and_then(|c| c.shapes.get(slot))
                .cloned()
                .ok_or_else(|| no_such_element(id))?;
            resolved.push((id, slot, shape));
        }
        let cell = cell_name.ok_or_else(|| AgentError::invalid("no shapes given"))?;
        Ok((cell, resolved))
    }

    /// Combines a set of shapes with a planar boolean, writing the result polygons
    /// to `layer` in the same cell and deleting the input shapes.
    ///
    /// Mirrors the editor's boolean: rectangles and polygons become boolean input
    /// (a path is a stroke, not a fill, and is skipped); union/intersection/xor fold
    /// pairwise and difference subtracts every later shape from the first. An
    /// operation that yields no geometry (for example a disjoint intersection) still
    /// consumes its inputs, matching a destructive editor combine. The result
    /// polygons are appended to `layer` and their ids returned.
    fn boolean_combine(
        &mut self,
        cell: &str,
        op: BooleanOpArg,
        ids: &[ElementId],
        layer: LayerArg,
    ) -> CommandResult {
        if ids.len() < 2 {
            return Err(AgentError::invalid(
                "a boolean combine needs at least two shapes",
            ));
        }
        let (owner, resolved) = self.resolve_shapes_in_cell(ids)?;
        if owner != cell {
            return Err(AgentError::invalid(format!(
                "shapes are in cell `{owner}`, not `{cell}`"
            )));
        }
        let polys: Vec<Polygon> = resolved
            .iter()
            .filter_map(|(_, _, s)| shape_as_polygon(s))
            .collect();
        if polys.len() < 2 {
            return Err(AgentError::invalid(
                "a boolean combine needs at least two fillable shapes (rectangles or \
                 polygons); paths are skipped",
            ));
        }
        let result = fold_boolean(to_boolean_op(op), &polys)?;

        // Delete the inputs highest-slot-first so earlier removals do not shift the
        // slots of later ones, then reconcile each id out of the allocator.
        let mut slots: Vec<(ElementId, usize)> =
            resolved.iter().map(|(id, slot, _)| (*id, *slot)).collect();
        slots.sort_by(|a, b| b.1.cmp(&a.1));
        for (_, slot) in &slots {
            self.commit(Edit::RemoveShape {
                cell: cell.to_owned(),
                index: *slot,
            })?;
            self.alloc.remove(cell, ElementKind::Shape, *slot);
        }

        // Append the result polygons to the target layer, allocating a fresh id each.
        let out_layer = layer_id(layer);
        let mut new_ids = Vec::with_capacity(result.len());
        for poly in result {
            if poly.len() < 3 {
                continue;
            }
            let slot = self.document().cell(cell).map_or(0, |c| c.shapes.len());
            self.commit(Edit::AddShape {
                cell: cell.to_owned(),
                shape: DrawShape::new(out_layer, ShapeKind::Polygon(poly)),
            })?;
            new_ids.push(self.alloc.allocate(cell, ElementKind::Shape, slot));
        }
        Ok(self.ok(new_ids))
    }

    /// Aligns a set of shapes within their combined bounding box, keeping each id.
    ///
    /// Computes a per-shape translation from the alignment kind (mirroring the
    /// editor's align), then re-lays each moved shape in place by removing it and
    /// re-appending the translated geometry under the same id. Shapes already in
    /// position are untouched.
    fn align_shapes(&mut self, ids: &[ElementId], align: AlignArg) -> CommandResult {
        if ids.len() < 2 {
            return Err(AgentError::invalid("aligning needs at least two shapes"));
        }
        let (cell, resolved) = self.resolve_shapes_in_cell(ids)?;
        let shapes: Vec<DrawShape> = resolved.iter().map(|(_, _, s)| s.clone()).collect();
        let offsets = align_offsets(&shapes, align);
        self.apply_translations(&cell, &resolved, &offsets)?;
        Ok(self.ok(ids.to_vec()))
    }

    /// Distributes a set of shapes so adjacent gaps are equal along `axis`, keeping
    /// each id. With fewer than three shapes there is nothing to distribute.
    fn distribute_shapes(&mut self, ids: &[ElementId], axis: AxisArg) -> CommandResult {
        if ids.len() < 3 {
            return Err(AgentError::invalid(
                "distributing needs at least three shapes",
            ));
        }
        let (cell, resolved) = self.resolve_shapes_in_cell(ids)?;
        let shapes: Vec<DrawShape> = resolved.iter().map(|(_, _, s)| s.clone()).collect();
        let horizontal = matches!(axis, AxisArg::Horizontal);
        let offsets = distribute_offsets(&shapes, horizontal);
        self.apply_translations(&cell, &resolved, &offsets)?;
        Ok(self.ok(ids.to_vec()))
    }

    /// Applies a per-shape `(dx, dy)` translation list (aligned with `resolved`),
    /// rebinding each moved shape's id to its new slot.
    ///
    /// Like [`transform_shapes`](Self::transform_shapes), the append-only edit
    /// vocabulary forces a remove-then-append per moved shape; ids are re-resolved
    /// each step because an earlier removal in the same cell shifts later slots. A
    /// zero offset leaves the shape (and its slot) untouched.
    fn apply_translations(
        &mut self,
        cell: &str,
        resolved: &[ResolvedShape],
        offsets: &[(i32, i32)],
    ) -> Result<(), AgentError> {
        for ((id, _, _), (dx, dy)) in resolved.iter().zip(offsets.iter().copied()) {
            if dx == 0 && dy == 0 {
                continue;
            }
            let slot = self
                .alloc
                .resolve(*id)
                .map(|r| r.slot)
                .ok_or_else(|| no_such_element(*id))?;
            let shape = self
                .document()
                .cell(cell)
                .and_then(|c| c.shapes.get(slot))
                .cloned()
                .ok_or_else(|| no_such_element(*id))?;
            let moved = translate_shape(&shape, dx, dy);
            self.commit(Edit::RemoveShape {
                cell: cell.to_owned(),
                index: slot,
            })?;
            self.alloc.remove(cell, ElementKind::Shape, slot);
            let new_slot = self.document().cell(cell).map_or(0, |c| c.shapes.len());
            self.commit(Edit::AddShape {
                cell: cell.to_owned(),
                shape: moved,
            })?;
            self.alloc.rebind(*id, cell, ElementKind::Shape, new_slot);
        }
        Ok(())
    }

    /// Offsets (grows or shrinks) each shape by `delta` DBU, replacing it on its own
    /// layer and keeping its id.
    ///
    /// Mirrors the editor's offset: a rectangle or polygon is offset by `delta` and
    /// its result replaces it; a path is skipped. A shrink that collapses a shape to
    /// nothing removes it (the id is retired). A zero delta is a no-op. Ids are
    /// re-resolved each step because a removal shifts later slots.
    fn offset_shapes(&mut self, ids: &[ElementId], delta: i32) -> CommandResult {
        if delta == 0 {
            return Ok(self.ok(ids.to_vec()));
        }
        let (cell, resolved) = self.resolve_shapes_in_cell(ids)?;
        let mut kept = Vec::with_capacity(resolved.len());
        for (id, _, shape) in &resolved {
            let Some(poly) = shape_as_polygon(shape) else {
                // A path is not a fill region; leave it in place, id unchanged.
                kept.push(*id);
                continue;
            };
            let result =
                reticle_geometry::offset(&[poly], delta).map_err(|e| map_geometry_err(&e))?;
            let result: Vec<Polygon> = result.into_iter().filter(|p| p.len() >= 3).collect();

            // Re-resolve: an earlier retirement in this cell may have shifted the slot.
            let slot = self
                .alloc
                .resolve(*id)
                .map(|r| r.slot)
                .ok_or_else(|| no_such_element(*id))?;
            let out_layer = shape.layer;
            self.commit(Edit::RemoveShape {
                cell: cell.clone(),
                index: slot,
            })?;
            self.alloc.remove(&cell, ElementKind::Shape, slot);
            if result.is_empty() {
                // A shrink erased the shape; the id is retired with it.
                continue;
            }
            // The first result polygon inherits this shape's id; any extra polygons
            // (an offset can split a shape) get fresh ids.
            for (i, poly) in result.into_iter().enumerate() {
                let new_slot = self.document().cell(&cell).map_or(0, |c| c.shapes.len());
                self.commit(Edit::AddShape {
                    cell: cell.clone(),
                    shape: DrawShape::new(out_layer, ShapeKind::Polygon(poly)),
                })?;
                if i == 0 {
                    self.alloc.rebind(*id, &cell, ElementKind::Shape, new_slot);
                    kept.push(*id);
                } else {
                    kept.push(self.alloc.allocate(&cell, ElementKind::Shape, new_slot));
                }
            }
        }
        Ok(self.ok(kept))
    }

    /// Builds a via stack in `cell`: a square cut on `cut_layer` centered at
    /// `center`, enclosed on `lower_layer` and `upper_layer` by the margins the
    /// technology's enclosure rules require (falling back to `default_enclosure`).
    ///
    /// Ports the editor's via-stack builder. The enclosure margin for each picked
    /// layer is the largest [`RuleKind::Enclosure`] rule of that layer enclosing the
    /// cut layer, or `default_enclosure` when the technology declares none. The three
    /// shapes are appended (cut, then lower and upper enclosure) and their ids
    /// returned in that order. A non-positive `cut_size` is an
    /// [`ErrorCode::InvalidArgument`] error.
    #[allow(clippy::too_many_arguments)]
    fn build_via_stack(
        &mut self,
        cell: &str,
        lower_layer: LayerArg,
        upper_layer: LayerArg,
        cut_layer: LayerArg,
        center: PointArg,
        cut_size: i32,
        default_enclosure: i32,
    ) -> CommandResult {
        self.require_cell(cell)?;
        if cut_size <= 0 {
            return Err(AgentError::invalid("via cut size must be positive"));
        }
        let lower = layer_id(lower_layer);
        let upper = layer_id(upper_layer);
        let cut = layer_id(cut_layer);
        let c = to_point(center);

        let half = cut_size / 2;
        let cut_rect = Rect::new(
            Point::new(c.x.saturating_sub(half), c.y.saturating_sub(half)),
            Point::new(
                c.x.saturating_add(cut_size - half),
                c.y.saturating_add(cut_size - half),
            ),
        );
        let rules = &self.document().technology().rules;
        let lower_margin = enclosure_of(rules, lower, cut).unwrap_or(default_enclosure);
        let upper_margin = enclosure_of(rules, upper, cut).unwrap_or(default_enclosure);

        let shapes = [
            DrawShape::new(cut, ShapeKind::Rect(cut_rect)),
            DrawShape::new(
                lower,
                ShapeKind::Rect(cut_rect.expanded(lower_margin.max(0))),
            ),
            DrawShape::new(
                upper,
                ShapeKind::Rect(cut_rect.expanded(upper_margin.max(0))),
            ),
        ];
        let mut new_ids = Vec::with_capacity(shapes.len());
        for shape in shapes {
            let slot = self.document().cell(cell).map_or(0, |c| c.shapes.len());
            self.commit(Edit::AddShape {
                cell: cell.to_owned(),
                shape,
            })?;
            new_ids.push(self.alloc.allocate(cell, ElementKind::Shape, slot));
        }
        Ok(self.ok(new_ids))
    }

    // ----- queries ----------------------------------------------------------

    fn query_shapes(
        &self,
        cell: &str,
        layer: Option<LayerArg>,
        region: Option<RectArg>,
    ) -> CommandResult {
        use reticle_geometry::Shape as _;
        let c = self
            .document()
            .cell(cell)
            .ok_or_else(|| AgentError::no_such_cell(cell))?;
        let layer_filter = layer.map(layer_id);
        let region_filter = region.map(to_rect);
        let mut shapes = Vec::new();
        for (slot, shape) in c.shapes.iter().enumerate() {
            if let Some(lf) = layer_filter
                && shape.layer != lf
            {
                continue;
            }
            let bbox = shape.bounding_box();
            if let Some(rf) = region_filter
                && !rf.intersects(&bbox)
                && rf.intersection(&bbox).is_none()
            {
                // Keep shapes that overlap or touch the region; a zero-area touch is
                // still a hit, but `intersects` needs positive area, so also admit an
                // exact intersection. A strictly disjoint box is dropped.
                if !touches(&rf, &bbox) {
                    continue;
                }
            }
            // The id for this slot, if one was allocated (shapes imported or created
            // before any id request may lack one; expose the slot regardless).
            let id = self.alloc.id_for(cell, ElementKind::Shape, slot);
            shapes.push(serde_json::json!({
                "id": id.map(|e| e.0),
                "slot": slot,
                "layer": { "layer": shape.layer.layer, "datatype": shape.layer.datatype },
                "kind": shape_kind_json(shape),
                "bbox": rect_json(bbox),
            }));
        }
        Ok(self.data(serde_json::json!({ "cell": cell, "shapes": shapes })))
    }

    fn get_cell_info(&self, cell: &str) -> CommandResult {
        let c = self
            .document()
            .cell(cell)
            .ok_or_else(|| AgentError::no_such_cell(cell))?;
        let bbox = self.doc.cell_bbox(cell);
        Ok(self.data(serde_json::json!({
            "cell": cell,
            "shapes": c.shapes.len(),
            "instances": c.instances.len(),
            "arrays": c.arrays.len(),
            "labels": c.labels.len(),
            "pins": c.pins.len(),
            "bbox": bbox.map(rect_json),
        })))
    }

    /// Lists the active technology's layer table. Cannot fail, so it returns the
    /// response directly; the dispatch arm wraps it in `Ok`.
    fn list_layers(&self) -> AgentResponse {
        let tech = self.document().technology();
        let layers: Vec<_> = tech
            .layers
            .iter()
            .map(|l| {
                serde_json::json!({
                    "layer": l.id.layer,
                    "datatype": l.id.datatype,
                    "name": l.name,
                    "color_rgba": l.color_rgba,
                    "visible": l.visible,
                })
            })
            .collect();
        self.data(serde_json::json!({
            "technology": tech.name,
            "dbu_per_micron": tech.dbu_per_micron,
            "layers": layers,
        }))
    }

    // ----- technology -------------------------------------------------------

    fn set_technology(&mut self, source: &str) -> CommandResult {
        let tech = reticle_io::parse_technology(source).map_err(|e| {
            AgentError::new(ErrorCode::InvalidArgument, format!("technology parse: {e}"))
        })?;
        // Rebuild the editor around a document carrying the new technology. Setting
        // technology is not an `Edit`, so it is applied to the document snapshot and
        // the editor is re-wrapped; the revision still advances so callers see a
        // change. The id map is preserved because cell contents are unchanged.
        let mut doc = self.document().clone();
        doc.set_technology(tech);
        self.doc = EditableDocument::new(doc);
        self.revision += 1;
        Ok(self.ok(vec![]))
    }

    // ----- DRC --------------------------------------------------------------

    fn run_drc(&mut self, cell: &str, region: Option<RectArg>) -> CommandResult {
        use reticle_model::RuleSet as _;
        self.require_cell(cell)?;
        let engine = reticle_drc::DrcEngine::new(self.document().technology().rules.clone());
        let violations = if let Some(r) = region {
            engine.check_region(self.document(), cell, to_rect(r))
        } else {
            engine.check_cell(self.document(), cell)
        };
        let value = violations_json(&violations);
        Ok(self.data(value))
    }

    /// Returns the standing violation set.
    ///
    /// Violations are not cached between runs in this session model; a fresh
    /// `run_drc` is the source of truth. This reports an empty set with a note so a
    /// caller expecting retained state still gets a well-formed answer. Cannot fail,
    /// so the dispatch arm wraps the response in `Ok`.
    fn get_violations(&self) -> AgentResponse {
        self.data(serde_json::json!({
            "violations": [],
            "note": "call run_drc; violations are returned by the run and not cached",
        }))
    }

    // ----- routing ----------------------------------------------------------

    fn route_net(
        &mut self,
        cell: &str,
        net: String,
        layer: LayerArg,
        terminals: &[PointArg],
    ) -> CommandResult {
        use reticle_model::Router as _;
        self.require_cell(cell)?;
        if terminals.len() < 2 {
            return Err(AgentError::invalid(
                "a net needs at least two terminals to route",
            ));
        }
        let request = reticle_model::RouteRequest {
            cell: cell.to_owned(),
            nets: vec![reticle_model::NetSpec {
                name: net,
                terminals: terminals.iter().map(|p| to_point(*p)).collect(),
                layer: layer_id(layer),
            }],
        };
        // The router appends wire shapes to the cell directly on a `Document`; run it
        // on a snapshot, then re-wrap the editor so the mutation is captured and the
        // revision advances. Newly emitted wires are not individually id-addressable.
        let mut doc = self.document().clone();
        let report = reticle_route::MazeRouter::new().route(&mut doc, &request);
        self.doc = EditableDocument::new(doc);
        self.revision += 1;
        Ok(self.data(serde_json::json!({
            "routed": report.routed,
            "failed": report.failed,
            "total_length_dbu": report.total_length_dbu,
        })))
    }

    // ----- extraction -------------------------------------------------------

    fn run_extract(&self, cell: &str) -> CommandResult {
        self.require_cell(cell)?;
        let netlist = reticle_extract::Extractor::new().extract(self.document(), cell);
        Ok(self.data(netlist_json(&netlist)))
    }

    /// Checks the cell against a connectivity intent spec.
    ///
    /// Parses `intent` as a JSON [`IntentSpec`](crate::IntentSpec), runs
    /// `reticle_extract::check_intent` over the current document, and returns the
    /// [`IntentReport`](crate::IntentReport) (opens and shorts) as structured data.
    /// A malformed spec is an `InvalidArgument` error.
    fn check_intent(&self, cell: &str, intent: &str) -> CommandResult {
        self.require_cell(cell)?;
        let spec: reticle_extract::IntentSpec = serde_json::from_str(intent).map_err(|e| {
            AgentError::new(
                ErrorCode::InvalidArgument,
                format!("invalid intent spec JSON: {e}"),
            )
        })?;
        let report = reticle_extract::check_intent(self.document(), cell, &spec);
        let value = serde_json::to_value(&report).map_err(|e| {
            AgentError::new(
                ErrorCode::EngineError,
                format!("serialize intent report: {e}"),
            )
        })?;
        Ok(self.data(value))
    }

    fn netlist_compare(&self, cell: &str, expected: &str) -> CommandResult {
        self.require_cell(cell)?;
        let expected_netlist = parse_expected_netlist(expected)?;
        let extracted = reticle_extract::Extractor::new().extract(self.document(), cell);
        let diff = reticle_extract::compare_netlists(&extracted, &expected_netlist);
        Ok(self.data(serde_json::json!({
            "equivalent": diff.is_empty(),
            "missing": diff.missing.iter().map(|p| serde_json::json!({"a": p.a, "b": p.b})).collect::<Vec<_>>(),
            "extra": diff.extra.iter().map(|p| serde_json::json!({"a": p.a, "b": p.b})).collect::<Vec<_>>(),
        })))
    }

    // ----- IO ---------------------------------------------------------------

    fn export_gds(&self) -> CommandResult {
        use reticle_model::Exporter as _;
        let bytes = reticle_io::Gds
            .export(self.document())
            .map_err(map_model_err)?;
        Ok(self.blob(bytes))
    }

    fn export_oasis(&self) -> CommandResult {
        use reticle_model::Exporter as _;
        let bytes = reticle_io::Oasis
            .export(self.document())
            .map_err(map_model_err)?;
        Ok(self.blob(bytes))
    }

    fn import_gds(&mut self, bytes: &[u8]) -> CommandResult {
        use reticle_model::Importer as _;
        let doc = reticle_io::Gds.import(bytes).map_err(map_model_err)?;
        // Replacing the document invalidates every prior id, so the allocator is
        // reset. The revision advances to signal the wholesale change.
        self.doc = EditableDocument::new(doc);
        self.alloc = crate::session::Allocator::new();
        self.revision += 1;
        Ok(self.ok(vec![]))
    }

    fn render_png(&mut self, region: RectArg, width: u32, height: u32) -> CommandResult {
        if width == 0 || height == 0 {
            return Err(AgentError::invalid(
                "render width and height must be positive",
            ));
        }
        let top = self.top_cell_name().ok_or_else(|| {
            AgentError::new(ErrorCode::InvalidArgument, "document has no cell to render")
        })?;
        // Offscreen rendering needs a blocking GPU context, which exists only on
        // native (`WgpuContext::new_blocking` is `#[cfg(not(wasm32))]`). On wasm the
        // command degrades to a clean engine error so the crate still compiles and a
        // wasm host can report "unsupported" instead of failing to build.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ctx) = reticle_render::WgpuContext::new_blocking() else {
                return Err(AgentError::new(
                    ErrorCode::EngineError,
                    "no GPU adapter available for rendering",
                ));
            };
            let camera = framing_camera(to_rect(region), width, height);
            let mut renderer = reticle_render::WgpuRenderer::new();
            let rgba = renderer.render_document_offscreen(
                &ctx,
                self.document(),
                &top,
                &camera,
                (width, height),
            );
            let png = encode_png(&rgba, width, height)?;
            Ok(self.blob(png))
        }
        #[cfg(target_arch = "wasm32")]
        {
            // `region` and `top` are consumed only by the native render path above.
            let _ = (region, top);
            Err(AgentError::new(
                ErrorCode::EngineError,
                "offscreen rendering is not available on wasm (no blocking GPU context)",
            ))
        }
    }

    // ----- session persistence ---------------------------------------------

    fn save_session(&self) -> CommandResult {
        let snapshot = self.snapshot_json();
        let bytes = serde_json::to_vec(&snapshot).map_err(|e| {
            AgentError::new(ErrorCode::EngineError, format!("serialize session: {e}"))
        })?;
        Ok(self.blob(bytes))
    }

    fn load_session(&mut self, snapshot: &str) -> CommandResult {
        let restored = Session::from_snapshot_str(snapshot)?;
        // Adopt the restored document, id map, and revision, but keep this session's
        // transcript so the `load_session` command records cleanly on top of it. The
        // revision continues upward from whichever session was further along, so it
        // never goes backwards.
        self.doc = restored.doc;
        self.alloc = restored.alloc;
        self.revision = self.revision.max(restored.revision) + 1;
        Ok(self.ok(vec![]))
    }

    // ----- helpers ----------------------------------------------------------

    /// A cell to render/export against: the document's first declared top cell, or
    /// any cell if no top is set (so a single-cell document still renders).
    fn top_cell_name(&self) -> Option<String> {
        let doc = self.document();
        doc.top_cells()
            .first()
            .cloned()
            .or_else(|| doc.cells().next().map(|c| c.name.clone()))
    }
}

// ===== free helpers =========================================================

/// Maps a [`reticle_model::ModelError`] onto an [`AgentError`] with a fitting code.
fn map_model_err(e: reticle_model::ModelError) -> AgentError {
    use reticle_model::ModelError;
    match e {
        ModelError::CellNotFound(n) => AgentError::no_such_cell(&n),
        ModelError::DuplicateCell(n) => {
            AgentError::new(ErrorCode::InvalidArgument, format!("duplicate cell `{n}`"))
        }
        ModelError::IndexOutOfBounds(i) => AgentError::new(
            ErrorCode::NoSuchElement,
            format!("element index {i} out of bounds"),
        ),
        ModelError::Geometry(g) => {
            AgentError::new(ErrorCode::InvalidArgument, format!("geometry: {g}"))
        }
        ModelError::Unsupported(why) => {
            AgentError::new(ErrorCode::EngineError, format!("unsupported: {why}"))
        }
        // `ModelError` is `#[non_exhaustive]`; a future variant maps to a generic
        // engine error rather than failing to compile.
        other => AgentError::new(ErrorCode::EngineError, format!("model error: {other}")),
    }
}

/// A `NoSuchElement` error naming the id.
fn no_such_element(id: ElementId) -> AgentError {
    AgentError::new(ErrorCode::NoSuchElement, format!("no such element {id}"))
}

/// Whether two rectangles overlap or merely touch (shared edge or corner), the
/// inclusive test used for region queries.
fn touches(a: &Rect, b: &Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

// ----- arg conversions ------------------------------------------------------

/// Converts a [`PointArg`] to an engine [`Point`].
fn to_point(p: PointArg) -> Point {
    Point::new(p.x, p.y)
}

/// Converts a [`RectArg`] to an engine [`Rect`] (normalizing corners).
fn to_rect(r: RectArg) -> Rect {
    Rect::new(to_point(r.min), to_point(r.max))
}

/// Converts a [`LayerArg`] to an engine [`LayerId`].
fn layer_id(l: LayerArg) -> LayerId {
    LayerId::new(l.layer, l.datatype)
}

/// Converts an optional [`EndcapArg`] to an engine [`Endcap`] (default flat).
fn to_endcap(e: Option<EndcapArg>) -> Endcap {
    match e {
        None | Some(EndcapArg::Flat) => Endcap::Flat,
        Some(EndcapArg::Square) => Endcap::Square,
        Some(EndcapArg::Round) => Endcap::Round,
    }
}

/// Converts an [`OrientationArg`] to an engine [`Orientation`].
fn to_orientation(o: OrientationArg) -> Orientation {
    match o {
        OrientationArg::R0 => Orientation::R0,
        OrientationArg::R90 => Orientation::R90,
        OrientationArg::R180 => Orientation::R180,
        OrientationArg::R270 => Orientation::R270,
        OrientationArg::MirrorX => Orientation::MirrorX,
        OrientationArg::MirrorX90 => Orientation::MirrorX90,
        OrientationArg::MirrorX180 => Orientation::MirrorX180,
        OrientationArg::MirrorX270 => Orientation::MirrorX270,
    }
}

/// Converts a [`TransformArg`] to an engine [`Transform`], validating the
/// magnification ratio (positive numerator and denominator that fit in `u32`).
fn to_transform(t: TransformArg) -> Result<Transform, AgentError> {
    if t.mag_den == 0 {
        return Err(AgentError::invalid(
            "magnification denominator must be non-zero",
        ));
    }
    if t.mag_num <= 0 || t.mag_den <= 0 {
        return Err(AgentError::invalid(
            "magnification numerator and denominator must be positive",
        ));
    }
    let num = u32::try_from(t.mag_num)
        .map_err(|_| AgentError::invalid("magnification numerator out of range"))?;
    let den = u32::try_from(t.mag_den)
        .map_err(|_| AgentError::invalid("magnification denominator out of range"))?;
    let magnification =
        Magnification::new(num, den).ok_or_else(|| AgentError::invalid("invalid magnification"))?;
    Ok(Transform {
        translation: Point::new(t.dx, t.dy),
        orientation: to_orientation(t.orientation),
        magnification,
    })
}

/// Transforms a drawable shape's geometry by `transform` (orient, magnify,
/// translate), mirroring the model's internal placement transform for a single
/// shape.
fn transform_shape(transform: &Transform, shape: &DrawShape) -> DrawShape {
    let kind = match &shape.kind {
        ShapeKind::Rect(rect) => {
            let corners = [
                rect.min,
                Point::new(rect.max.x, rect.min.y),
                rect.max,
                Point::new(rect.min.x, rect.max.y),
            ];
            let mapped = corners.into_iter().map(|c| transform.apply(c));
            ShapeKind::Rect(Rect::from_points(mapped).unwrap_or_default())
        }
        ShapeKind::Polygon(poly) => ShapeKind::Polygon(Polygon::new(
            poly.vertices()
                .iter()
                .map(|p| transform.apply(*p))
                .collect(),
        )),
        ShapeKind::Path(path) => ShapeKind::Path(Path::new(
            path.points().iter().map(|p| transform.apply(*p)).collect(),
            transform.magnification.scale(path.width()),
            path.endcap(),
        )),
    };
    DrawShape::new(shape.layer, kind)
}

// ----- Wave 2 op helpers ----------------------------------------------------

/// Maps a serde [`BooleanOpArg`] to the engine's [`BooleanOp`].
fn to_boolean_op(op: BooleanOpArg) -> BooleanOp {
    match op {
        BooleanOpArg::Union => BooleanOp::Union,
        BooleanOpArg::Intersection => BooleanOp::Intersection,
        BooleanOpArg::Difference => BooleanOp::Difference,
        BooleanOpArg::Xor => BooleanOp::Xor,
    }
}

/// Maps a [`reticle_geometry::GeometryError`] onto an [`AgentError`].
fn map_geometry_err(e: &reticle_geometry::GeometryError) -> AgentError {
    AgentError::new(ErrorCode::InvalidArgument, format!("geometry: {e}"))
}

/// Converts a fillable shape (rectangle or polygon) to a boolean-input polygon, or
/// `None` for a path (a stroke, not a fill region). Mirrors the editor's rule that
/// booleans and offsets run on filled geometry only.
fn shape_as_polygon(shape: &DrawShape) -> Option<Polygon> {
    match &shape.kind {
        ShapeKind::Rect(r) => Some(Polygon::from_rect(*r)),
        ShapeKind::Polygon(p) => Some(p.clone()),
        ShapeKind::Path(_) => None,
    }
}

/// Folds a boolean across polygons on one layer.
///
/// Union / intersection / xor are associative and accumulate pairwise; difference
/// subtracts every later polygon from the first (the usual "A minus the rest"). An
/// empty accumulator short-circuits. Degenerate (fewer than three vertices) inputs
/// are skipped.
fn fold_boolean(op: BooleanOp, polys: &[Polygon]) -> Result<Vec<Polygon>, AgentError> {
    let mut it = polys.iter().filter(|p| p.len() >= 3);
    let Some(first) = it.next() else {
        return Ok(Vec::new());
    };
    let mut acc = vec![first.clone()];
    for poly in it {
        let rhs = vec![poly.clone()];
        acc = polygon_boolean(op, &acc, &rhs).map_err(|e| map_geometry_err(&e))?;
        if acc.is_empty() {
            break;
        }
    }
    Ok(acc)
}

/// The combined bounding box of a set of shapes, or `None` if empty.
fn combined_bbox(shapes: &[DrawShape]) -> Option<Rect> {
    shapes
        .iter()
        .map(reticle_geometry::Shape::bounding_box)
        .reduce(|a, b| a.union(&b))
}

/// Rounds a widened DBU delta back to [`i32`], saturating (bounding-box deltas are
/// always in range in practice).
fn narrow_dbu(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// Midpoint of two coordinates, rounded to the nearest DBU.
fn midpoint_dbu(a: i32, b: i32) -> i32 {
    let m = f64::midpoint(f64::from(a), f64::from(b)).round();
    if m >= f64::from(i32::MAX) {
        i32::MAX
    } else if m <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        m as i32
    }
}

/// Per-shape `(dx, dy)` translations to align `shapes` within their combined
/// bounding box. Mirrors the editor's `align_offsets`. An empty selection or a
/// degenerate box yields all-zero offsets.
fn align_offsets(shapes: &[DrawShape], align: AlignArg) -> Vec<(i32, i32)> {
    let Some(bounds) = combined_bbox(shapes) else {
        return vec![(0, 0); shapes.len()];
    };
    let diff = |a: i32, b: i32| i64::from(a) - i64::from(b);
    shapes
        .iter()
        .map(|s| {
            let b = s.bounding_box();
            match align {
                AlignArg::Left => (narrow_dbu(diff(bounds.min.x, b.min.x)), 0),
                AlignArg::Right => (narrow_dbu(diff(bounds.max.x, b.max.x)), 0),
                AlignArg::Top => (0, narrow_dbu(diff(bounds.max.y, b.max.y))),
                AlignArg::Bottom => (0, narrow_dbu(diff(bounds.min.y, b.min.y))),
                AlignArg::CenterX => {
                    let target = midpoint_dbu(bounds.min.x, bounds.max.x);
                    let cur = midpoint_dbu(b.min.x, b.max.x);
                    (narrow_dbu(diff(target, cur)), 0)
                }
                AlignArg::CenterY => {
                    let target = midpoint_dbu(bounds.min.y, bounds.max.y);
                    let cur = midpoint_dbu(b.min.y, b.max.y);
                    (0, narrow_dbu(diff(target, cur)))
                }
            }
        })
        .collect()
}

/// Per-shape `(dx, dy)` translations to distribute `shapes` so adjacent edge-to-edge
/// gaps are equal along the axis. Mirrors the editor's `distribute_offsets`: the two
/// extreme shapes stay put and the inner shapes are respaced. Fewer than three
/// shapes yields all-zero offsets.
fn distribute_offsets(shapes: &[DrawShape], horizontal: bool) -> Vec<(i32, i32)> {
    let n = shapes.len();
    let mut offsets = vec![(0, 0); n];
    if n < 3 {
        return offsets;
    }
    let center = |idx: usize| -> f64 {
        let b = shapes[idx].bounding_box();
        if horizontal {
            f64::midpoint(f64::from(b.min.x), f64::from(b.max.x))
        } else {
            f64::midpoint(f64::from(b.min.y), f64::from(b.max.y))
        }
    };
    let extent = |idx: usize| -> f64 {
        let b = shapes[idx].bounding_box();
        if horizontal {
            f64::from(b.max.x) - f64::from(b.min.x)
        } else {
            f64::from(b.max.y) - f64::from(b.min.y)
        }
    };
    let lo = |idx: usize| -> f64 {
        let b = shapes[idx].bounding_box();
        if horizontal {
            f64::from(b.min.x)
        } else {
            f64::from(b.min.y)
        }
    };

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        center(a)
            .partial_cmp(&center(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let first = order[0];
    let last = order[n - 1];
    let span = (lo(last) + extent(last)) - lo(first);
    let total_extent: f64 = order.iter().map(|&i| extent(i)).sum();
    let gap = (span - total_extent) / (n as f64 - 1.0);

    let mut cursor = lo(first) + extent(first) + gap;
    for &idx in &order[1..n - 1] {
        let delta = round_dbu_f64(cursor - lo(idx));
        offsets[idx] = if horizontal { (delta, 0) } else { (0, delta) };
        cursor += extent(idx) + gap;
    }
    offsets
}

/// Rounds a float coordinate delta to the nearest DBU, clamped to the range.
fn round_dbu_f64(v: f64) -> i32 {
    let r = v.round();
    if r >= f64::from(i32::MAX) {
        i32::MAX
    } else if r <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        r as i32
    }
}

/// Translates a shape by `(dx, dy)` DBU, preserving its layer and kind (saturating
/// at the DBU range). Mirrors the editor's `translate_shape`.
fn translate_shape(shape: &DrawShape, dx: i32, dy: i32) -> DrawShape {
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

/// The minimum enclosure of `inner_layer` by `outer_layer` required by `rules`, in
/// DBU, or `None` if the technology declares no such rule. Mirrors the editor's
/// `enclosure_of`: matches an [`RuleKind::Enclosure`] rule whose primary `layer` is
/// the enclosing layer and whose `other_layer` is the enclosed layer, taking the
/// largest value when several apply.
fn enclosure_of(rules: &[Rule], outer_layer: LayerId, inner_layer: LayerId) -> Option<i32> {
    rules
        .iter()
        .filter(|rule| {
            rule.kind == RuleKind::Enclosure
                && rule.layer == outer_layer
                && rule.other_layer == Some(inner_layer)
        })
        .map(|rule| narrow_dbu(rule.value))
        .max()
}

// ----- JSON shaping ---------------------------------------------------------

/// A rectangle as `{min:{x,y}, max:{x,y}}`.
fn rect_json(r: Rect) -> serde_json::Value {
    serde_json::json!({
        "min": { "x": r.min.x, "y": r.min.y },
        "max": { "x": r.max.x, "y": r.max.y },
    })
}

/// A shape's geometry as a tagged JSON object.
fn shape_kind_json(shape: &DrawShape) -> serde_json::Value {
    match &shape.kind {
        ShapeKind::Rect(r) => serde_json::json!({ "type": "rect", "bbox": rect_json(*r) }),
        ShapeKind::Polygon(p) => serde_json::json!({
            "type": "polygon",
            "points": p.vertices().iter().map(|v| serde_json::json!({"x": v.x, "y": v.y})).collect::<Vec<_>>(),
        }),
        ShapeKind::Path(p) => serde_json::json!({
            "type": "path",
            "width": p.width(),
            "points": p.points().iter().map(|v| serde_json::json!({"x": v.x, "y": v.y})).collect::<Vec<_>>(),
        }),
    }
}

/// A `RuleKind` as a stable snake-case string.
fn rule_kind_str(kind: reticle_model::RuleKind) -> &'static str {
    use reticle_model::RuleKind;
    match kind {
        RuleKind::Width => "width",
        RuleKind::Spacing => "spacing",
        RuleKind::Enclosure => "enclosure",
        RuleKind::Extension => "extension",
        RuleKind::Notch => "notch",
        RuleKind::Area => "area",
        RuleKind::Density => "density",
        RuleKind::Angle => "angle",
        _ => "unknown",
    }
}

/// A list of DRC violations as structured JSON.
fn violations_json(violations: &[Violation]) -> serde_json::Value {
    let items: Vec<_> = violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "rule": v.rule,
                "kind": rule_kind_str(v.kind),
                "layer": { "layer": v.layer.layer, "datatype": v.layer.datatype },
                "other_layer": v.other_layer.map(|l| serde_json::json!({"layer": l.layer, "datatype": l.datatype})),
                "measured": v.measured,
                "required": v.required,
                "location": rect_json(v.location),
                "message": v.message,
            })
        })
        .collect();
    serde_json::json!({ "count": violations.len(), "violations": items })
}

/// A netlist as structured JSON: nets with names and member shape indices.
fn netlist_json(netlist: &reticle_extract::Netlist) -> serde_json::Value {
    let nets: Vec<_> = netlist
        .nets
        .iter()
        .map(|n| {
            serde_json::json!({
                "name": n.name,
                "shape_count": n.shape_count,
                "shapes": n.shapes,
            })
        })
        .collect();
    serde_json::json!({ "net_count": netlist.nets.len(), "nets": nets })
}

/// Parses an expected netlist from its serialized form.
///
/// Accepts `{"nets":[{"name":..,"shapes":[..]},..]}` (the same shape
/// [`netlist_json`] emits) or a bare array of nets. `Netlist`/`Net` do not derive
/// serde, so a local mirror is parsed and rebuilt via [`reticle_extract::Net::new`].
fn parse_expected_netlist(source: &str) -> Result<reticle_extract::Netlist, AgentError> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct NetIn {
        #[serde(default)]
        name: String,
        shapes: Vec<usize>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NetlistIn {
        Wrapped { nets: Vec<NetIn> },
        Bare(Vec<NetIn>),
    }

    let parsed: NetlistIn = serde_json::from_str(source).map_err(|e| {
        AgentError::new(
            ErrorCode::InvalidArgument,
            format!("expected netlist parse: {e}"),
        )
    })?;
    let (NetlistIn::Wrapped { nets: nets_in } | NetlistIn::Bare(nets_in)) = parsed;
    let nets = nets_in
        .into_iter()
        .map(|n| reticle_extract::Net::new(n.name, n.shapes))
        .collect();
    Ok(reticle_extract::Netlist::new(nets))
}

// ----- render helpers -------------------------------------------------------

/// A camera that frames `bbox` in a `width` x `height` image with a small margin,
/// mirroring the CLI's offscreen framing.
#[cfg(not(target_arch = "wasm32"))]
fn framing_camera(bbox: Rect, width: u32, height: u32) -> reticle_model::Camera {
    /// Fraction of the viewport left empty around the design.
    const MARGIN: f32 = 0.05;
    let cx = i64::midpoint(i64::from(bbox.min.x), i64::from(bbox.max.x)) as i32;
    let cy = i64::midpoint(i64::from(bbox.min.y), i64::from(bbox.max.y)) as i32;
    let center = Point::new(cx, cy);
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let span_x = (bbox.width().max(1)) as f32;
    let span_y = (bbox.height().max(1)) as f32;
    let ppd = ((w / span_x).min(h / span_y) * (1.0 - MARGIN)).max(f32::MIN_POSITIVE);
    let half_w = w / (2.0 * ppd);
    let half_h = h / (2.0 * ppd);
    let viewport = Rect::new(
        Point::new(
            (center.x as f32 - half_w) as i32,
            (center.y as f32 - half_h) as i32,
        ),
        Point::new(
            (center.x as f32 + half_w) as i32,
            (center.y as f32 + half_h) as i32,
        ),
    );
    reticle_model::Camera {
        center,
        pixels_per_dbu: ppd,
        viewport,
    }
}

/// Encodes tightly packed RGBA8 `pixels` as PNG bytes.
#[cfg(not(target_arch = "wasm32"))]
fn encode_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AgentError> {
    let buffer = image::RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| AgentError::new(ErrorCode::EngineError, "rendered buffer size mismatch"))?;
    let mut out = std::io::Cursor::new(Vec::new());
    buffer
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| AgentError::new(ErrorCode::EngineError, format!("png encode: {e}")))?;
    Ok(out.into_inner())
}

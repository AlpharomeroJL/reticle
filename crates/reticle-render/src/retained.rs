//! Retained, per-cell scene caching.
//!
//! The offscreen path in [`crate::geometry`] flattens the whole hierarchy to leaf
//! shapes and re-tessellates them every frame. That is fine for a one-shot render
//! but quadratically wasteful for an interactive loop over a large, deeply arrayed
//! design. [`RetainedScene`] fixes the steady-state cost:
//!
//! * Each cell's *own* geometry is tessellated **once** into a local-space
//!   [`CellChunk`] (rectangles kept as instances, polygons/paths as an indexed
//!   mesh), in the cell's own coordinate system with no placement transform baked
//!   in.
//! * Instances and arrays are expanded into a flat list of [`InstanceEntry`]s: one
//!   entry per placement, carrying the referenced cell plus the composed
//!   orient/scale/translate [`InstanceTransform`] that positions that cell's cached
//!   geometry in the top cell's coordinate system.
//! * A dirty set of cell names drives rebuilds, so editing one cell re-tessellates
//!   only that cell (and re-expands the instance list), not the entire scene.
//!
//! The vertex shader applies each entry's [`InstanceTransform`] to the cached
//! local-space vertices, so the same tessellation is reused for every placement of a
//! cell. This module produces the CPU-side data (cached chunks + instance-buffer
//! contents); uploading and drawing it lives in [`crate::pipelines`].

use crate::geometry::{MeshVertex, RectInstance, SceneGeometry};
use crate::palette::Palette;
use bytemuck::{Pod, Zeroable};
use reticle_geometry::Transform;
use reticle_model::Document;
use std::collections::{HashMap, HashSet};

/// Per-instance placement data uploaded to the GPU: an orientation code, a uniform
/// magnification, and an integer translation. Matches `InstanceTransform` in
/// `shapes.wgsl`.
///
/// The linear part (orientation + magnification) is reconstructed in the vertex
/// shader from `orientation_code` (see [`reticle_geometry::Orientation::code`]) and
/// `magnification`, then the result is offset by `translate`. Keeping the transform
/// as these three fields, rather than a baked 2x3 matrix, matches the exact integer
/// placement model and keeps the per-instance buffer to 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct InstanceTransform {
    /// Dihedral orientation code in `0..8` (see [`reticle_geometry::Orientation::code`]).
    pub orientation_code: u32,
    /// Uniform magnification factor applied after orientation.
    pub magnification: f32,
    /// Integer translation `(x, y)` in DBU, applied last.
    pub translate: [i32; 2],
}

impl InstanceTransform {
    /// The identity placement: no rotation, unit scale, no translation.
    pub const IDENTITY: Self = Self {
        orientation_code: 0,
        magnification: 1.0,
        translate: [0, 0],
    };

    /// Encodes a model [`Transform`] into its GPU form.
    #[must_use]
    pub fn from_transform(t: &Transform) -> Self {
        Self {
            orientation_code: t.orientation.code(),
            magnification: t.magnification.factor(),
            translate: [t.translation.x, t.translation.y],
        }
    }
}

impl Default for InstanceTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A retained instanced rectangle: a cell's local-space rect ([`RectInstance`])
/// fused with the placement [`InstanceTransform`] that positions it. Matches
/// `RectInstanceT` in `shapes.wgsl`; the vertex shader applies the transform, so one
/// cached rect serves every placement of its cell.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct RectInstanceT {
    /// Local-space minimum corner `(x, y)` in DBU.
    pub min_xy: [f32; 2],
    /// Local-space maximum corner `(x, y)` in DBU.
    pub max_xy: [f32; 2],
    /// Linear RGBA fill color.
    pub color: [f32; 4],
    /// Dihedral orientation code in `0..8`.
    pub orientation_code: u32,
    /// Uniform magnification applied after orientation.
    pub magnification: f32,
    /// Integer translation `(x, y)` in DBU, applied last.
    pub translate: [i32; 2],
}

impl RectInstanceT {
    /// Fuses a cached local-space rect with a placement transform.
    #[must_use]
    pub fn new(rect: &RectInstance, transform: InstanceTransform) -> Self {
        Self {
            min_xy: rect.min_xy,
            max_xy: rect.max_xy,
            color: rect.color,
            orientation_code: transform.orientation_code,
            magnification: transform.magnification,
            translate: transform.translate,
        }
    }
}

/// The CPU-side geometry expanded from a [`RetainedScene`] ready for GPU upload:
/// retained rect instances (each carrying its placement transform) plus a
/// transform-baked triangle mesh for polygons and paths.
///
/// The rect path keeps the cell's tessellation shared and expands only 32-byte
/// instance records per placement. The mesh path bakes the placement transform into
/// the vertices (meshes are comparatively rare, so duplicating their vertices per
/// placement is cheap and keeps the shader simple). Both are regenerated only when
/// the scene's structure changes, never per camera move.
#[derive(Clone, Debug, Default)]
pub struct ExpandedScene {
    /// One retained rect instance per (placement, cached rect).
    pub rects: Vec<RectInstanceT>,
    /// Transform-baked mesh vertices for polygons and paths.
    pub mesh_vertices: Vec<MeshVertex>,
    /// Indices into `mesh_vertices` (triangle list).
    pub mesh_indices: Vec<u32>,
}

impl ExpandedScene {
    /// Returns `true` if there is nothing to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty() && self.mesh_indices.is_empty()
    }

    /// The number of retained rect instances.
    #[must_use]
    pub fn rect_count(&self) -> usize {
        self.rects.len()
    }

    /// The number of mesh indices.
    #[must_use]
    pub fn index_count(&self) -> usize {
        self.mesh_indices.len()
    }
}

/// One cell's cached, local-space tessellation plus a dirty flag.
///
/// The geometry is the cell's own shapes only; instances and arrays it contains are
/// expanded separately into [`InstanceEntry`]s that reference *other* cells' chunks.
#[derive(Clone, Debug, Default)]
pub struct CellChunk {
    /// The cell's own geometry in its local coordinate system.
    geometry: SceneGeometry,
    /// Whether [`RetainedScene::rebuild`] should re-tessellate this chunk.
    dirty: bool,
}

impl CellChunk {
    /// The cached local-space geometry.
    #[must_use]
    pub fn geometry(&self) -> &SceneGeometry {
        &self.geometry
    }

    /// Whether this chunk is awaiting re-tessellation.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// One expanded placement: which cell to draw and the transform that positions its
/// cached geometry in the top cell's coordinate system.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceEntry {
    /// The referenced cell's name (a key into [`RetainedScene::chunk`]).
    pub cell: String,
    /// The composed placement transform for this entry.
    pub transform: InstanceTransform,
}

/// A retained cache of a document's per-cell tessellation and expanded instances.
///
/// Build it once with [`RetainedScene::new`], then keep it across frames. Call
/// [`RetainedScene::mark_dirty`] for each edited cell and [`RetainedScene::rebuild`]
/// to bring the cache up to date; only dirty cells are re-tessellated. The expanded
/// [`instances`](RetainedScene::instances) list is regenerated whenever the top cell
/// or any cell in its subtree changed.
#[derive(Clone, Debug, Default)]
pub struct RetainedScene {
    /// Per-cell cached local geometry, keyed by cell name.
    chunks: HashMap<String, CellChunk>,
    /// Cells whose chunks need re-tessellation on the next rebuild.
    dirty: HashSet<String>,
    /// The top cell the expanded instance list is currently built for.
    top_cell: String,
    /// The flattened placement list for `top_cell`: one entry per instance/array
    /// element, each referencing a cached chunk with its composed transform.
    instances: Vec<InstanceEntry>,
    /// Whether the instance list must be regenerated on the next rebuild.
    instances_dirty: bool,
}

impl RetainedScene {
    /// The recursion guard depth. Deeper than any realistic cell nesting; a chain
    /// longer than this (or a cycle the visited-set misses) is truncated rather than
    /// overflowing the stack.
    const MAX_DEPTH: usize = 256;

    /// Builds a retained scene for `top` from `doc`, tessellating every cell once
    /// and expanding `top`'s instance hierarchy.
    #[must_use]
    pub fn new(doc: &Document, top: &str, palette: &Palette) -> Self {
        let mut scene = Self {
            top_cell: top.to_owned(),
            instances_dirty: true,
            ..Self::default()
        };
        // Every cell starts dirty so the first rebuild tessellates the whole doc.
        for cell in doc.cells() {
            scene.dirty.insert(cell.name.clone());
        }
        scene.rebuild(doc, palette);
        scene
    }

    /// Marks a cell's chunk for re-tessellation on the next [`RetainedScene::rebuild`].
    ///
    /// Also flags the expanded instance list as stale, because a change to any cell
    /// in the top cell's subtree can change what is drawn. This is conservative but
    /// cheap: the instance walk is far cheaper than re-tessellating.
    pub fn mark_dirty(&mut self, cell: &str) {
        self.dirty.insert(cell.to_owned());
        self.instances_dirty = true;
    }

    /// Points the scene at a different top cell, forcing an instance-list rebuild.
    pub fn set_top_cell(&mut self, top: &str) {
        if self.top_cell != top {
            top.clone_into(&mut self.top_cell);
            self.instances_dirty = true;
        }
    }

    /// The top cell the instance list is built for.
    #[must_use]
    pub fn top_cell(&self) -> &str {
        &self.top_cell
    }

    /// Re-tessellates every dirty cell and, if needed, re-expands the instance list.
    ///
    /// Cells present in the cache but absent from `doc` are dropped. Cells in `doc`
    /// but absent from the cache are tessellated fresh. Only cells in the dirty set
    /// pay the tessellation cost; the rest keep their cached chunk untouched.
    pub fn rebuild(&mut self, doc: &Document, palette: &Palette) {
        // Drop chunks for cells that no longer exist.
        self.chunks.retain(|name, _| doc.cell(name).is_some());

        // Re-tessellate dirty cells (own geometry only, in local space).
        for name in self.dirty.drain().collect::<Vec<_>>() {
            let Some(cell) = doc.cell(&name) else {
                self.chunks.remove(&name);
                continue;
            };
            let geometry = SceneGeometry::build(&cell.shapes, palette);
            self.chunks.insert(
                name,
                CellChunk {
                    geometry,
                    dirty: false,
                },
            );
        }

        if self.instances_dirty {
            self.instances = self.expand_instances(doc);
            self.instances_dirty = false;
        }
    }

    /// The cached chunk for a cell, if it has been tessellated.
    #[must_use]
    pub fn chunk(&self, cell: &str) -> Option<&CellChunk> {
        self.chunks.get(cell)
    }

    /// The number of cached cell chunks.
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// The expanded per-placement instance entries for the current top cell.
    ///
    /// This is exactly the per-instance transform buffer the renderer uploads: one
    /// entry per instance and per array element, each naming the cached chunk to
    /// draw and the transform that places it.
    #[must_use]
    pub fn instances(&self) -> &[InstanceEntry] {
        &self.instances
    }

    /// Whether the expanded instance list is stale (a rebuild would regenerate it).
    #[must_use]
    pub fn instances_dirty(&self) -> bool {
        self.instances_dirty
    }

    /// Expands the cached chunks and instance list into GPU-ready geometry.
    ///
    /// Each placement contributes its cell's cached rects as retained instances (the
    /// transform rides along per instance) and its cached mesh vertices with the
    /// placement transform baked in. Call this only when the scene structure changed;
    /// the result is uploaded once and reused across camera moves.
    #[must_use]
    pub fn expand(&self) -> ExpandedScene {
        let mut out = ExpandedScene::default();
        for entry in &self.instances {
            let Some(chunk) = self.chunks.get(&entry.cell) else {
                continue;
            };
            let geom = &chunk.geometry;
            // Rects: share the cached instance, attach this placement's transform.
            out.rects.extend(
                geom.rects
                    .iter()
                    .map(|r| RectInstanceT::new(r, entry.transform)),
            );
            // Mesh: bake the transform into each vertex, rebasing indices.
            if !geom.mesh_indices.is_empty() {
                let base = u32::try_from(out.mesh_vertices.len()).unwrap_or(u32::MAX);
                out.mesh_vertices
                    .extend(geom.mesh_vertices.iter().map(|v| MeshVertex {
                        position: apply_instance(entry.transform, v.position),
                        color: v.color,
                    }));
                out.mesh_indices
                    .extend(geom.mesh_indices.iter().map(|i| i.saturating_add(base)));
            }
        }
        out
    }

    /// Walks the top cell's placement hierarchy into a flat instance list.
    ///
    /// The top cell's own geometry is emitted as one identity-transform entry, then
    /// every instance and array element is emitted with its composed transform. A
    /// cell reachable by several paths contributes one entry per path, which is what
    /// the renderer needs to draw each placement.
    fn expand_instances(&self, doc: &Document) -> Vec<InstanceEntry> {
        let mut out = Vec::new();
        let mut visiting = HashSet::new();
        self.walk(
            doc,
            &self.top_cell,
            Transform::IDENTITY,
            0,
            &mut visiting,
            &mut out,
        );
        out
    }

    /// Recursive worker for [`RetainedScene::expand_instances`]. `acc` is the
    /// transform from the current cell's coordinates up to the top cell.
    fn walk(
        &self,
        doc: &Document,
        name: &str,
        acc: Transform,
        depth: usize,
        visiting: &mut HashSet<String>,
        out: &mut Vec<InstanceEntry>,
    ) {
        if depth >= Self::MAX_DEPTH || !visiting.insert(name.to_owned()) {
            return; // depth cap or cycle guard
        }
        if let Some(cell) = doc.cell(name) {
            // The cell's own geometry, placed by the accumulated transform.
            if let Some(chunk) = self.chunks.get(name)
                && !chunk.geometry.is_empty()
            {
                out.push(InstanceEntry {
                    cell: name.to_owned(),
                    transform: InstanceTransform::from_transform(&acc),
                });
            }
            // Single placements: compose child-local -> parent (transform) -> top (acc).
            for inst in &cell.instances {
                let composed = inst.transform.then(&acc);
                self.walk(doc, &inst.cell, composed, depth + 1, visiting, out);
            }
            // Array placements: one composed transform per (row, column) element.
            for array in &cell.arrays {
                for row in 0..array.rows {
                    for col in 0..array.columns {
                        let dx = array.column_pitch.saturating_mul(i32_span(col));
                        let dy = array.row_pitch.saturating_mul(i32_span(row));
                        let element = Transform::translate(dx, dy)
                            .then(&array.transform)
                            .then(&acc);
                        self.walk(doc, &array.cell, element, depth + 1, visiting, out);
                    }
                }
            }
        }
        visiting.remove(name);
    }
}

/// The DBU offset multiplier for array index `i`, clamped into the coordinate range.
fn i32_span(i: u32) -> i32 {
    i32::try_from(i).unwrap_or(i32::MAX)
}

/// Applies an [`InstanceTransform`] to a local-space position, matching the vertex
/// shader's `vs_rect_retained` math exactly (orient, then scale, then translate) so
/// baked mesh vertices line up with shader-transformed rects.
fn apply_instance(t: InstanceTransform, local: [f32; 2]) -> [f32; 2] {
    let [x, y] = local;
    // Columns are the images of (1,0) and (0,1); must match `orientation_matrix`
    // in shapes.wgsl and `Orientation::apply` in reticle-geometry.
    let (ox, oy) = match t.orientation_code {
        0 => (x, y),
        1 => (-y, x),
        2 => (-x, -y),
        3 => (y, -x),
        4 => (x, -y),
        5 => (y, x),
        6 => (-x, y),
        _ => (-y, -x), // code 7: MirrorX270
    };
    [
        ox * t.magnification + t.translate[0] as f32,
        oy * t.magnification + t.translate[1] as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::{InstanceTransform, RetainedScene};
    use reticle_geometry::{LayerId, Orientation, Point, Rect, Transform};
    use reticle_model::{
        ArrayInstance, Cell, Document, DrawShape, Instance, ShapeKind, Technology,
    };

    /// An empty technology: leaf layers resolve through the fallback palette.
    fn palette() -> crate::Palette {
        crate::Palette::from_technology(&Technology::default())
    }

    fn rect_shape(x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            LayerId::new(0, 0),
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    /// A document with a leaf cell of one rect and a top cell that places it twice
    /// (one plain instance, one 2x3 array).
    fn hierarchy_doc() -> Document {
        let mut leaf = Cell::new("leaf");
        leaf.shapes.push(rect_shape(0, 0, 10, 10));

        let mut top = Cell::new("top");
        top.shapes.push(rect_shape(0, 0, 5, 5)); // top's own geometry
        top.instances.push(Instance {
            cell: "leaf".to_owned(),
            transform: Transform::translate(100, 0),
        });
        top.arrays.push(ArrayInstance {
            cell: "leaf".to_owned(),
            transform: Transform::translate(0, 100),
            columns: 3,
            rows: 2,
            column_pitch: 20,
            row_pitch: 20,
        });

        let mut doc = Document::new();
        doc.insert_cell(leaf);
        doc.insert_cell(top);
        doc.set_top_cells(vec!["top".to_owned()]);
        doc
    }

    #[test]
    fn identity_transform_encoding() {
        let t = InstanceTransform::from_transform(&Transform::IDENTITY);
        assert_eq!(t, InstanceTransform::IDENTITY);
        assert_eq!(t.orientation_code, 0);
        assert!((t.magnification - 1.0).abs() < 1e-6);
        assert_eq!(t.translate, [0, 0]);
    }

    #[test]
    fn transform_encoding_carries_orientation_and_translation() {
        let t = Transform {
            translation: Point::new(-7, 42),
            orientation: Orientation::R270,
            magnification: reticle_geometry::Magnification::new(3, 2).unwrap(),
        };
        let enc = InstanceTransform::from_transform(&t);
        assert_eq!(enc.orientation_code, Orientation::R270.code());
        assert_eq!(enc.translate, [-7, 42]);
        assert!((enc.magnification - 1.5).abs() < 1e-6);
    }

    #[test]
    fn caches_one_chunk_per_cell() {
        let doc = hierarchy_doc();
        let scene = RetainedScene::new(&doc, "top", &palette());
        assert_eq!(scene.chunk_count(), 2);
        assert!(scene.chunk("leaf").is_some());
        assert!(scene.chunk("top").is_some());
        // The leaf chunk holds exactly its own one rect, in local space.
        let leaf = scene.chunk("leaf").unwrap();
        assert_eq!(leaf.geometry().rects.len(), 1);
        assert!(!leaf.is_dirty());
    }

    #[test]
    fn instance_buffer_contents_for_known_scene() {
        let doc = hierarchy_doc();
        let scene = RetainedScene::new(&doc, "top", &palette());
        let entries = scene.instances();

        // 1 (top's own geometry) + 1 (instance) + 6 (2x3 array) = 8 placements.
        assert_eq!(entries.len(), 8);

        // First entry is the top cell's own geometry at identity.
        assert_eq!(entries[0].cell, "top");
        assert_eq!(entries[0].transform, InstanceTransform::IDENTITY);

        // The single instance places `leaf` at (100, 0).
        let inst = &entries[1];
        assert_eq!(inst.cell, "leaf");
        assert_eq!(inst.transform.translate, [100, 0]);

        // The 2x3 array places `leaf` at (0,100)+(col*20, row*20). Collect the array
        // translations (entries 2..8) and check the exact set.
        let mut array_translations: Vec<[i32; 2]> =
            entries[2..].iter().map(|e| e.transform.translate).collect();
        array_translations.sort_unstable();
        let mut expected = Vec::new();
        for row in 0..2 {
            for col in 0..3 {
                expected.push([col * 20, 100 + row * 20]);
            }
        }
        expected.sort_unstable();
        assert_eq!(array_translations, expected);
        // Every array element references the leaf chunk.
        assert!(entries[2..].iter().all(|e| e.cell == "leaf"));
    }

    #[test]
    fn mark_dirty_invalidates_only_that_chunk() {
        let doc = hierarchy_doc();
        let mut scene = RetainedScene::new(&doc, "top", &palette());

        // Edit the leaf: add a second rect, then mark it dirty and rebuild.
        let mut doc2 = doc.clone();
        doc2.cell_mut("leaf")
            .unwrap()
            .shapes
            .push(rect_shape(0, 0, 1, 1));
        scene.mark_dirty("leaf");
        assert!(scene.instances_dirty());
        scene.rebuild(&doc2, &palette());

        // The leaf chunk now has two rects; the top chunk is unchanged (one rect).
        assert_eq!(scene.chunk("leaf").unwrap().geometry().rects.len(), 2);
        assert_eq!(scene.chunk("top").unwrap().geometry().rects.len(), 1);
        assert!(!scene.instances_dirty());
    }

    #[test]
    fn rebuild_drops_removed_cells() {
        let doc = hierarchy_doc();
        let mut scene = RetainedScene::new(&doc, "top", &palette());
        assert_eq!(scene.chunk_count(), 2);

        // Remove the leaf cell entirely and mark the top cell dirty (its instances
        // now dangle). The chunk for `leaf` must be dropped on rebuild.
        let mut doc2 = doc.clone();
        doc2.remove_cell("leaf");
        scene.mark_dirty("leaf");
        scene.mark_dirty("top");
        scene.rebuild(&doc2, &palette());
        assert!(scene.chunk("leaf").is_none());
        assert_eq!(scene.chunk_count(), 1);
        // With leaf gone, only the top cell's own geometry remains as one entry.
        assert_eq!(scene.instances().len(), 1);
        assert_eq!(scene.instances()[0].cell, "top");
    }

    #[test]
    fn cycle_guard_terminates() {
        // A -> B -> A instance cycle must not loop forever.
        let mut a = Cell::new("a");
        a.shapes.push(rect_shape(0, 0, 1, 1));
        a.instances.push(Instance {
            cell: "b".to_owned(),
            transform: Transform::IDENTITY,
        });
        let mut b = Cell::new("b");
        b.instances.push(Instance {
            cell: "a".to_owned(),
            transform: Transform::IDENTITY,
        });
        let mut doc = Document::new();
        doc.insert_cell(a);
        doc.insert_cell(b);
        doc.set_top_cells(vec!["a".to_owned()]);

        let scene = RetainedScene::new(&doc, "a", &palette());
        // Terminates; `a`'s own geometry is emitted at least once.
        assert!(scene.instances().iter().any(|e| e.cell == "a"));
    }
}

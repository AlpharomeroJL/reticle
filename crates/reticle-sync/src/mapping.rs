//! The mapping between the native hierarchical model and the `yrs` CRDT.
//!
//! # Layout on the `yrs` document
//!
//! A [`SyncDocument`](crate::SyncDocument) keeps a single [`yrs::Doc`] with a flat
//! set of **root** shared maps, one per kind of record. Every map is keyed by a
//! globally-unique element id (`actor:counter`), never by a value a second peer
//! might also write:
//!
//! * `cells`, element id → cell name. A cell's existence.
//! * `shapes`, element id → shape record.
//! * `instances`, element id → instance record.
//! * `arrays`, element id → array record.
//! * `top_cells`, element id → top-cell name.
//!
//! Each record's value is a compact [`Any::Array`] whose **first element is the
//! owning cell name**, followed by the geometry/placement fields (the `cells` and
//! `top_cells` values are the bare name string).
//!
//! ## Why flat and id-keyed, and why it converges
//!
//! `yrs` deduplicates root shared types by name: the `shapes` map on peer *A* and
//! the `shapes` map on peer *B* are the *same* logical CRDT object once their
//! updates merge. Keeping every collection at the root avoids the "concurrent
//! nested-map creation" hazard (two peers each creating a per-cell `shapes` map
//! would race, and a last-write-wins merge would discard one peer's shapes).
//!
//! Keying by a unique id, rather than by cell name, avoids the *other* hazard:
//! two peers writing the **same map key** (`"cell0" → true`) creates competing
//! items that `yrs` resolves by tombstoning the loser, which does not always
//! survive a full-state (`encode_state_as_update`) round-trip cleanly. With unique
//! keys there are never two items on one key, so nothing is ever tombstoned by a
//! concurrent add; every peer's records simply coexist as a set.
//!
//! Materialization dedups cell names, groups records by their embedded cell name,
//! and sorts each group by element id, yielding a deterministic [`Document`] that
//! is identical on every converged peer regardless of update-exchange order.
//!
//! # Compact value encodings
//!
//! All coordinates are database units ([`i32`]) and round-trip exactly through the
//! `f64`-backed [`Any::Number`].
//!
//! * **Shape**: `[cell, layer, datatype, kind, ..geometry]`
//!   * `kind = 0` (Rect): `..geometry = [min_x, min_y, max_x, max_y]`
//!   * `kind = 1` (Polygon): `..geometry = [x0, y0, x1, y1, ..]`
//!   * `kind = 2` (Path): `..geometry = [width, endcap, endcap_ext, x0, y0, ..]`
//! * **Instance**: `[cell, tx, ty, orientation, mag_num, mag_den]`
//! * **Array**: `[cell, tx, ty, orientation, mag_num, mag_den, columns, rows,
//!   column_pitch, row_pitch]`

use crate::error::{Result, SyncError};
use reticle_geometry::{
    Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, Instance, ShapeKind};
use std::collections::BTreeMap;
use std::sync::Arc;
use yrs::{Any, Map, MapRef, ReadTxn, TransactionMut, WriteTxn};

/// Root set of cell names.
pub(crate) const CELLS: &str = "cells";
/// Root map of shape records keyed by element id.
pub(crate) const SHAPES: &str = "shapes";
/// Root map of instance records keyed by element id.
pub(crate) const INSTANCES: &str = "instances";
/// Root map of array records keyed by element id.
pub(crate) const ARRAYS: &str = "arrays";
/// Root set of top-cell names.
pub(crate) const TOP_CELLS: &str = "top_cells";

const KIND_RECT: i64 = 0;
const KIND_POLYGON: i64 = 1;
const KIND_PATH: i64 = 2;

// -----------------------------------------------------------------------------
// Small scalar helpers
// -----------------------------------------------------------------------------

/// Wraps an [`i64`] as a `yrs` number value.
fn num(v: i64) -> Any {
    Any::from(v)
}

/// Reads element `idx` of a `yrs` array as an [`i64`], if present and numeric.
fn arr_int(items: &[Any], idx: usize) -> Result<i64> {
    match items.get(idx) {
        Some(Any::Number(n)) => Ok(*n as i64),
        Some(Any::BigInt(n)) => Ok(*n),
        _ => Err(SyncError::Malformed("expected integer array element")),
    }
}

/// Reads a [`Point`] from elements `idx` and `idx + 1`.
fn arr_point(items: &[Any], idx: usize) -> Result<Point> {
    Ok(Point::new(arr_i32(items, idx)?, arr_i32(items, idx + 1)?))
}

/// Reads element `idx` of a `yrs` array as an [`i32`].
fn arr_i32(items: &[Any], idx: usize) -> Result<i32> {
    i32::try_from(arr_int(items, idx)?).map_err(|_| SyncError::Malformed("value out of i32 range"))
}

/// Reads element `idx` of a `yrs` array as a [`u32`].
fn arr_u32(items: &[Any], idx: usize) -> Result<u32> {
    u32::try_from(arr_int(items, idx)?).map_err(|_| SyncError::Malformed("value out of u32 range"))
}

/// Reads element `idx` of a `yrs` array as a [`u16`].
fn arr_u16(items: &[Any], idx: usize) -> Result<u16> {
    u16::try_from(arr_int(items, idx)?).map_err(|_| SyncError::Malformed("value out of u16 range"))
}

/// Reads element `idx` of a `yrs` array as an owned [`String`].
fn arr_string(items: &[Any], idx: usize) -> Result<String> {
    match items.get(idx) {
        Some(Any::String(s)) => Ok(s.to_string()),
        _ => Err(SyncError::Malformed("expected string array element")),
    }
}

/// A `yrs` string value from a `&str`.
fn ystr(s: &str) -> Any {
    Any::String(Arc::from(s))
}

// -----------------------------------------------------------------------------
// Orientation <-> integer
// -----------------------------------------------------------------------------

/// Encodes an [`Orientation`] as its stable D4 index (matching the proto enum).
fn orientation_to_int(o: Orientation) -> i64 {
    match o {
        Orientation::R0 => 0,
        Orientation::R90 => 1,
        Orientation::R180 => 2,
        Orientation::R270 => 3,
        Orientation::MirrorX => 4,
        Orientation::MirrorX90 => 5,
        Orientation::MirrorX180 => 6,
        Orientation::MirrorX270 => 7,
    }
}

/// Decodes a D4 index back into an [`Orientation`], defaulting to [`Orientation::R0`].
fn orientation_from_int(v: i64) -> Orientation {
    match v {
        1 => Orientation::R90,
        2 => Orientation::R180,
        3 => Orientation::R270,
        4 => Orientation::MirrorX,
        5 => Orientation::MirrorX90,
        6 => Orientation::MirrorX180,
        7 => Orientation::MirrorX270,
        _ => Orientation::R0,
    }
}

// -----------------------------------------------------------------------------
// Endcap <-> (tag, extension)
// -----------------------------------------------------------------------------

/// Encodes an [`Endcap`] as a `(tag, extension)` pair (matching the proto enum).
fn endcap_to_parts(cap: Endcap) -> (i64, i64) {
    match cap {
        Endcap::Flat => (0, 0),
        Endcap::Square => (1, 0),
        Endcap::Round => (2, 0),
        Endcap::Custom(ext) => (3, i64::from(ext)),
    }
}

/// Decodes an [`Endcap`] from its `(tag, extension)` pair.
fn endcap_from_parts(tag: i64, ext: i32) -> Endcap {
    match tag {
        1 => Endcap::Square,
        2 => Endcap::Round,
        3 => Endcap::Custom(ext),
        _ => Endcap::Flat,
    }
}

// -----------------------------------------------------------------------------
// Magnification <-> (num, den)
// -----------------------------------------------------------------------------

/// Greatest common divisor of two unsigned integers (Euclid).
fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Extracts a magnification's `(num, den)`, normalizing unity to `(1, 1)`.
///
/// [`Magnification`] keeps its numerator and denominator private, exposing only
/// unity detection and [`Magnification::scale`]. Unity, the case for essentially
/// all placements, is recovered exactly. A non-unity ratio is reconstructed by
/// scaling a high-precision reference (`2^20`) and reducing the resulting
/// fraction; the encoding is deterministic and idempotent (re-encoding a decoded
/// value yields the same pair), so it never perturbs convergence, at the cost of
/// sub-parts-per-million rounding on exotic non-unity factors.
fn magnification_parts(mag: Magnification) -> (u32, u32) {
    /// High-precision reference denominator for rational reconstruction.
    const REF: u32 = 1 << 20;
    if mag.is_unity() {
        return (1, 1);
    }
    let scaled = u32::try_from(mag.scale(REF as i32).max(0)).unwrap_or(0);
    let divisor = gcd(scaled, REF).max(1);
    (scaled / divisor, REF / divisor)
}

// -----------------------------------------------------------------------------
// Transform helpers (shared by instance and array)
// -----------------------------------------------------------------------------

/// Appends the five transform fields (translation, orientation, magnification) of
/// `transform` onto `out`.
fn push_transform(out: &mut Vec<Any>, transform: &Transform) {
    let (mag_num, mag_den) = magnification_parts(transform.magnification);
    out.push(num(i64::from(transform.translation.x)));
    out.push(num(i64::from(transform.translation.y)));
    out.push(num(orientation_to_int(transform.orientation)));
    out.push(num(i64::from(mag_num)));
    out.push(num(i64::from(mag_den)));
}

/// Reads a [`Transform`] from `items` starting at index `start` (five fields).
fn read_transform(items: &[Any], start: usize) -> Result<Transform> {
    let translation = arr_point(items, start)?;
    let orientation = orientation_from_int(arr_int(items, start + 2)?);
    let mag_num = arr_u32(items, start + 3)?;
    let mag_den = arr_u32(items, start + 4)?;
    let magnification = Magnification::new(mag_num, mag_den).unwrap_or_default();
    Ok(Transform {
        translation,
        orientation,
        magnification,
    })
}

// -----------------------------------------------------------------------------
// Shape encode / decode (with owning cell name in element 0)
// -----------------------------------------------------------------------------

/// Encodes a [`DrawShape`] into a record whose first element is `cell`.
pub(crate) fn encode_shape(cell: &str, shape: &DrawShape) -> Any {
    let mut out = vec![
        ystr(cell),
        num(i64::from(shape.layer.layer)),
        num(i64::from(shape.layer.datatype)),
    ];
    match &shape.kind {
        ShapeKind::Rect(r) => {
            out.push(num(KIND_RECT));
            out.extend([
                num(i64::from(r.min.x)),
                num(i64::from(r.min.y)),
                num(i64::from(r.max.x)),
                num(i64::from(r.max.y)),
            ]);
        }
        ShapeKind::Polygon(p) => {
            out.push(num(KIND_POLYGON));
            for v in p.vertices() {
                out.push(num(i64::from(v.x)));
                out.push(num(i64::from(v.y)));
            }
        }
        ShapeKind::Path(p) => {
            out.push(num(KIND_PATH));
            let (tag, ext) = endcap_to_parts(p.endcap());
            out.push(num(i64::from(p.width())));
            out.push(num(tag));
            out.push(num(ext));
            for pt in p.points() {
                out.push(num(i64::from(pt.x)));
                out.push(num(i64::from(pt.y)));
            }
        }
    }
    Any::Array(Arc::from(out))
}

/// Decodes a `(cell, shape)` record from a shape value.
fn decode_shape(value: &Any) -> Result<(String, DrawShape)> {
    let Any::Array(items) = value else {
        return Err(SyncError::Malformed("shape value is not an array"));
    };
    let cell = arr_string(items, 0)?;
    let layer = LayerId::new(arr_u16(items, 1)?, arr_u16(items, 2)?);
    let kind = match arr_int(items, 3)? {
        KIND_RECT => {
            let min = arr_point(items, 4)?;
            let max = arr_point(items, 6)?;
            ShapeKind::Rect(Rect::new(min, max))
        }
        KIND_POLYGON => {
            let mut verts = Vec::new();
            let mut idx = 4;
            while idx + 1 < items.len() {
                verts.push(arr_point(items, idx)?);
                idx += 2;
            }
            ShapeKind::Polygon(Polygon::new(verts))
        }
        KIND_PATH => {
            let width = arr_i32(items, 4)?;
            let tag = arr_int(items, 5)?;
            let ext = arr_i32(items, 6)?;
            let mut pts = Vec::new();
            let mut idx = 7;
            while idx + 1 < items.len() {
                pts.push(arr_point(items, idx)?);
                idx += 2;
            }
            ShapeKind::Path(Path::new(pts, width, endcap_from_parts(tag, ext)))
        }
        _ => return Err(SyncError::Malformed("unknown shape kind tag")),
    };
    Ok((cell, DrawShape::new(layer, kind)))
}

// -----------------------------------------------------------------------------
// Instance / array encode / decode
// -----------------------------------------------------------------------------

/// Encodes an [`Instance`] into a record whose first element is `cell`.
pub(crate) fn encode_instance(cell: &str, inst: &Instance) -> Any {
    let mut out = vec![ystr(cell), ystr(&inst.cell)];
    push_transform(&mut out, &inst.transform);
    Any::Array(Arc::from(out))
}

/// Decodes a `(cell, instance)` record.
fn decode_instance(value: &Any) -> Result<(String, Instance)> {
    let Any::Array(items) = value else {
        return Err(SyncError::Malformed("instance value is not an array"));
    };
    let cell = arr_string(items, 0)?;
    let target = arr_string(items, 1)?;
    let transform = read_transform(items, 2)?;
    Ok((
        cell,
        Instance {
            cell: target,
            transform,
        },
    ))
}

/// Encodes an [`ArrayInstance`] into a record whose first element is `cell`.
pub(crate) fn encode_array(cell: &str, array: &ArrayInstance) -> Any {
    let mut out = vec![ystr(cell), ystr(&array.cell)];
    push_transform(&mut out, &array.transform);
    out.push(num(i64::from(array.columns)));
    out.push(num(i64::from(array.rows)));
    out.push(num(i64::from(array.column_pitch)));
    out.push(num(i64::from(array.row_pitch)));
    Any::Array(Arc::from(out))
}

/// Decodes a `(cell, array)` record.
fn decode_array(value: &Any) -> Result<(String, ArrayInstance)> {
    let Any::Array(items) = value else {
        return Err(SyncError::Malformed("array value is not an array"));
    };
    let cell = arr_string(items, 0)?;
    let target = arr_string(items, 1)?;
    let transform = read_transform(items, 2)?;
    let columns = arr_u32(items, 7)?;
    let rows = arr_u32(items, 8)?;
    let column_pitch = arr_i32(items, 9)?;
    let row_pitch = arr_i32(items, 10)?;
    Ok((
        cell,
        ArrayInstance {
            cell: target,
            transform,
            columns,
            rows,
            column_pitch,
            row_pitch,
        },
    ))
}

// -----------------------------------------------------------------------------
// Writes
// -----------------------------------------------------------------------------

/// Returns `true` if `map` already holds an entry whose value is the bare string
/// `name`.
fn name_present<T: ReadTxn>(txn: &T, map: &MapRef, name: &str) -> bool {
    map.iter(txn)
        .any(|(_, value)| matches!(value, yrs::Out::Any(Any::String(s)) if s.as_ref() == name))
}

/// Records the existence of a cell (idempotent per peer): inserts a `cells` entry
/// mapping a fresh id to the cell name, unless this peer already recorded it.
///
/// Two peers recording the same new cell produce two distinct id-keyed entries
/// with equal names; materialization dedups them, so no data is lost and no map
/// key ever collides.
pub(crate) fn ensure_cell(
    txn: &mut TransactionMut,
    name: &str,
    next_id: &mut dyn FnMut() -> String,
) {
    let cells = txn.get_or_insert_map(CELLS);
    if !name_present(txn, &cells, name) {
        cells.insert(txn, next_id(), ystr(name));
    }
}

/// Seeds an entire [`Cell`] into the CRDT under fresh element ids from `next_id`.
pub(crate) fn write_cell(
    txn: &mut TransactionMut,
    cell: &Cell,
    next_id: &mut dyn FnMut() -> String,
) {
    ensure_cell(txn, &cell.name, next_id);
    let shapes = txn.get_or_insert_map(SHAPES);
    for shape in &cell.shapes {
        let id = next_id();
        shapes.insert(txn, id, encode_shape(&cell.name, shape));
    }
    let instances = txn.get_or_insert_map(INSTANCES);
    for inst in &cell.instances {
        let id = next_id();
        instances.insert(txn, id, encode_instance(&cell.name, inst));
    }
    let arrays = txn.get_or_insert_map(ARRAYS);
    for array in &cell.arrays {
        let id = next_id();
        arrays.insert(txn, id, encode_array(&cell.name, array));
    }
}

/// Inserts a single shape record into a cell under `id`, creating the cell's
/// existence entry (with `cell_id`) if this peer had not recorded it.
pub(crate) fn insert_shape(
    txn: &mut TransactionMut,
    cell: &str,
    id: &str,
    shape: &DrawShape,
    next_id: &mut dyn FnMut() -> String,
) {
    ensure_cell(txn, cell, next_id);
    let shapes = txn.get_or_insert_map(SHAPES);
    shapes.insert(txn, id, encode_shape(cell, shape));
}

/// Inserts a single instance record into a cell under `id`.
pub(crate) fn insert_instance(
    txn: &mut TransactionMut,
    cell: &str,
    id: &str,
    inst: &Instance,
    next_id: &mut dyn FnMut() -> String,
) {
    ensure_cell(txn, cell, next_id);
    let instances = txn.get_or_insert_map(INSTANCES);
    instances.insert(txn, id, encode_instance(cell, inst));
}

/// Inserts a single array record into a cell under `id`.
pub(crate) fn insert_array(
    txn: &mut TransactionMut,
    cell: &str,
    id: &str,
    array: &ArrayInstance,
    next_id: &mut dyn FnMut() -> String,
) {
    ensure_cell(txn, cell, next_id);
    let arrays = txn.get_or_insert_map(ARRAYS);
    arrays.insert(txn, id, encode_array(cell, array));
}

/// Removes a cell and every record that belongs to it from the CRDT.
pub(crate) fn remove_cell(txn: &mut TransactionMut, name: &str) {
    // Every root map is id-keyed; delete each entry that names `name`.
    remove_named(txn, CELLS, name, true);
    remove_named(txn, TOP_CELLS, name, true);
    remove_named(txn, SHAPES, name, false);
    remove_named(txn, INSTANCES, name, false);
    remove_named(txn, ARRAYS, name, false);
}

/// Deletes every entry of root map `root` that refers to `name`.
///
/// When `bare` is set the value is the name string itself (`cells`, `top_cells`);
/// otherwise the name is the first element of the record array.
fn remove_named(txn: &mut TransactionMut, root: &str, name: &str, bare: bool) {
    let map = txn.get_or_insert_map(root);
    let doomed: Vec<String> = map
        .iter(txn)
        .filter_map(|(id, value)| {
            let matches = if bare {
                matches!(&value, yrs::Out::Any(Any::String(s)) if s.as_ref() == name)
            } else {
                matches!(&value, yrs::Out::Any(Any::Array(items))
                    if matches!(items.first(), Some(Any::String(s)) if s.as_ref() == name))
            };
            matches.then(|| id.to_owned())
        })
        .collect();
    for id in doomed {
        map.remove(txn, &id);
    }
}

/// Marks (or clears) a cell as a top cell in the id-keyed `top_cells` set.
pub(crate) fn set_top_cell(
    txn: &mut TransactionMut,
    name: &str,
    is_top: bool,
    next_id: &mut dyn FnMut() -> String,
) {
    if is_top {
        let tops = txn.get_or_insert_map(TOP_CELLS);
        if !name_present(txn, &tops, name) {
            tops.insert(txn, next_id(), ystr(name));
        }
    } else {
        remove_named(txn, TOP_CELLS, name, true);
    }
}

// -----------------------------------------------------------------------------
// Materialization (CRDT -> Document)
// -----------------------------------------------------------------------------

/// Materializes the whole CRDT into a fresh [`Document`].
///
/// Records are grouped by their embedded cell name and each group is sorted by
/// element id, so the output is deterministic and independent of insertion or
/// merge order, this is what lets two converged peers produce equal
/// [`Document`]s.
///
/// # Errors
///
/// Returns [`SyncError::Malformed`] if any stored value does not match its
/// expected encoding.
pub(crate) fn materialize<T: ReadTxn>(txn: &T, roots: &Roots) -> Result<Document> {
    let mut document = Document::new();

    // Every distinct cell name (id-keyed, possibly duplicated across peers)
    // becomes an initially-empty cell.
    if let Some(cells) = &roots.cells {
        for (_, value) in cells.iter(txn) {
            let yrs::Out::Any(Any::String(name)) = value else {
                return Err(SyncError::Malformed("cell entry is not a string"));
            };
            if document.cell(&name).is_none() {
                document.insert_cell(Cell::new(name.to_string()));
            }
        }
    }

    // Shapes, grouped by cell and ordered by id.
    if let Some(shapes) = &roots.shapes {
        let mut by_cell: BTreeMap<String, Vec<(String, DrawShape)>> = BTreeMap::new();
        for (id, value) in shapes.iter(txn) {
            let yrs::Out::Any(any) = value else {
                return Err(SyncError::Malformed("shape value is not a scalar"));
            };
            let (cell, shape) = decode_shape(&any)?;
            by_cell
                .entry(cell)
                .or_default()
                .push((id.to_owned(), shape));
        }
        for (cell_name, mut records) in by_cell {
            let cell = ensure_document_cell(&mut document, &cell_name);
            records.sort_by(|a, b| a.0.cmp(&b.0));
            cell.shapes = records.into_iter().map(|(_, s)| s).collect();
        }
    }

    // Instances, grouped by cell and ordered by id.
    if let Some(instances) = &roots.instances {
        let mut by_cell: BTreeMap<String, Vec<(String, Instance)>> = BTreeMap::new();
        for (id, value) in instances.iter(txn) {
            let yrs::Out::Any(any) = value else {
                return Err(SyncError::Malformed("instance value is not a scalar"));
            };
            let (cell, inst) = decode_instance(&any)?;
            by_cell.entry(cell).or_default().push((id.to_owned(), inst));
        }
        for (cell_name, mut records) in by_cell {
            let cell = ensure_document_cell(&mut document, &cell_name);
            records.sort_by(|a, b| a.0.cmp(&b.0));
            cell.instances = records.into_iter().map(|(_, i)| i).collect();
        }
    }

    // Arrays, grouped by cell and ordered by id.
    if let Some(arrays) = &roots.arrays {
        let mut by_cell: BTreeMap<String, Vec<(String, ArrayInstance)>> = BTreeMap::new();
        for (id, value) in arrays.iter(txn) {
            let yrs::Out::Any(any) = value else {
                return Err(SyncError::Malformed("array value is not a scalar"));
            };
            let (cell, array) = decode_array(&any)?;
            by_cell
                .entry(cell)
                .or_default()
                .push((id.to_owned(), array));
        }
        for (cell_name, mut records) in by_cell {
            let cell = ensure_document_cell(&mut document, &cell_name);
            records.sort_by(|a, b| a.0.cmp(&b.0));
            cell.arrays = records.into_iter().map(|(_, a)| a).collect();
        }
    }

    // Top cells that still resolve to an existing cell: dedup and sort for
    // determinism.
    if let Some(tops) = &roots.top_cells {
        let mut top_names: Vec<String> = Vec::new();
        for (_, value) in tops.iter(txn) {
            if let yrs::Out::Any(Any::String(name)) = value {
                let name = name.to_string();
                if document.cell(&name).is_some() && !top_names.contains(&name) {
                    top_names.push(name);
                }
            }
        }
        top_names.sort();
        top_names.dedup();
        document.set_top_cells(top_names);
    }

    Ok(document)
}

/// Returns a mutable reference to `name` in `document`, creating an empty cell if
/// a record referenced a cell whose existence marker was not (yet) present.
fn ensure_document_cell<'d>(document: &'d mut Document, name: &str) -> &'d mut Cell {
    if document.cell(name).is_none() {
        document.insert_cell(Cell::new(name));
    }
    document
        .cell_mut(name)
        .expect("cell was just inserted if missing")
}

/// A snapshot of the CRDT root maps, resolved once per materialization.
#[derive(Debug)]
pub(crate) struct Roots {
    /// The `cells` existence set, if it has been created.
    pub cells: Option<MapRef>,
    /// The `shapes` record map, if it has been created.
    pub shapes: Option<MapRef>,
    /// The `instances` record map, if it has been created.
    pub instances: Option<MapRef>,
    /// The `arrays` record map, if it has been created.
    pub arrays: Option<MapRef>,
    /// The `top_cells` set, if it has been created.
    pub top_cells: Option<MapRef>,
}

impl Roots {
    /// Resolves every root map from a read transaction (each may be absent until
    /// first written).
    pub(crate) fn resolve<T: ReadTxn>(txn: &T) -> Self {
        Self {
            cells: txn.get_map(CELLS),
            shapes: txn.get_map(SHAPES),
            instances: txn.get_map(INSTANCES),
            arrays: txn.get_map(ARRAYS),
            top_cells: txn.get_map(TOP_CELLS),
        }
    }
}

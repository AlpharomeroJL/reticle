//! The frozen agent command vocabulary.
//!
//! [`AgentCommand`] is the full serde command set over the engine, tagged by an
//! `op` field so it round-trips as JSON. It is frozen here in Wave 0 and applied
//! against a session in a later wave. Every mutating command returns the affected
//! [`ElementId`]s and the new document revision (see [`crate::AgentResponse`]).

use serde::{Deserialize, Serialize};

use crate::ElementId;
use crate::args::{EndcapArg, LayerArg, PointArg, RectArg, TransformArg};

/// A single serializable command over the engine, tagged by its `op`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentCommand {
    /// Create a new, empty cell. Errors on a duplicate name.
    CreateCell {
        /// The new cell's name.
        name: String,
    },
    /// Remove a cell by name.
    DeleteCell {
        /// The cell to remove.
        name: String,
    },
    /// Add a rectangle to a cell.
    AddRect {
        /// Target cell.
        cell: String,
        /// Layer and datatype.
        layer: LayerArg,
        /// The rectangle in database units.
        rect: RectArg,
    },
    /// Add a polygon (at least three vertices) to a cell.
    AddPolygon {
        /// Target cell.
        cell: String,
        /// Layer and datatype.
        layer: LayerArg,
        /// The polygon vertices in order.
        points: Vec<PointArg>,
    },
    /// Add a path (at least two vertices) to a cell.
    AddPath {
        /// Target cell.
        cell: String,
        /// Layer and datatype.
        layer: LayerArg,
        /// Path width in database units.
        width: i32,
        /// The path spine vertices in order.
        points: Vec<PointArg>,
        /// End-cap style; defaults to flat when omitted.
        #[serde(default)]
        endcap: Option<EndcapArg>,
    },
    /// Place a single instance of a child cell.
    PlaceInstance {
        /// Parent cell that gains the placement.
        cell: String,
        /// The child cell to place.
        child: String,
        /// The placement transform.
        transform: TransformArg,
    },
    /// Place a regular array of a child cell.
    PlaceArray {
        /// Parent cell that gains the array.
        cell: String,
        /// The child cell to array.
        child: String,
        /// Transform of the first element.
        transform: TransformArg,
        /// Number of columns.
        columns: u32,
        /// Number of rows.
        rows: u32,
        /// Column pitch in database units.
        column_pitch: i32,
        /// Row pitch in database units.
        row_pitch: i32,
    },
    /// Apply a transform to a set of existing elements.
    TransformShapes {
        /// The elements to transform.
        ids: Vec<ElementId>,
        /// The transform to apply.
        transform: TransformArg,
    },
    /// Delete a set of existing elements.
    DeleteShapes {
        /// The elements to delete.
        ids: Vec<ElementId>,
    },
    /// Query shapes in a cell, optionally filtered by layer and region.
    QueryShapes {
        /// The cell to query.
        cell: String,
        /// Restrict to this layer, if given.
        #[serde(default)]
        layer: Option<LayerArg>,
        /// Restrict to shapes overlapping this region, if given.
        #[serde(default)]
        region: Option<RectArg>,
    },
    /// Get summary information about a cell (counts and bounding box).
    GetCellInfo {
        /// The cell to summarize.
        cell: String,
    },
    /// List the layers in the active technology.
    ListLayers,
    /// Replace the active technology from a technology-file source.
    SetTechnology {
        /// The technology-file text.
        source: String,
    },
    /// Run design-rule checking over a cell, optionally scoped to a region.
    RunDrc {
        /// The cell to check.
        cell: String,
        /// Restrict the check to this region, if given.
        #[serde(default)]
        region: Option<RectArg>,
    },
    /// Return the violations from the most recent DRC run.
    GetViolations,
    /// Route a net between terminals on a layer.
    RouteNet {
        /// The cell to route in.
        cell: String,
        /// The net name.
        net: String,
        /// The routing layer.
        layer: LayerArg,
        /// The terminal points to connect.
        terminals: Vec<PointArg>,
    },
    /// Extract connectivity (a netlist) from a cell.
    RunExtract {
        /// The cell to extract.
        cell: String,
    },
    /// Check the cell against a named intent spec (connectivity intent).
    CheckIntent {
        /// The cell to check.
        cell: String,
        /// The intent spec, as its serialized form.
        intent: String,
    },
    /// Compare the extracted netlist against an expected netlist.
    NetlistCompare {
        /// The cell whose extraction is compared.
        cell: String,
        /// The expected netlist, as its serialized form.
        expected: String,
    },
    /// Export the document as GDSII bytes.
    ExportGds,
    /// Export the document as OASIS bytes.
    ExportOasis,
    /// Import a GDSII document, replacing the session document.
    ImportGds {
        /// The GDSII bytes.
        bytes: Vec<u8>,
    },
    /// Render a region of the document to a PNG.
    RenderPng {
        /// The region to render, in database units.
        region: RectArg,
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
    },
    /// Save the session (document plus transcript) to its store.
    SaveSession,
    /// Load a session from a serialized snapshot.
    LoadSession {
        /// The serialized session snapshot.
        snapshot: String,
    },
}

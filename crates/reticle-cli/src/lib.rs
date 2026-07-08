//! The Reticle headless pipeline library.
//!
//! This crate hosts the testable logic behind the `reticle` binary: a batch
//! import / DRC / route / extract / export / render pipeline over hierarchical 2D
//! IC layouts. Each stage is a plain function that takes a parsed [`Document`] (or
//! a file path) and returns a [`Result`], so the CLI in `main.rs` stays a thin
//! [`clap`] dispatcher and every stage is exercised directly from integration
//! tests.
//!
//! # Format detection
//!
//! Layout files are read by extension: a `.gds` (or `.gdsii`) suffix is parsed as
//! GDSII via [`Gds`], and anything else is parsed as the in-house OASIS subset via
//! [`Oasis`] (see [`load_document`]). Both formats implement the `reticle-model`
//! [`Importer`] and [`Exporter`]
//! traits, so the pipeline treats them uniformly.
//!
//! # Errors
//!
//! Every stage funnels subsystem failures into a single [`CliError`], which maps
//! IO, parsing, model, and rendering problems into one type the binary can print
//! and turn into a process exit code.
//!
//! # The Tiny Tapeout precheck oracle
//!
//! The [`tt_precheck`] module parses Tiny Tapeout's own precheck output (its Magic DRC
//! and `KLayout` report files) into a structured [`tt_precheck::PrecheckReport`] whose
//! failures the agent loop consumes like DRC violations. The live precheck itself runs
//! Linux-native in a pinned Docker container via `just tt-precheck <gds>`
//! (`scripts/tt-precheck.ps1`); the parser here needs neither Docker nor the PDK and is
//! unit-tested against the precheck's committed output format.
//!
//! # The LEF/DEF import oracle
//!
//! The [`lefdef_oracle`] module applies the same pinned-container-oracle pattern to
//! `reticle-lefdef`: it cross-checks that crate's LEF/DEF import against `OpenROAD` running
//! in the pinned image, asserting the two agree on the structural facts (macro, component,
//! and pin counts, and the die area). Like the precheck, it skips honestly when Docker or
//! the image is unavailable, and a committed set of fixtures proves the cross-check both
//! ways (a faithful import matches; a corrupted DEF diverges) with no Docker in the gate.

#![forbid(unsafe_code)]

pub mod convert;
pub mod lefdef_oracle;
pub mod tt_precheck;

pub use convert::{ConvertSummary, run_convert};

use std::fmt;
use std::path::{Path, PathBuf};

use reticle_drc::DrcEngine;
use reticle_extract::Extractor;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::{Gds, Oasis, parse_technology};
use reticle_model::{
    Camera, Cell, Document, Exporter, Importer, NetSpec, RouteReport, RouteRequest, Router, Rule,
    RuleKind, RuleSet, Technology, Violation,
};
use reticle_render::{WgpuContext, WgpuRenderer};
use reticle_route::MazeRouter;

/// The layout container format of a file, selected by extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// GDSII binary (`.gds` / `.gdsii`).
    Gds,
    /// The in-house OASIS-inspired subset (any other extension).
    Oasis,
}

impl Format {
    /// Infers the format from a path's extension: `.gds`/`.gdsii` (case
    /// insensitive) is [`Format::Gds`], everything else is [`Format::Oasis`].
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("gds") || ext.eq_ignore_ascii_case("gdsii") => {
                Self::Gds
            }
            _ => Self::Oasis,
        }
    }

    /// Parses a user-supplied format name (`gds`/`gdsii` or `oasis`/`oas`).
    ///
    /// # Errors
    ///
    /// Returns [`CliError::UnknownFormat`] for any other value.
    pub fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "gds" | "gdsii" => Ok(Self::Gds),
            "oasis" | "oas" => Ok(Self::Oasis),
            other => Err(CliError::UnknownFormat(other.to_string())),
        }
    }

    /// A short lowercase label for the format.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Gds => "gds",
            Self::Oasis => "oasis",
        }
    }

    /// Decodes `bytes` into a [`Document`] using this format's importer.
    fn import(self, bytes: &[u8]) -> reticle_model::Result<Document> {
        match self {
            Self::Gds => Gds.import(bytes),
            Self::Oasis => Oasis.import(bytes),
        }
    }

    /// Encodes `doc` into bytes using this format's exporter.
    fn export(self, doc: &Document) -> reticle_model::Result<Vec<u8>> {
        match self {
            Self::Gds => Gds.export(doc),
            Self::Oasis => Oasis.export(doc),
        }
    }
}

/// The single error type for the pipeline: subsystem errors mapped into one place.
#[derive(Debug)]
pub enum CliError {
    /// An underlying filesystem error, tagged with the path it happened on.
    Io {
        /// The path the IO operation was attempted on.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A `reticle-model` error surfaced from import, export, or parsing.
    Model(reticle_model::ModelError),
    /// A PNG encoding error from the [`image`] crate.
    Image(image::ImageError),
    /// The document has no cell to operate on.
    NoTopCell,
    /// The requested cell does not exist in the document.
    CellNotFound(String),
    /// A `--format` value that is neither GDS nor OASIS.
    UnknownFormat(String),
    /// The `.rtla` archive builder failed while converting a GDSII file.
    Build(reticle_index::BuildError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "I/O error on {}: {source}", path.display())
            }
            Self::Model(e) => write!(f, "{e}"),
            Self::Image(e) => write!(f, "PNG encoding error: {e}"),
            Self::NoTopCell => write!(f, "document has no cells to operate on"),
            Self::CellNotFound(name) => write!(f, "cell not found: {name}"),
            Self::UnknownFormat(name) => {
                write!(f, "unknown format `{name}` (expected `gds` or `oasis`)")
            }
            Self::Build(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Model(e) => Some(e),
            Self::Image(e) => Some(e),
            Self::Build(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reticle_model::ModelError> for CliError {
    fn from(e: reticle_model::ModelError) -> Self {
        Self::Model(e)
    }
}

impl From<image::ImageError> for CliError {
    fn from(e: image::ImageError) -> Self {
        Self::Image(e)
    }
}

/// The pipeline result type.
pub type Result<T> = std::result::Result<T, CliError>;

/// Reads `path` and decodes it into a [`Document`], choosing the importer by the
/// file extension (see [`Format::from_path`]).
///
/// # Errors
///
/// Returns [`CliError::Io`] if the file cannot be read and [`CliError::Model`] if
/// the bytes are not valid for the detected format.
pub fn load_document(path: &Path) -> Result<Document> {
    let format = Format::from_path(path);
    let bytes = read_file(path)?;
    let doc = format.import(&bytes)?;
    Ok(doc)
}

/// Reads a file, mapping any IO error into a [`CliError::Io`] tagged with `path`.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes `bytes` to `path`, mapping any IO error into a [`CliError::Io`].
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Picks the cell to treat as the top of the design.
///
/// Prefers the document's first declared top cell; if none are declared (some
/// inputs leave the list empty) it falls back to any cell, so single-cell files
/// still work. Returns [`CliError::NoTopCell`] for an empty document.
///
/// # Errors
///
/// Returns [`CliError::NoTopCell`] when the document has no cells at all.
pub fn pick_top_cell(doc: &Document) -> Result<String> {
    if let Some(top) = doc.top_cells().first() {
        return Ok(top.clone());
    }
    doc.cells()
        .map(|c| c.name.clone())
        .min()
        .ok_or(CliError::NoTopCell)
}

/// A structured summary of a document's contents, produced by [`summarize`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentSummary {
    /// Total number of cells.
    pub cell_count: usize,
    /// The names of the document's top cells, sorted.
    pub top_cells: Vec<String>,
    /// Total number of flat shapes across every cell (own geometry only).
    pub shape_count: usize,
    /// Total number of single instances across every cell.
    pub instance_count: usize,
    /// Total number of array instances across every cell.
    pub array_count: usize,
    /// The distinct `(layer, datatype)` pairs used by any shape, sorted.
    pub layers: Vec<LayerId>,
}

/// Builds a [`DocumentSummary`] by walking every cell once.
#[must_use]
pub fn summarize(doc: &Document) -> DocumentSummary {
    let mut shape_count = 0usize;
    let mut instance_count = 0usize;
    let mut array_count = 0usize;
    let mut layers: Vec<LayerId> = Vec::new();

    for cell in doc.cells() {
        shape_count += cell.shapes.len();
        instance_count += cell.instances.len();
        array_count += cell.arrays.len();
        for shape in &cell.shapes {
            if !layers.contains(&shape.layer) {
                layers.push(shape.layer);
            }
        }
    }
    layers.sort_unstable();

    let mut top_cells: Vec<String> = doc.top_cells().to_vec();
    top_cells.sort();

    DocumentSummary {
        cell_count: doc.cell_count(),
        top_cells,
        shape_count,
        instance_count,
        array_count,
        layers,
    }
}

/// Resolves the DRC rule set to run against a document.
///
/// If `tech_file` is given, its rules are parsed with [`parse_technology`].
/// Otherwise the document's own technology rules are used, and if those are empty
/// a small [`default_rules`] set is synthesized so the command always checks
/// something.
///
/// # Errors
///
/// Returns [`CliError::Io`] if the technology file cannot be read and
/// [`CliError::Model`] if it cannot be parsed.
pub fn resolve_rules(doc: &Document, tech_file: Option<&Path>) -> Result<Vec<Rule>> {
    if let Some(path) = tech_file {
        let text = read_file(path)?;
        let source = String::from_utf8_lossy(&text);
        let tech: Technology = parse_technology(&source)?;
        return Ok(tech.rules);
    }
    let own = &doc.technology().rules;
    if own.is_empty() {
        Ok(default_rules(doc))
    } else {
        Ok(own.clone())
    }
}

/// A minimal fallback rule set used when neither a technology file nor the
/// document supplies rules.
///
/// It emits a width rule on every layer that carries geometry (and always on the
/// common metal layer `1/0`), so a document with any sub-micron feature will
/// report at least one violation. The threshold is deliberately large so tiny
/// synthetic shapes are flagged.
#[must_use]
pub fn default_rules(doc: &Document) -> Vec<Rule> {
    /// Minimum feature width, in DBU, for the synthesized default rule.
    const DEFAULT_MIN_WIDTH: i64 = 100;

    let mut layers: Vec<LayerId> = Vec::new();
    for cell in doc.cells() {
        for shape in &cell.shapes {
            if !layers.contains(&shape.layer) {
                layers.push(shape.layer);
            }
        }
    }
    if layers.is_empty() {
        layers.push(LayerId::new(1, 0));
    }
    layers.sort_unstable();

    layers
        .into_iter()
        .map(|layer| Rule {
            name: format!("default_min_width_{}_{}", layer.layer, layer.datatype),
            kind: RuleKind::Width,
            layer,
            other_layer: None,
            value: DEFAULT_MIN_WIDTH,
        })
        .collect()
}

/// Flattens the hierarchy under `top` into a single-cell [`Document`] for checking.
///
/// The returned document has exactly one cell, named `top`, whose shapes are the
/// fully expanded geometry of `top` in `doc`, every instance and array resolved with
/// its composed transform (see [`Document::flatten`]). The `drc`, `route`, and
/// `extract` stages run against this so a hierarchical design is checked as the flat
/// geometry it actually represents; a top cell that is a pure array of sub-cells (as
/// `xtask gen-layout` produces) would otherwise expose no own geometry to check. The
/// document's technology is carried over so tech-derived DRC rules still resolve.
#[must_use]
pub fn flatten_top_cell(doc: &Document, top: &str) -> Document {
    let mut cell = Cell::new(top);
    cell.shapes = doc.flatten(top);
    let mut flat = Document::new();
    flat.set_technology(doc.technology().clone());
    flat.insert_cell(cell);
    flat.set_top_cells(vec![top.to_owned()]);
    flat
}

/// Runs DRC over `cell` of `doc` with `rules`, returning every [`Violation`].
///
/// The check uses the cell's own geometry (hierarchy is not flattened); callers that
/// want the whole design checked pass a [`flatten_top_cell`] document. See the
/// `reticle-drc` crate docs for the semantics of each rule kind.
///
/// # Errors
///
/// Returns [`CliError::CellNotFound`] if `cell` is not in the document.
pub fn run_drc(doc: &Document, cell: &str, rules: Vec<Rule>) -> Result<Vec<Violation>> {
    if doc.cell(cell).is_none() {
        return Err(CliError::CellNotFound(cell.to_string()));
    }
    let engine = DrcEngine::new(rules);
    Ok(engine.check_cell(doc, cell))
}

/// Builds a [`RouteRequest`] for `cell` by synthesizing a couple of nets from the
/// cell's own geometry.
///
/// A shape's own footprint blocks the routing grid, so terminals are placed in the
/// open channel just *outside* each shape's bounding box (a fixed margin past its
/// right edge, at mid-height) rather than at its blocked center. Consecutive
/// exterior points are then connected on a spare routing layer, so a file with two
/// or more shapes yields a request the maze router can actually complete. A cell
/// with fewer than two shapes yields a request with no nets (the router then
/// reports zero routed).
#[must_use]
pub fn synth_route_request(doc: &Document, cell: &str) -> RouteRequest {
    /// The datatype offset used for the synthesized routing layer, kept clear of
    /// datatype 0 so routed wire does not collide with typical drawn geometry.
    const ROUTE_DATATYPE: u16 = 100;
    /// How far outside a shape's bounding box (in DBU) to place its terminal, so
    /// the terminal lands on open, unblocked tracks beside the obstacle.
    const TERMINAL_MARGIN: i32 = 40;

    let Some(cell_ref) = doc.cell(cell) else {
        return RouteRequest {
            cell: cell.to_string(),
            nets: Vec::new(),
        };
    };

    // One access point per shape, in the open space just past its right edge.
    let access: Vec<Point> = cell_ref
        .shapes
        .iter()
        .map(|s| {
            let bbox = bbox_of(s);
            let mid_y = center_of(&bbox).y;
            Point::new(bbox.max.x.saturating_add(TERMINAL_MARGIN), mid_y)
        })
        .collect();

    let mut nets = Vec::new();
    if access.len() >= 2 {
        let layer = LayerId::new(1, ROUTE_DATATYPE);
        // Two small nets: first-to-second and (if present) second-to-last.
        nets.push(NetSpec {
            name: "n0".to_string(),
            terminals: vec![access[0], access[1]],
            layer,
        });
        if access.len() >= 3 {
            nets.push(NetSpec {
                name: "n1".to_string(),
                terminals: vec![access[1], access[access.len() - 1]],
                layer,
            });
        }
    }

    RouteRequest {
        cell: cell.to_string(),
        nets,
    }
}

/// Routes `request` into `doc` with a default [`MazeRouter`], returning its report.
///
/// The router mutates `doc`, appending the routed wire geometry to the target
/// cell; the returned [`RouteReport`] summarizes routed/failed counts and total
/// wire length.
pub fn run_route(doc: &mut Document, request: &RouteRequest) -> RouteReport {
    let mut router = MazeRouter::new();
    router.route(doc, request)
}

/// Extracts connectivity for `cell` of `doc`, returning `(net_count, sizes)` where
/// `sizes` is each net's shape count in net order.
///
/// # Errors
///
/// Returns [`CliError::CellNotFound`] if `cell` is not in the document.
pub fn run_extract(doc: &Document, cell: &str) -> Result<(usize, Vec<usize>)> {
    if doc.cell(cell).is_none() {
        return Err(CliError::CellNotFound(cell.to_string()));
    }
    let netlist = Extractor::new().extract(doc, cell);
    let sizes = netlist.nets.iter().map(|n| n.shape_count).collect();
    Ok((netlist.nets.len(), sizes))
}

/// Converts `doc` to `format` and writes the encoded bytes to `out`.
///
/// # Errors
///
/// Returns [`CliError::Model`] if the document cannot be represented in `format`
/// (for example the OASIS subset rejects paths, instances, and arrays) and
/// [`CliError::Io`] if the output file cannot be written.
pub fn run_export(doc: &Document, out: &Path, format: Format) -> Result<()> {
    let bytes = format.export(doc)?;
    write_file(out, &bytes)?;
    Ok(())
}

/// A camera that frames `bbox` into a `width` x `height` pixel target with a small
/// margin, so the whole cell is visible and centered.
///
/// Falls back to a unit-scale camera at the origin for a degenerate (zero-area)
/// bounding box, which keeps the render path from producing a NaN projection.
#[must_use]
pub fn framing_camera(bbox: Rect, width: u32, height: u32) -> Camera {
    /// Fraction of the viewport left as empty margin around the design.
    const MARGIN: f32 = 0.05;

    let center = center_of(&bbox);
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let span_x = bbox.width().max(1) as f32;
    let span_y = bbox.height().max(1) as f32;

    // Pixels per DBU that fit the design on each axis, then take the tighter one
    // and back off by the margin so nothing is clipped at the edge.
    let fit_x = w / span_x;
    let fit_y = h / span_y;
    let ppd = (fit_x.min(fit_y) * (1.0 - MARGIN)).max(f32::MIN_POSITIVE);

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

    Camera {
        center,
        pixels_per_dbu: ppd,
        viewport,
    }
}

/// The outcome of [`run_render`]: whether a GPU was available and, if so, where
/// the PNG was written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RenderOutcome {
    /// A GPU was found; a PNG of the given `(width, height)` was written to `path`.
    Rendered {
        /// The file the PNG was saved to.
        path: PathBuf,
        /// The rendered image width in pixels.
        width: u32,
        /// The rendered image height in pixels.
        height: u32,
    },
    /// No compatible GPU adapter was available; nothing was rendered.
    NoGpu,
}

/// Renders `cell` of `doc` offscreen at `width` x `height` and saves it as a PNG at
/// `out`.
///
/// Acquires a headless GPU with [`WgpuContext::new_blocking`]; if no adapter is
/// available (for example CI without a software rasterizer) it returns
/// [`RenderOutcome::NoGpu`] without writing a file, so callers can treat a missing
/// GPU as a graceful skip rather than a failure.
///
/// # Errors
///
/// Returns [`CliError::CellNotFound`] if `cell` is missing and
/// [`CliError::Image`] / [`CliError::Io`] if the PNG cannot be encoded or written.
pub fn run_render(
    doc: &Document,
    cell: &str,
    out: &Path,
    width: u32,
    height: u32,
) -> Result<RenderOutcome> {
    if doc.cell(cell).is_none() {
        return Err(CliError::CellNotFound(cell.to_string()));
    }
    let Some(ctx) = WgpuContext::new_blocking() else {
        return Ok(RenderOutcome::NoGpu);
    };

    let bbox = doc
        .cell_bbox(cell)
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)));
    let camera = framing_camera(bbox, width, height);

    let mut renderer = WgpuRenderer::new();
    let rgba = renderer.render_document_offscreen(&ctx, doc, cell, &camera, (width, height));

    save_png(out, &rgba, width, height)?;
    Ok(RenderOutcome::Rendered {
        path: out.to_path_buf(),
        width,
        height,
    })
}

/// Encodes tightly packed RGBA8 `pixels` as a PNG and writes it to `path`.
///
/// # Errors
///
/// Returns [`CliError::Image`] if the buffer does not match `width * height * 4`
/// or the PNG cannot be encoded, and [`CliError::Io`] if the file cannot be
/// written.
fn save_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<()> {
    let buffer = image::RgbaImage::from_raw(width, height, pixels.to_vec()).ok_or_else(|| {
        CliError::Image(image::ImageError::Parameter(
            image::error::ParameterError::from_kind(
                image::error::ParameterErrorKind::DimensionMismatch,
            ),
        ))
    })?;
    buffer.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

/// The bounding box of a drawable shape.
fn bbox_of(shape: &reticle_model::DrawShape) -> Rect {
    use reticle_geometry::Shape;
    shape.bounding_box()
}

/// The integer center point of a rectangle.
fn center_of(rect: &Rect) -> Point {
    let cx = i64::midpoint(i64::from(rect.min.x), i64::from(rect.max.x));
    let cy = i64::midpoint(i64::from(rect.min.y), i64::from(rect.max.y));
    Point::new(cx as i32, cy as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Rect, Transform};
    use reticle_model::{ArrayInstance, DrawShape, ShapeKind};

    /// A pure-array top cell owns no geometry, so the un-flattened cell extracts to
    /// zero nets; [`flatten_top_cell`] expands the array so the same design extracts
    /// to the real per-element nets.
    #[test]
    fn flatten_top_cell_expands_array_hierarchy() {
        let layer = LayerId::new(1, 0);
        let mut leaf = Cell::new("leaf");
        leaf.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
        ));
        let mut top = Cell::new("top");
        top.arrays.push(ArrayInstance {
            cell: "leaf".to_owned(),
            transform: Transform::IDENTITY,
            columns: 2,
            rows: 1,
            column_pitch: 100,
            row_pitch: 100,
        });
        let mut doc = Document::new();
        doc.insert_cell(leaf);
        doc.insert_cell(top);
        doc.set_top_cells(vec!["top".to_owned()]);

        // The un-flattened top cell owns no shapes.
        assert_eq!(run_extract(&doc, "top").expect("extract top").0, 0);

        // Flattening resolves the 2x1 array into two disjoint placed rects.
        let flat = flatten_top_cell(&doc, "top");
        assert_eq!(flat.cell_count(), 1);
        let (nets, sizes) = run_extract(&flat, "top").expect("extract flat");
        assert_eq!(nets, 2, "two disjoint rects are two nets");
        assert_eq!(sizes, vec![1, 1]);
    }
}

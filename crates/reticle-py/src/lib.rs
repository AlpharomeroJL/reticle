//! Python bindings for the Reticle document and layout-generator APIs.
//!
//! This crate exposes a small, Pythonic surface over the same headless pipeline
//! the `reticle` CLI drives: open a layout from a GDSII or OASIS file, inspect its
//! cells and shapes, place a built-in generator into a cell by id with JSON
//! parameters, render a cell offscreen to PNG bytes, and save the document back
//! out. It is compiled as a stable-ABI (`abi3`) extension module so a single
//! wheel covers every CPython 3.x from 3.9 up.
//!
//! The module is imported as `reticle_py._core`; the pure-Python package in
//! `python/reticle_py` re-exports it and adds a Jupyter inline viewer widget.
//!
//! # Trust boundary
//!
//! Every count and size that crosses the Python boundary is validated before it
//! reaches the native code. Pixel dimensions are bounded to a sane maximum so a
//! caller cannot request a multi-gigabyte render allocation, cell names are
//! checked against the document before use, and parameter blobs are parsed and
//! validated by the generator registry (which reports a precise field error).

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use pyo3::exceptions::{PyIOError, PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use reticle_cli::{Format, RenderOutcome, framing_camera, load_document, run_export, summarize};
use reticle_gen::Registry;
use reticle_geometry::{Point, Rect, Shape};
use reticle_model::{Document as CoreDocument, ShapeKind};
use reticle_render::{WgpuContext, WgpuRenderer};

/// The largest pixel dimension a single render request may ask for on either
/// axis. A 16384 x 16384 RGBA frame is already a gigabyte of pixels; anything
/// larger is treated as a mistake rather than honored, so an untrusted caller
/// cannot drive an unbounded allocation.
const MAX_RENDER_DIM: u32 = 16_384;

/// Validates a pixel dimension supplied from Python: it must be at least one and
/// no larger than [`MAX_RENDER_DIM`].
fn check_dim(name: &str, value: u32) -> PyResult<u32> {
    if value == 0 {
        return Err(PyValueError::new_err(format!("{name} must be at least 1")));
    }
    if value > MAX_RENDER_DIM {
        return Err(PyValueError::new_err(format!(
            "{name} = {value} exceeds the maximum of {MAX_RENDER_DIM}"
        )));
    }
    Ok(value)
}

/// Encodes a tightly packed RGBA8 buffer as PNG bytes in memory.
fn encode_png(rgba: Vec<u8>, width: u32, height: u32) -> PyResult<Vec<u8>> {
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| PyRuntimeError::new_err("rendered pixel buffer had an unexpected size"))?;
    let mut cursor = Cursor::new(Vec::new());
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| PyRuntimeError::new_err(format!("PNG encoding failed: {e}")))?;
    Ok(cursor.into_inner())
}

/// A `[x0, y0, x1, y1]` list for a rectangle, or `None` for no box.
fn bbox_list(py: Python<'_>, bbox: Option<Rect>) -> Option<Py<PyAny>> {
    bbox.map(|r| {
        [r.min.x, r.min.y, r.max.x, r.max.y]
            .into_pyobject(py)
            .expect("i32 array converts to a Python list")
            .into_any()
            .unbind()
    })
}

/// A hierarchical layout document: a set of named cells plus technology data.
///
/// Open one with [`Document::open`], inspect it, place generators into its cells,
/// render a cell to PNG, and write it back with [`Document::save`].
#[pyclass]
struct Document {
    doc: CoreDocument,
}

#[pymethods]
impl Document {
    /// Opens a layout file and decodes it into a document.
    ///
    /// The container format is chosen by the file extension: `.gds` / `.gdsii`
    /// is read as GDSII, anything else as the in-house OASIS subset.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        let doc = load_document(Path::new(path)).map_err(cli_err)?;
        Ok(Self { doc })
    }

    /// Writes the document to `path`. The format is taken from `format`
    /// (`"gds"` or `"oasis"`) when given, otherwise inferred from the extension.
    #[pyo3(signature = (path, format=None))]
    fn save(&self, path: &str, format: Option<&str>) -> PyResult<()> {
        let fmt = match format {
            Some(name) => Format::parse(name).map_err(cli_err)?,
            None => Format::from_path(Path::new(path)),
        };
        run_export(&self.doc, Path::new(path), fmt).map_err(cli_err)
    }

    /// The names of every cell in the document, sorted.
    fn cell_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.doc.cells().map(|c| c.name.clone()).collect();
        names.sort();
        names
    }

    /// The names of the document's top (root) cells.
    fn top_cells(&self) -> Vec<String> {
        self.doc.top_cells().to_vec()
    }

    /// The number of cells in the document.
    fn cell_count(&self) -> usize {
        self.doc.cell_count()
    }

    /// A structured summary of the document: cell, shape, instance and array
    /// counts, the top-cell names, and the distinct layers in use.
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = summarize(&self.doc);
        let dict = PyDict::new(py);
        dict.set_item("cell_count", s.cell_count)?;
        dict.set_item("top_cells", s.top_cells)?;
        dict.set_item("shape_count", s.shape_count)?;
        dict.set_item("instance_count", s.instance_count)?;
        dict.set_item("array_count", s.array_count)?;
        let layers: Vec<(u16, u16)> = s.layers.iter().map(|l| (l.layer, l.datatype)).collect();
        dict.set_item("layers", layers)?;
        Ok(dict)
    }

    /// The own (non-instanced) shapes of a cell, each as a dict with its layer,
    /// datatype, kind (`"rect"`, `"polygon"` or `"path"`) and bounding box.
    ///
    /// Raises `KeyError` if the cell does not exist.
    fn shapes<'py>(&self, py: Python<'py>, cell: &str) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let cell_ref = self
            .doc
            .cell(cell)
            .ok_or_else(|| PyKeyError::new_err(format!("cell not found: {cell}")))?;
        let mut out = Vec::with_capacity(cell_ref.shapes.len());
        for shape in &cell_ref.shapes {
            let dict = PyDict::new(py);
            dict.set_item("layer", shape.layer.layer)?;
            dict.set_item("datatype", shape.layer.datatype)?;
            let kind = match &shape.kind {
                ShapeKind::Rect(_) => "rect",
                ShapeKind::Polygon(_) => "polygon",
                ShapeKind::Path(_) => "path",
            };
            dict.set_item("kind", kind)?;
            let b = shape.bounding_box();
            dict.set_item("bbox", [b.min.x, b.min.y, b.max.x, b.max.y])?;
            out.push(dict);
        }
        Ok(out)
    }

    /// Places a built-in generator into `cell` by id, driven by `params_json` (a
    /// JSON object of the generator's parameters). The generated geometry is
    /// appended to the cell in place.
    ///
    /// Returns a dict with `shapes_added` and an optional `bbox` of the emitted
    /// geometry. Raises `KeyError` for an unknown cell, and `ValueError` for an
    /// unknown generator id or invalid parameters (with a precise field message).
    fn place_generator<'py>(
        &mut self,
        py: Python<'py>,
        cell: &str,
        generator_id: &str,
        params_json: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        if self.doc.cell(cell).is_none() {
            return Err(PyKeyError::new_err(format!("cell not found: {cell}")));
        }
        let params: serde_json::Value = serde_json::from_str(params_json)
            .map_err(|e| PyValueError::new_err(format!("params_json is not valid JSON: {e}")))?;

        // Clone the technology so the mutable cell borrow below does not collide
        // with the shared borrow the generator needs for the tech.
        let tech = self.doc.technology().clone();
        let registry = Registry::with_builtins();
        let target = self
            .doc
            .cell_mut(cell)
            .expect("cell presence was checked above");

        let output = registry
            .generate(generator_id, &params, &tech, target)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;

        let dict = PyDict::new(py);
        dict.set_item("shapes_added", output.shapes_added)?;
        dict.set_item("bbox", bbox_list(py, output.bbox))?;
        Ok(dict)
    }

    /// Renders `cell` offscreen at `width` x `height` and returns the PNG bytes,
    /// or `None` when no GPU adapter is available (a headless machine without a
    /// software rasterizer).
    ///
    /// Raises `KeyError` if the cell does not exist and `ValueError` if a
    /// dimension is zero or larger than the render limit.
    fn render_png(
        &self,
        py: Python<'_>,
        cell: &str,
        width: u32,
        height: u32,
    ) -> PyResult<Option<Py<PyBytes>>> {
        let width = check_dim("width", width)?;
        let height = check_dim("height", height)?;
        if self.doc.cell(cell).is_none() {
            return Err(PyKeyError::new_err(format!("cell not found: {cell}")));
        }

        // Reuse the CLI render path: acquire a headless GPU, frame the cell, and
        // rasterize offscreen. A missing adapter is a graceful `None`, matching
        // the CLI's `RenderOutcome::NoGpu`.
        let Some(ctx) = WgpuContext::new_blocking() else {
            return Ok(None);
        };
        let bbox = self
            .doc
            .cell_bbox(cell)
            .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)));
        let camera = framing_camera(bbox, width, height);
        let mut renderer = WgpuRenderer::new();
        let rgba =
            renderer.render_document_offscreen(&ctx, &self.doc, cell, &camera, (width, height));
        let png = encode_png(rgba, width, height)?;
        Ok(Some(PyBytes::new(py, &png).unbind()))
    }

    fn __repr__(&self) -> String {
        format!(
            "Document(cells={}, top_cells={:?})",
            self.doc.cell_count(),
            self.doc.top_cells()
        )
    }
}

/// The `RenderOutcome` marker is re-exported only so the dependency on
/// `reticle-cli`'s render types stays explicit; the Python surface reports a
/// missing GPU as `None` rather than exposing the enum.
#[allow(dead_code)]
fn _render_outcome_is_used(o: RenderOutcome) -> bool {
    matches!(o, RenderOutcome::NoGpu)
}

/// The built-in generators, each as a dict of `id`, `title` and `description`.
/// Pass an id and a JSON parameter object to [`Document::place_generator`].
#[pyfunction]
fn generators() -> Vec<HashMap<String, String>> {
    Registry::with_builtins()
        .infos()
        .into_iter()
        .map(|info| {
            let mut m = HashMap::new();
            m.insert("id".to_string(), info.id.to_string());
            m.insert("title".to_string(), info.title.to_string());
            m.insert("description".to_string(), info.description.to_string());
            m
        })
        .collect()
}

/// The crate version string.
#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Maps a `reticle-cli` pipeline error into the closest Python exception: I/O
/// failures become `IOError`, everything else a `ValueError`.
fn cli_err(err: reticle_cli::CliError) -> PyErr {
    match err {
        reticle_cli::CliError::Io { .. } => PyIOError::new_err(err.to_string()),
        other => PyValueError::new_err(other.to_string()),
    }
}

/// The `reticle_py._core` extension module.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "__doc__",
        "Native Reticle bindings: documents, generators, rendering.",
    )?;
    m.add_class::<Document>()?;
    m.add_function(wrap_pyfunction!(generators, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_zero_is_rejected() {
        assert!(check_dim("width", 0).is_err());
    }

    #[test]
    fn dim_over_limit_is_rejected() {
        assert!(check_dim("height", MAX_RENDER_DIM + 1).is_err());
    }

    #[test]
    fn dim_in_range_passes_through() {
        assert_eq!(check_dim("width", 800).expect("in range"), 800);
        assert_eq!(
            check_dim("height", MAX_RENDER_DIM).expect("at limit"),
            MAX_RENDER_DIM
        );
    }

    #[test]
    fn builtin_generators_are_listed() {
        let ids: Vec<String> = generators().into_iter().map(|m| m["id"].clone()).collect();
        assert!(ids.contains(&"guard_ring".to_string()), "ids: {ids:?}");
        assert!(!ids.is_empty());
    }
}

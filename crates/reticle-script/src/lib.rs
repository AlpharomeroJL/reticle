//! Embedded scripting for Reticle.
//!
//! Wave 3 exposes the model to an embedded [`rhai`] engine, create, query,
//! transform, run DRC, and export, plus a plugin folder convention and worked
//! example scripts.
//!
//! # Quick start
//!
//! ```
//! use reticle_script::ScriptEngine;
//!
//! let mut engine = ScriptEngine::new();
//! engine
//!     .eval(
//!         r#"
//!         create_cell("TOP");
//!         add_rect("TOP", 1, 0, 0, 0, 100, 200);
//!         "#,
//!     )
//!     .expect("script runs");
//!
//! let doc = engine.document();
//! assert_eq!(doc.cell_count(), 1);
//! assert_eq!(doc.cell("TOP").unwrap().shapes.len(), 1);
//! ```
//!
//! # The scripting API
//!
//! [`ScriptEngine::eval`] runs a script against a host-owned document; the changes
//! are reflected into the document [`ScriptEngine::document`] returns. The full set
//! of functions a script may call is registered by the internal `api` module. In
//! brief:
//!
//! - **Create / edit**: `create_cell`, `add_rect`, `add_polygon`, `add_path`,
//!   `add_path_capped`, `add_instance`, `add_array`, `set_top_cells`.
//! - **Query**: `cell_count`, `has_cell`, `shape_count`, `instance_count`,
//!   `array_count`, `cell_bbox`, `shapes_bbox`, `flatten_count`.
//! - **Transform**: `flatten_into`.
//! - **DRC**: `load_technology`, `add_width_rule`, `add_spacing_rule`,
//!   `add_area_rule`, `add_notch_rule`, `add_enclosure_rule`, `rule_count`,
//!   `run_drc`, `drc_messages`.
//! - **Export**: `export_gds`, `export_oasis` (each returns a `rhai` blob).
//!
//! # Plugins
//!
//! [`ScriptEngine::run_plugin_dir`] evaluates every `.rhai` file in a directory in
//! sorted (lexicographic) order against the same document, so a folder of scripts
//! composes into one design. Worked examples live in `examples/` next to this
//! crate (`param_cell.rhai`, `drc_sweep.rhai`).
//!
//! # Errors
//!
//! Fallible methods return [`ScriptError`] (aliased [`Result`]), which distinguishes
//! a `rhai` evaluation failure, a rejected model edit, and a plugin-directory I/O
//! error, each with a message. This is a deliberate, contract-sanctioned refinement
//! of the Wave 0 placeholder signature (which returned `reticle_model::Result<()>`):
//! a script failure carries `rhai`'s diagnostic, including source position, which
//! the model's error enum cannot represent, so the richer [`ScriptError`] is
//! returned instead.

#![forbid(unsafe_code)]

mod api;
mod error;
mod host;
// --- phase2 scaffold: sandboxed PCell producer (ADR 0107) ---
mod pcell;
// --- end phase2 scaffold ---

use std::path::Path;

use rhai::Engine;

pub use error::{Result, ScriptError};
// --- phase2 scaffold: sandboxed PCell producer (ADR 0107) ---
pub use pcell::{ProduceError, SandboxLimits, produce};
// --- end phase2 scaffold ---

use host::{SharedHost, shared_host};

/// An embedded script engine bound to a document (Wave 3: [`rhai`]).
///
/// The engine owns a `rhai` [`Engine`] with the Reticle API registered on it and a
/// shared handle to the host state the API mutates. [`ScriptEngine::eval`] runs a
/// script; [`ScriptEngine::document`] returns the resulting document.
#[derive(Debug)]
pub struct ScriptEngine {
    /// The configured `rhai` engine (API functions already registered).
    engine: Engine,
    /// Shared, interior-mutable host state driven by the registered functions.
    host: SharedHost,
    /// The most recent document snapshot, refreshed after each evaluation so
    /// [`ScriptEngine::document`] can return a borrow.
    document: reticle_model::Document,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    /// Creates a script engine over a fresh document, with the Reticle scripting
    /// API registered.
    #[must_use]
    pub fn new() -> Self {
        let host = shared_host();
        let mut engine = Engine::new();
        api::register_api(&mut engine, &host);
        Self {
            engine,
            host,
            document: reticle_model::Document::new(),
        }
    }

    /// Evaluates a script against the bound document, then refreshes the snapshot
    /// returned by [`ScriptEngine::document`].
    ///
    /// Edits performed by the script (adding cells, shapes, instances, arrays, and
    /// setting top cells or technology) accumulate in the host document, so calling
    /// `eval` repeatedly composes scripts against the same design.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::Eval`] if the script fails to compile or traps at
    /// runtime (a bad argument, an unknown cell, an unsupported export, …). The
    /// message includes `rhai`'s diagnostic and source position.
    pub fn eval(&mut self, source: &str) -> Result<()> {
        let result = self.engine.run(source);
        // Refresh the snapshot even on failure so partial edits made before the
        // trap are still observable through `document()`.
        self.refresh();
        result?;
        Ok(())
    }

    /// Evaluates every `.rhai` file in `dir` in sorted (lexicographic) filename
    /// order, against the same document.
    ///
    /// Non-`.rhai` entries and sub-directories are ignored. This is the plugin
    /// convention: dropping ordered scripts into a folder composes them into one
    /// design (e.g. `10_cells.rhai`, `20_route.rhai`, `30_drc.rhai`).
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::Io`] if the directory or one of its files cannot be
    /// read, or [`ScriptError::Eval`] if any script fails (evaluation stops at the
    /// first failing file). The returned count on success is the number of scripts
    /// that ran.
    pub fn run_plugin_dir(&mut self, dir: impl AsRef<Path>) -> Result<usize> {
        let dir = dir.as_ref();
        let mut scripts: Vec<std::path::PathBuf> = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| ScriptError::Io {
            path: dir.to_path_buf(),
            source: e.to_string(),
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| ScriptError::Io {
                path: dir.to_path_buf(),
                source: e.to_string(),
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rhai") {
                scripts.push(path);
            }
        }
        scripts.sort();

        for path in &scripts {
            let source = std::fs::read_to_string(path).map_err(|e| ScriptError::Io {
                path: path.clone(),
                source: e.to_string(),
            })?;
            self.eval(&source)?;
        }
        Ok(scripts.len())
    }

    /// The bound document, reflecting every script evaluated so far.
    #[must_use]
    pub fn document(&self) -> &reticle_model::Document {
        &self.document
    }

    /// Refreshes the cached document snapshot from the host state.
    fn refresh(&mut self) {
        self.document = self.host.borrow().snapshot();
    }
}

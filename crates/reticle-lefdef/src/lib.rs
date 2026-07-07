//! LEF/DEF import into the Reticle document model.
//!
//! This crate parses the two text formats an `OpenROAD`/`OpenLane` flow emits, LEF
//! (the technology and the macro cell abstracts) and DEF (the placed, routed
//! design), and lowers them into a [`reticle_model::Document`] plus the run-level
//! metadata a viewer overlays. The single public result is [`LefDefDesign`], the
//! contract the run viewer (lane 5B) consumes.
//!
//! # Entry points
//!
//! - [`import_lef_def`] takes a LEF byte slice and a DEF byte slice and returns a
//!   [`LefDefDesign`].
//! - [`import_run_dir`] takes a flow output directory, finds its LEF and DEF files,
//!   and imports them. Report parsing (congestion, utilization, timing) is a lane
//!   5B concern; the [`ReportOverlays`] slots are present but left empty here.
//!
//! # Supported subset
//!
//! This is a deliberate subset, chosen to render an `OpenROAD` run, not a full
//! LEF/DEF implementation. See the interop chapter in the book and ADR 0063 for the
//! precise scope. In short:
//!
//! - **LEF**: `UNITS DATABASE MICRONS`; `LAYER` (name, `TYPE`, `WIDTH`); `SITE`
//!   (`CLASS`, `SIZE`); `MACRO` (`CLASS`, `SIZE`, `PIN` with `PORT`/`LAYER`/`RECT`,
//!   `OBS`). Via geometry, spacing tables, and antenna rules are skipped.
//! - **DEF**: `DESIGN`; `UNITS DISTANCE MICRONS`; `DIEAREA`; `ROW`; `COMPONENTS`
//!   (`PLACED`/`FIXED` location and orientation); `PINS` (`NET`, `DIRECTION`,
//!   `LAYER` rectangle, `PLACED`); `NETS` (`ROUTED` wires and vias). `SPECIALNETS`,
//!   `GROUPS`, `REGIONS`, and `BLOCKAGES` are skipped with a warning.
//!
//! Only `RECT` geometry is lowered from LEF ports and obstructions; a `POLYGON` in
//! a port is skipped with a warning. Rectilinear die areas are reduced to their
//! bounding box.
//!
//! # Robustness
//!
//! LEF and DEF are untrusted input. Neither [`import_lef_def`] nor [`import_run_dir`]
//! panics or hangs on any byte sequence: inputs over [`MAX_INPUT_BYTES`] are refused
//! before parsing (so a hostile length cannot force a large allocation, the OASIS
//! out-of-memory lesson), bytes are decoded lossily so invalid UTF-8 never panics,
//! the tokenizer and parsers advance by at least one token per step over a finite
//! stream (so no parse loops forever), and no collection is ever pre-sized from a
//! count read out of the input. A statement that cannot be parsed is a clean
//! [`LefDefError`]; a recoverable problem is a [`LefDefWarning`] on
//! [`LefDefDesign::warnings`] and the rest of the design still imports.

mod def;
mod design;
mod error;
mod lef;
mod lex;
mod lower;
mod orient;

pub use design::{
    CongestionCell, CriticalNet, DesignPin, LefDefDesign, Net, NetSegment, ReportOverlays, Row,
    Site,
};
pub use error::{LefDefError, LefDefWarning, WarningKind};

use std::path::Path;

/// The largest LEF or DEF input this importer will attempt to parse, in bytes
/// (256 MiB). A stream at or under this bound tokenizes within a bounded
/// allocation; a larger one is refused with a [`LefDefError::TooLarge`] rather than
/// risking an out-of-memory abort on a hostile or truncated-huge input. Real
/// `OpenROAD` run artifacts are a few megabytes, far under this ceiling.
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

/// Imports a LEF/DEF pair into a [`LefDefDesign`].
///
/// `lef` is the technology-and-macros LEF (or several concatenated LEF files, which
/// is valid: each carries its own blocks). `def` is the placed, routed design.
///
/// # Errors
///
/// Returns [`LefDefError::TooLarge`] if either input exceeds [`MAX_INPUT_BYTES`], or
/// [`LefDefError::Lef`] / [`LefDefError::Def`] if a statement is structurally
/// malformed. Recoverable problems are collected on [`LefDefDesign::warnings`]
/// instead of failing the import.
pub fn import_lef_def(lef: &[u8], def: &[u8]) -> Result<LefDefDesign, LefDefError> {
    if lef.len() > MAX_INPUT_BYTES {
        return Err(LefDefError::TooLarge {
            which: "LEF",
            bytes: lef.len(),
            limit: MAX_INPUT_BYTES,
        });
    }
    if def.len() > MAX_INPUT_BYTES {
        return Err(LefDefError::TooLarge {
            which: "DEF",
            bytes: def.len(),
            limit: MAX_INPUT_BYTES,
        });
    }

    // Decode lossily so invalid UTF-8 in a hostile file never panics; the parsers
    // only key on ASCII keywords and punctuation, so a replacement character in a
    // name is harmless.
    let lef_text = String::from_utf8_lossy(lef);
    let def_text = String::from_utf8_lossy(def);

    let lef_data = lef::parse(&lef_text)?;
    let def_data = def::parse(&def_text)?;

    Ok(lower::lower(lef_data, def_data))
}

/// Imports a flow output directory: finds its LEF and DEF files and imports them.
///
/// The directory is walked (bounded in depth and file count) for `*.lef` and
/// `*.def` files. All LEF files found are concatenated (LEF is additive); of the
/// DEF files, the one whose name sorts last is used, which selects the later flow
/// stage (for example `6_final.def` over `2_floorplan.def`). Report files are not
/// read here: the [`ReportOverlays`] on the result stay empty for lane 5B to fill.
///
/// # Errors
///
/// Returns [`LefDefError::MissingFile`] if the directory has no `.lef` or no `.def`,
/// [`LefDefError::Io`] on a filesystem error, or the same parse/size errors as
/// [`import_lef_def`].
pub fn import_run_dir(dir: &Path) -> Result<LefDefDesign, LefDefError> {
    let mut lef_paths = Vec::new();
    let mut def_paths = Vec::new();
    collect_lef_def(dir, 0, &mut lef_paths, &mut def_paths)?;

    if lef_paths.is_empty() {
        return Err(LefDefError::MissingFile("LEF (.lef)".to_string()));
    }
    if def_paths.is_empty() {
        return Err(LefDefError::MissingFile("DEF (.def)".to_string()));
    }

    // Concatenate all LEF files; each has its own blocks and END LIBRARY.
    lef_paths.sort();
    let mut lef_bytes = Vec::new();
    for p in &lef_paths {
        let mut bytes = std::fs::read(p).map_err(|e| LefDefError::Io(e.to_string()))?;
        lef_bytes.append(&mut bytes);
        lef_bytes.push(b'\n');
        if lef_bytes.len() > MAX_INPUT_BYTES {
            return Err(LefDefError::TooLarge {
                which: "LEF",
                bytes: lef_bytes.len(),
                limit: MAX_INPUT_BYTES,
            });
        }
    }

    // Pick the DEF whose name sorts last (the later flow stage).
    def_paths.sort();
    let def_path = def_paths.last().expect("def_paths is non-empty");
    let def_bytes = std::fs::read(def_path).map_err(|e| LefDefError::Io(e.to_string()))?;

    import_lef_def(&lef_bytes, &def_bytes)
}

/// Depth and fan-out caps for the run-directory walk, so a pathological tree cannot
/// make [`import_run_dir`] wander without bound.
const MAX_WALK_DEPTH: usize = 8;
const MAX_WALK_ENTRIES: usize = 100_000;

/// Recursively collects `*.lef` and `*.def` paths under `dir`, bounded in depth and
/// total entries. A read error on a subdirectory is ignored so one unreadable
/// corner does not fail the whole import.
fn collect_lef_def(
    dir: &Path,
    depth: usize,
    lef_paths: &mut Vec<std::path::PathBuf>,
    def_paths: &mut Vec<std::path::PathBuf>,
) -> Result<(), LefDefError> {
    if depth > MAX_WALK_DEPTH || lef_paths.len() + def_paths.len() > MAX_WALK_ENTRIES {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if depth == 0 => return Err(LefDefError::Io(e.to_string())),
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lef_def(&path, depth + 1, lef_paths, def_paths)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext.to_ascii_lowercase().as_str() {
                "lef" => lef_paths.push(path),
                "def" => def_paths.push(path),
                _ => {}
            }
        }
    }
    Ok(())
}

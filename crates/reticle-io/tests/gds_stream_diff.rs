//! Differential test: the forward-only [`GdsRecordReader`] must agree with the DOM
//! importer ([`reticle_io::Gds`]) on every real corpus file.
//!
//! The invariant is one-directional and load-bearing for Wave 2: **whatever the DOM
//! importer accepts, the streaming reader must also accept and describe identically**
//! (same library scale, same cell names, same kept-shape counts per layer, same
//! label/instance/array counts). The streaming reader may be *more* tolerant on
//! inputs the DOM importer rejects; it must never be less tolerant on inputs it
//! accepts. For files the DOM importer rejects we only require the streaming reader
//! not to panic or hang (it is driven to exhaustion).
//!
//! The streaming reader emits raw geometry (a boundary is surfaced whatever its
//! vertex count), so to compare *kept-shape* counts this test replays the exact
//! degeneracy/oversize policy the DOM importer applies in `boundary_to_shape` /
//! `path_to_shape`. That keeps the reader a low-level record decoder while still
//! proving parity with the policy layer above it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reticle_io::gds::MAX_SHAPE_VERTICES;
use reticle_io::{Gds, GdsEvent, GdsRecordReader};
use reticle_model::Importer;

/// Per-cell tallies compared between the two readers.
#[derive(Default, PartialEq, Eq, Debug)]
struct CellCounts {
    /// Kept shapes (boundaries + paths that survive the degeneracy filter), keyed
    /// by `(layer, datatype)`.
    shapes: BTreeMap<(u16, u16), usize>,
    labels: usize,
    instances: usize,
    arrays: usize,
}

/// The workspace root, two levels up from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `.gds` file under the three real corpora named in the lane packet.
fn corpus_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let dirs = [
        root.join("corpus").join("tinytapeout"),
        root.join("examples").join("tapeout"),
        root.join("crates").join("reticle-app").join("assets"),
    ];
    let mut files = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gds") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Does a boundary with these vertices survive the DOM importer's filter (and if so
/// it counts as one kept shape)? Mirrors `gds::boundary_to_shape` exactly.
fn boundary_kept(xy: &[(i32, i32)]) -> bool {
    if xy.len() > MAX_SHAPE_VERTICES {
        return false;
    }
    let mut pts = xy.to_vec();
    if pts.len() >= 2 && pts.first() == pts.last() {
        pts.pop();
    }
    pts.len() >= 3
}

/// Does a path with these vertices survive the DOM importer's filter? Mirrors
/// `gds::path_to_shape` exactly.
fn path_kept(xy: &[(i32, i32)]) -> bool {
    xy.len() <= MAX_SHAPE_VERTICES && xy.len() >= 2
}

/// Drives the streaming reader to exhaustion and folds its events into per-cell
/// counts plus the recovered `dbu_per_micron`. Returns `Err` if the reader errors.
fn stream_counts(bytes: &[u8]) -> Result<(BTreeMap<String, CellCounts>, Option<i64>), String> {
    let mut reader = GdsRecordReader::new(bytes);
    let mut cells: BTreeMap<String, CellCounts> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut dbu = None;

    while let Some(event) = reader.next_event().map_err(|e| e.to_string())? {
        match event {
            GdsEvent::BeginLibrary { dbu_per_micron } => dbu = Some(dbu_per_micron),
            GdsEvent::BeginStruct { name } => {
                // Last struct of a given name wins, matching `Document::insert_cell`.
                cells.insert(name.clone(), CellCounts::default());
                current = Some(name);
            }
            GdsEvent::EndStruct => current = None,
            GdsEvent::Boundary {
                layer,
                datatype,
                xy,
            } => {
                if boundary_kept(&xy)
                    && let Some(c) = current.as_ref().and_then(|n| cells.get_mut(n))
                {
                    *c.shapes.entry((layer, datatype)).or_default() += 1;
                }
            }
            GdsEvent::Path {
                layer,
                datatype,
                xy,
                ..
            } => {
                if path_kept(&xy)
                    && let Some(c) = current.as_ref().and_then(|n| cells.get_mut(n))
                {
                    *c.shapes.entry((layer, datatype)).or_default() += 1;
                }
            }
            GdsEvent::Text { .. } => {
                if let Some(c) = current.as_ref().and_then(|n| cells.get_mut(n)) {
                    c.labels += 1;
                }
            }
            GdsEvent::StructRef { .. } => {
                if let Some(c) = current.as_ref().and_then(|n| cells.get_mut(n)) {
                    c.instances += 1;
                }
            }
            GdsEvent::ArrayRef { .. } => {
                if let Some(c) = current.as_ref().and_then(|n| cells.get_mut(n)) {
                    c.arrays += 1;
                }
            }
            GdsEvent::EndLibrary => {}
        }
    }
    Ok((cells, dbu))
}

/// The DOM importer's document, folded into the same per-cell counts.
fn dom_counts(doc: &reticle_model::Document) -> BTreeMap<String, CellCounts> {
    let mut cells = BTreeMap::new();
    for cell in doc.cells() {
        let mut counts = CellCounts {
            labels: cell.labels.len(),
            instances: cell.instances.len(),
            arrays: cell.arrays.len(),
            ..CellCounts::default()
        };
        for shape in &cell.shapes {
            *counts
                .shapes
                .entry((shape.layer.layer, shape.layer.datatype))
                .or_default() += 1;
        }
        cells.insert(cell.name.clone(), counts);
    }
    cells
}

#[test]
fn streaming_reader_agrees_with_dom_importer_on_corpus() {
    let files = corpus_files();
    assert!(
        files.len() >= 3,
        "expected the real corpora to contain GDS files; found {}",
        files.len()
    );

    let mut compared = 0usize;
    for path in files {
        let bytes = std::fs::read(&path).expect("read corpus file");
        let name = path.display();

        if let Ok(doc) = Gds.import(&bytes) {
            // The DOM importer accepted it, so the streaming reader must too.
            let (stream_cells, stream_dbu) = stream_counts(&bytes).unwrap_or_else(|e| {
                panic!("{name}: streaming reader errored on a file the DOM importer accepted: {e}")
            });

            let dom_cells = dom_counts(&doc);
            assert_eq!(
                stream_cells.keys().collect::<Vec<_>>(),
                dom_cells.keys().collect::<Vec<_>>(),
                "{name}: cell-name sets differ"
            );
            assert_eq!(stream_cells, dom_cells, "{name}: per-cell counts differ");

            if let Some(dbu) = stream_dbu {
                assert_eq!(
                    dbu,
                    doc.technology().dbu_per_micron,
                    "{name}: dbu_per_micron differs"
                );
            }
            compared += 1;
        } else {
            // The DOM importer rejected it; only require the streaming reader not to
            // panic or hang. Its result (events or an error) is unconstrained.
            let mut reader = GdsRecordReader::new(&bytes[..]);
            while let Ok(Some(_)) = reader.next_event() {}
        }
    }
    assert!(
        compared >= 3,
        "expected to compare at least the three real tapeout files; compared {compared}"
    );
}

/// The committed GDS crash fixtures encode the two panic classes the v8 Wave 0
/// campaign found in the DOM importer (out-of-range dates, zero-length string
/// records). libFuzzer cannot link on MSVC, so this native test stands in for the
/// fuzzer on the streaming path: driving the reader over each fixture to exhaustion
/// must never panic or hang (it returns events or a clean error). It is the
/// same-platform guard that the fixtures cannot reintroduce a fixed panic class here.
#[test]
fn streaming_reader_survives_committed_crash_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fuzz-regressions")
        .join("gds");
    let mut seen = 0usize;
    for entry in std::fs::read_dir(&dir)
        .expect("read fixtures dir")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "gds") {
            let bytes = std::fs::read(&path).expect("read fixture");
            let mut reader = GdsRecordReader::new(&bytes[..]);
            // Terminates and never panics; the result itself is unconstrained.
            while let Ok(Some(_)) = reader.next_event() {}
            seen += 1;
        }
    }
    assert!(seen >= 1, "expected committed crash fixtures under {dir:?}");
}

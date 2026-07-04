//! Regenerates the committed open-anything import corpus under `corpus/tinytapeout/`.
//!
//! The corpus proves the hardened GDSII importer never panics or hangs and always
//! returns a clean document or a clean error. It has two halves:
//!
//! * **Synthesized malformed GDSII**, hand-built here from raw record bytes so
//!   every hazard class is exercised deterministically and offline: an empty file,
//!   non-GDS bytes, a truncated stream, a bad record length, an absurd element
//!   count, a cyclic structure reference, and an oversized structure reference.
//!   None of these needs the network, so hardening is provable with no fetch.
//!
//! * **A minimized real sample** derived from a fetched Tiny Tapeout design. The
//!   full designs (0.5 to 2 MB, whole shuttle tiles) are not committed; this takes
//!   a real design fetched by `scripts/fetch-tinytapeout-gds.ps1` into
//!   `scratch/tinytapeout/`, keeps a handful of its real standard-cell
//!   definitions plus a small top that instances them, and re-exports a compact
//!   GDS. The cell names, layer/datatype pairs, coordinates, and record mix are
//!   real; only the tile has been trimmed. If the scratch design is absent (no
//!   fetch was run) this half is skipped with a note, and the malformed half is
//!   still written.
//!
//! Run with:
//! `cargo run -p reticle-io --example gen_tinytapeout_corpus --features corpus-tools`
//!
//! The generator is gated behind the `corpus-tools` feature so it never affects a
//! normal build or the published crate.

use reticle_io::{Gds, ImportWarning};
use reticle_model::{Cell, Document, Exporter, Importer};
use std::path::{Path, PathBuf};

/// The committed corpus directory, relative to the repo root.
const CORPUS_DIR: &str = "corpus/tinytapeout";

/// Where the fetch script drops full real designs (not committed).
const SCRATCH_DIR: &str = "scratch/tinytapeout";

fn main() {
    let root = repo_root();
    let corpus = root.join(CORPUS_DIR);
    std::fs::create_dir_all(&corpus).expect("create corpus dir");

    let mut manifest: Vec<(String, String)> = Vec::new();

    // ---- Synthesized malformed set (always, offline) ---------------------
    for (name, bytes, note) in synth_malformed() {
        let path = corpus.join(&name);
        std::fs::write(&path, &bytes).expect("write malformed sample");
        // Confirm the hardened importer neither panics nor hangs on it.
        let outcome = describe_import(&bytes);
        println!("wrote {name:<28} {} bytes  [{outcome}]", bytes.len());
        manifest.push((name, format!("{note} ({outcome})")));
    }

    // ---- Minimized real sample (when a fetched design is present) --------
    match minimize_real_sample(&root) {
        Some((name, bytes, note)) => {
            let path = corpus.join(&name);
            std::fs::write(&path, &bytes).expect("write minimized real sample");
            let outcome = describe_import(&bytes);
            println!("wrote {name:<28} {} bytes  [{outcome}]", bytes.len());
            manifest.push((name, format!("{note} ({outcome})")));
        }
        None => {
            println!(
                "note: no fetched design under {SCRATCH_DIR}/; run \
                 scripts/fetch-tinytapeout-gds.ps1 to regenerate the minimized real \
                 sample. Malformed corpus written regardless."
            );
        }
    }

    write_notice(&corpus, &manifest);
    println!("corpus written to {}", corpus.display());
}

/// Builds the synthesized malformed set: `(filename, bytes, provenance note)`.
///
/// Each targets a distinct hazard the hardened importer must survive.
fn synth_malformed() -> Vec<(String, Vec<u8>, &'static str)> {
    vec![
        (
            "malformed_empty.gds".to_owned(),
            Vec::new(),
            "synthesized: zero-length file",
        ),
        (
            "malformed_not_gds.gds".to_owned(),
            b"This is not a GDSII stream, just ASCII text.\n".to_vec(),
            "synthesized: non-GDS bytes (no HEADER record)",
        ),
        (
            "malformed_truncated.gds".to_owned(),
            truncated_after_header(),
            "synthesized: valid HEADER then an abrupt cut mid-stream",
        ),
        (
            "malformed_bad_record_len.gds".to_owned(),
            bad_record_length(),
            "synthesized: a record header claiming length 3 (below the 4-byte minimum)",
        ),
        (
            "malformed_absurd_xy_count.gds".to_owned(),
            absurd_xy_count(),
            "synthesized: an XY record header claiming far more bytes than follow",
        ),
        (
            "malformed_cyclic_sref.gds".to_owned(),
            cyclic_sref(),
            "synthesized: two structs that reference each other (A->B->A)",
        ),
        (
            "malformed_dangling_sref.gds".to_owned(),
            dangling_sref(),
            "synthesized: an SREF to a struct name that is never defined",
        ),
        (
            "degenerate_boundary.gds".to_owned(),
            degenerate_boundary(),
            "synthesized: a good boundary plus a degenerate 2-vertex boundary that \
             imports with a warning, not an error",
        ),
    ]
}

/// Imports `bytes` through the hardened path and returns a one-line description of
/// the outcome (Ok with cell/warning counts, or the error), proving no panic.
fn describe_import(bytes: &[u8]) -> String {
    match Gds.import_with_warnings(bytes) {
        Ok(import) => format!(
            "Ok: {} cells, {} warnings",
            import.document.cell_count(),
            import.warnings.len()
        ),
        Err(e) => format!("Err: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Raw GDSII record helpers for the synthesized malformed set.
// ---------------------------------------------------------------------------

/// GDSII record type codes used below (the high byte of the record header).
mod rtype {
    pub const HEADER: u8 = 0x00;
    pub const BGNLIB: u8 = 0x01;
    pub const LIBNAME: u8 = 0x02;
    pub const UNITS: u8 = 0x03;
    pub const ENDLIB: u8 = 0x04;
    pub const BGNSTR: u8 = 0x05;
    pub const STRNAME: u8 = 0x06;
    pub const ENDSTR: u8 = 0x07;
    pub const BOUNDARY: u8 = 0x08;
    pub const SREF: u8 = 0x0A;
    pub const LAYER: u8 = 0x0D;
    pub const DATATYPE: u8 = 0x0E;
    pub const XY: u8 = 0x10;
    pub const ENDEL: u8 = 0x11;
    pub const SNAME: u8 = 0x12;
}

/// GDSII data type codes (the low byte of the record header).
mod dtype {
    pub const NODATA: u8 = 0x00;
    pub const I16: u8 = 0x02;
    pub const I32: u8 = 0x03;
    pub const F64: u8 = 0x05;
    pub const ASCII: u8 = 0x06;
}

/// Appends one GDSII record: a big-endian `u16` length (payload + 4 header bytes),
/// the record type, the data type, then the payload.
fn record(out: &mut Vec<u8>, rt: u8, dt: u8, payload: &[u8]) {
    let len = (payload.len() + 4) as u16;
    out.extend_from_slice(&len.to_be_bytes());
    out.push(rt);
    out.push(dt);
    out.extend_from_slice(payload);
}

/// Twelve `i16` GDSII date fields (modified then accessed date, each y/m/d/h/m/s),
/// set to a valid calendar date so the record parses. GDSII requires real dates;
/// chrono (inside `gds21`) panics on an out-of-range one, so the malformed set uses
/// this valid stamp except where a date panic is explicitly the hazard under test.
fn valid_dates() -> [u8; 24] {
    let fields: [i16; 12] = [2023, 1, 1, 0, 0, 0, 2023, 1, 1, 0, 0, 0];
    let mut out = [0u8; 24];
    for (i, f) in fields.iter().enumerate() {
        out[i * 2..i * 2 + 2].copy_from_slice(&f.to_be_bytes());
    }
    out
}

/// The standard library preamble: HEADER, BGNLIB, LIBNAME, UNITS.
fn lib_preamble(out: &mut Vec<u8>, libname: &str) {
    record(out, rtype::HEADER, dtype::I16, &3i16.to_be_bytes());
    record(out, rtype::BGNLIB, dtype::I16, &valid_dates());
    record(out, rtype::LIBNAME, dtype::ASCII, ascii(libname).as_slice());
    // UNITS: two f64s (user unit, db unit). GDSII float format; 1 nm grid values
    // borrowed from a real header so the record decodes.
    let mut units = Vec::new();
    units.extend_from_slice(&gds_f64(1e-3)); // user unit = 0.001 (1 nm in um)
    units.extend_from_slice(&gds_f64(1e-9)); // db unit = 1e-9 m
    record(out, rtype::UNITS, dtype::F64, &units);
}

/// An ASCII payload, NUL-padded to an even length as GDSII requires.
fn ascii(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    if !v.len().is_multiple_of(2) {
        v.push(0);
    }
    v
}

/// Encodes an `f64` in GDSII's excess-64 base-16 float format.
fn gds_f64(x: f64) -> [u8; 8] {
    if x == 0.0 {
        return [0; 8];
    }
    let neg = x < 0.0;
    let mut mant = x.abs();
    let mut exp: i32 = 64;
    while mant >= 1.0 {
        mant /= 16.0;
        exp += 1;
    }
    while mant < 1.0 / 16.0 {
        mant *= 16.0;
        exp -= 1;
    }
    let mut out = [0u8; 8];
    out[0] = (exp as u8) & 0x7f;
    if neg {
        out[0] |= 0x80;
    }
    // 56-bit mantissa, most significant byte first.
    let mut frac = mant;
    for byte in out.iter_mut().skip(1) {
        frac *= 256.0;
        let b = frac.floor();
        *byte = b as u8;
        frac -= b;
    }
    out
}

/// A well-formed BOUNDARY element on a real metal layer number, a 1x1 um box.
fn boundary_element(out: &mut Vec<u8>, layer: i16, datatype: i16) {
    record(out, rtype::BOUNDARY, dtype::NODATA, &[]);
    record(out, rtype::LAYER, dtype::I16, &layer.to_be_bytes());
    record(out, rtype::DATATYPE, dtype::I16, &datatype.to_be_bytes());
    // A closed 5-point ring (first vertex repeated) as GDSII requires.
    let ring: [i32; 10] = [0, 0, 1000, 0, 1000, 1000, 0, 1000, 0, 0];
    let mut xy = Vec::new();
    for v in ring {
        xy.extend_from_slice(&v.to_be_bytes());
    }
    record(out, rtype::XY, dtype::I32, &xy);
    record(out, rtype::ENDEL, dtype::NODATA, &[]);
}

/// A HEADER record then an abrupt cut: parsing must fail cleanly, not hang.
fn truncated_after_header() -> Vec<u8> {
    let mut out = Vec::new();
    record(&mut out, rtype::HEADER, dtype::I16, &3i16.to_be_bytes());
    record(&mut out, rtype::BGNLIB, dtype::I16, &[0u8; 24]);
    // Start a LIBNAME record header claiming 10 bytes but supply only 2, then stop.
    out.extend_from_slice(&10u16.to_be_bytes());
    out.push(rtype::LIBNAME);
    out.push(dtype::ASCII);
    out.extend_from_slice(b"ab"); // 4 bytes short of the claimed length
    out
}

/// A record whose declared length (3) is below the 4-byte header minimum, which
/// `gds21` rejects with a record-length error rather than under-reading.
fn bad_record_length() -> Vec<u8> {
    let mut out = Vec::new();
    record(&mut out, rtype::HEADER, dtype::I16, &3i16.to_be_bytes());
    // Malformed: length 3 (< 4). The next byte pair is arbitrary.
    out.extend_from_slice(&3u16.to_be_bytes());
    out.push(rtype::BGNLIB);
    out.push(dtype::I16);
    out
}

/// A BOUNDARY whose XY record header claims a huge byte length that the file does
/// not actually contain: the reader must fail on the short read, never allocate to
/// the claimed size or spin.
fn absurd_xy_count() -> Vec<u8> {
    let mut out = Vec::new();
    lib_preamble(&mut out, "ABSURD");
    record(&mut out, rtype::BGNSTR, dtype::I16, &valid_dates());
    record(&mut out, rtype::STRNAME, dtype::ASCII, &ascii("TOP"));
    record(&mut out, rtype::BOUNDARY, dtype::NODATA, &[]);
    record(&mut out, rtype::LAYER, dtype::I16, &68i16.to_be_bytes());
    record(&mut out, rtype::DATATYPE, dtype::I16, &20i16.to_be_bytes());
    // XY header claims 60000 bytes of coordinates; only 8 follow.
    out.extend_from_slice(&60000u16.to_be_bytes());
    out.push(rtype::XY);
    out.push(dtype::I32);
    out.extend_from_slice(&0i32.to_be_bytes());
    out.extend_from_slice(&0i32.to_be_bytes());
    // No ENDLIB: the reader hits end-of-input mid-record.
    out
}

/// Two structs A and B that each SREF the other. Import must terminate (parsing is
/// linear in bytes) and the model's cycle guards keep bbox/flatten finite.
fn cyclic_sref() -> Vec<u8> {
    let mut out = Vec::new();
    lib_preamble(&mut out, "CYCLIC");
    for (name, refname) in [("A", "B"), ("B", "A")] {
        record(&mut out, rtype::BGNSTR, dtype::I16, &valid_dates());
        record(&mut out, rtype::STRNAME, dtype::ASCII, &ascii(name));
        boundary_element(&mut out, 68, 20);
        record(&mut out, rtype::SREF, dtype::NODATA, &[]);
        record(&mut out, rtype::SNAME, dtype::ASCII, &ascii(refname));
        let mut xy = Vec::new();
        xy.extend_from_slice(&0i32.to_be_bytes());
        xy.extend_from_slice(&0i32.to_be_bytes());
        record(&mut out, rtype::XY, dtype::I32, &xy);
        record(&mut out, rtype::ENDEL, dtype::NODATA, &[]);
        record(&mut out, rtype::ENDSTR, dtype::NODATA, &[]);
    }
    record(&mut out, rtype::ENDLIB, dtype::NODATA, &[]);
    out
}

/// A struct with an SREF to a name that is never defined: import must not treat the
/// dangling reference as fatal (it imports as an instance of a missing cell).
fn dangling_sref() -> Vec<u8> {
    let mut out = Vec::new();
    lib_preamble(&mut out, "DANGLING");
    record(&mut out, rtype::BGNSTR, dtype::I16, &valid_dates());
    record(&mut out, rtype::STRNAME, dtype::ASCII, &ascii("TOP"));
    boundary_element(&mut out, 69, 20);
    record(&mut out, rtype::SREF, dtype::NODATA, &[]);
    record(
        &mut out,
        rtype::SNAME,
        dtype::ASCII,
        &ascii("DOES_NOT_EXIST"),
    );
    let mut xy = Vec::new();
    xy.extend_from_slice(&500i32.to_be_bytes());
    xy.extend_from_slice(&500i32.to_be_bytes());
    record(&mut out, rtype::XY, dtype::I32, &xy);
    record(&mut out, rtype::ENDEL, dtype::NODATA, &[]);
    record(&mut out, rtype::ENDSTR, dtype::NODATA, &[]);
    record(&mut out, rtype::ENDLIB, dtype::NODATA, &[]);
    out
}

/// A struct with one good boundary and one degenerate boundary (a 2-vertex ring).
/// The importer keeps the good shape and drops the degenerate one with a
/// [`reticle_io::WarningKind::DegenerateGeometry`] warning, proving the recoverable
/// (warn-not-fail) path end to end.
fn degenerate_boundary() -> Vec<u8> {
    let mut out = Vec::new();
    lib_preamble(&mut out, "DEGEN");
    record(&mut out, rtype::BGNSTR, dtype::I16, &valid_dates());
    record(&mut out, rtype::STRNAME, dtype::ASCII, &ascii("TOP"));
    // Good boundary: a real closed box on met1.
    boundary_element(&mut out, 68, 20);
    // Degenerate boundary: only two vertices, which cannot bound any area.
    record(&mut out, rtype::BOUNDARY, dtype::NODATA, &[]);
    record(&mut out, rtype::LAYER, dtype::I16, &69i16.to_be_bytes());
    record(&mut out, rtype::DATATYPE, dtype::I16, &20i16.to_be_bytes());
    let mut xy = Vec::new();
    for v in [0i32, 0, 1000, 1000] {
        xy.extend_from_slice(&v.to_be_bytes());
    }
    record(&mut out, rtype::XY, dtype::I32, &xy);
    record(&mut out, rtype::ENDEL, dtype::NODATA, &[]);
    record(&mut out, rtype::ENDSTR, dtype::NODATA, &[]);
    record(&mut out, rtype::ENDLIB, dtype::NODATA, &[]);
    out
}

// ---------------------------------------------------------------------------
// Minimized real sample.
// ---------------------------------------------------------------------------

/// Real leaf cells to keep from the fetched design, chosen to exercise a mix of
/// records (boundaries, polygons, paths, and text labels) while staying small.
const KEEP_CELLS: &[&str] = &[
    "sky130_fd_sc_hd__and2_1",
    "sky130_fd_sc_hd__buf_1",
    "sky130_fd_sc_hd__conb_1",
];

/// The fetched design to minimize, and the committed sample's name.
const REAL_SOURCE: &str = "adder";
const REAL_SAMPLE_NAME: &str = "real_tinytapeout_min.gds";

/// Minimizes a fetched Tiny Tapeout design into a small real GDS, or `None` if the
/// design was not fetched into `scratch/tinytapeout/`.
fn minimize_real_sample(root: &Path) -> Option<(String, Vec<u8>, String)> {
    let src = root.join(SCRATCH_DIR).join(format!("{REAL_SOURCE}.gds"));
    let bytes = std::fs::read(&src).ok()?;
    let full = Gds.import(&bytes).ok()?;

    // Build a small document: the real leaf cells verbatim, plus a tiny top that
    // instances each once in a row, all carrying the source's real technology.
    let mut doc = Document::new();
    doc.set_technology(full.technology().clone());

    let mut top = Cell::new("TT_MIN_TOP");
    let mut x = 0i32;
    let mut kept = 0usize;
    for name in KEEP_CELLS {
        if let Some(cell) = full.cell(name) {
            // Instance the real cell in the top at an offset so they do not overlap.
            top.instances.push(reticle_model::Instance {
                cell: (*name).to_owned(),
                transform: reticle_geometry::Transform {
                    translation: reticle_geometry::Point::new(x, 0),
                    ..reticle_geometry::Transform::IDENTITY
                },
            });
            x += 5000; // 5 um apart, well clear of a ~2 um-wide cell
            doc.insert_cell(cell.clone());
            kept += 1;
        }
    }
    if kept == 0 {
        return None; // the expected cells were not in this design
    }
    doc.insert_cell(top);
    doc.set_top_cells(vec!["TT_MIN_TOP".to_owned()]);

    let mut out = Gds.export(&doc).ok()?;
    // `gds21`'s writer stamps BGNLIB/BGNSTR with the wall clock (`Utc::now`), which
    // would make the committed sample differ on every regeneration and embed a
    // build timestamp. Overwrite those date fields with a fixed valid stamp so the
    // corpus is byte-reproducible and carries no wall-clock time.
    normalize_dates(&mut out);
    let note = format!(
        "minimized from Tiny Tapeout 03 design '{REAL_SOURCE}': {kept} real SkyWater \
         leaf cells ({}) under a small synthesized top; re-exported with dates \
         normalized for reproducibility. Apache-2.0.",
        KEEP_CELLS.join(", ")
    );
    Some((REAL_SAMPLE_NAME.to_owned(), out, note))
}

/// Rewrites every BGNLIB and BGNSTR date payload in a GDS record stream to a fixed
/// valid stamp, walking the length-prefixed records in place.
///
/// GDSII records are `[len:u16 BE][rtype][dtype][payload]` with `len` counting the
/// four header bytes; BGNLIB (0x01) and BGNSTR (0x05) carry twelve `i16` date
/// fields. Any record whose length is inconsistent with the buffer stops the walk
/// (the stream is our own well-formed export, so this is belt-and-braces).
fn normalize_dates(bytes: &mut [u8]) {
    const BGNLIB: u8 = 0x01;
    const BGNSTR: u8 = 0x05;
    let stamp = valid_dates();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 4 || i + len > bytes.len() {
            break; // malformed or trailing bytes; stop rather than index past the end
        }
        let rtype = bytes[i + 2];
        if (rtype == BGNLIB || rtype == BGNSTR) && len == 4 + stamp.len() {
            bytes[i + 4..i + len].copy_from_slice(&stamp);
        }
        i += len;
    }
}

// ---------------------------------------------------------------------------
// NOTICE / provenance file.
// ---------------------------------------------------------------------------

/// Writes the corpus provenance file recording every committed sample.
fn write_notice(corpus: &Path, manifest: &[(String, String)]) {
    let mut s = String::new();
    s.push_str("# Tiny Tapeout import-hardening corpus\n\n");
    s.push_str(
        "This directory proves the hardened GDSII importer never panics or hangs and \
         always returns a clean document or a clean error. It is regenerated by \
         `cargo run -p reticle-io --example gen_tinytapeout_corpus --features corpus-tools`.\n\n",
    );
    s.push_str("## Files\n\n");
    s.push_str("| file | provenance |\n|------|------------|\n");
    for (name, note) in manifest {
        use std::fmt::Write as _;
        let _ = writeln!(s, "| `{name}` | {note} |");
    }
    s.push_str(
        "\n## Real-sample provenance\n\n\
         The `real_*` sample is derived from a real, published Tiny Tapeout 03 \
         submitted design fetched by `scripts/fetch-tinytapeout-gds.ps1` from \
         `https://github.com/TinyTapeout/tinytapeout-03` (Apache-2.0). Only a few \
         real SkyWater standard-cell definitions are kept, under a small synthesized \
         top cell, and re-exported so the committed file stays tiny; the cell names, \
         layer/datatype pairs, coordinates, and record mix are the real design's. \
         The full multi-megabyte tiles are intentionally not committed.\n\n\
         The `malformed_*` samples are fully synthesized in-repo (no third-party \
         content) to exercise distinct parser hazards.\n",
    );
    std::fs::write(corpus.join("NOTICE.md"), s).expect("write corpus NOTICE");
}

/// The repo root: the example runs from the crate dir under cargo, so climb two
/// levels from `CARGO_MANIFEST_DIR` (`crates/reticle-io`).
fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}

/// Silences an unused-import warning when the module compiles without the real
/// sample present; `ImportWarning` is part of the public surface this exercises.
#[allow(dead_code)]
fn _uses_warning(_: &ImportWarning) {}

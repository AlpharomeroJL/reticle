//! Integration tests for `reticle convert`: GDSII to a streamable `.rtla` archive.
//!
//! Each test synthesizes a tiny well-formed GDSII library in process (mirroring the
//! `gds_stream` reader's own record framing), writes it to a temp file, and drives
//! [`reticle_cli::run_convert`]. The two guarantees the lane exists to hold are
//! covered here: the conversion is byte-deterministic (same input, identical output
//! bytes), and the archive it writes is read back by lane 2B's real
//! [`MmapTileSource`], so the writer and reader agree end to end.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use pollster::block_on;
use reticle_cli::run_convert;
use reticle_index::archive::ArchivedTilePayload;
use reticle_index::tile_source::MmapTileSource;
use reticle_index::{TileCoord, TileSource};

/// A process-unique counter so parallel tests never collide on a temp filename.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temp path with a unique name and the given extension. Removed by [`TempFile`].
fn temp_path(stem: &str, ext: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("reticle_convert_{stem}_{pid}_{n}.{ext}"));
    path
}

/// An RAII guard that deletes a file (and any `.rtla` build-scratch beside it) on drop.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Minimal GDSII synthesis (the reader's framing: `[len:u16 BE][rtype][dtype][payload]`).
// ---------------------------------------------------------------------------

const RT_HEADER: u8 = 0x00;
const RT_BGNLIB: u8 = 0x01;
const RT_UNITS: u8 = 0x03;
const RT_ENDLIB: u8 = 0x04;
const RT_BGNSTR: u8 = 0x05;
const RT_STRNAME: u8 = 0x06;
const RT_ENDSTR: u8 = 0x07;
const RT_BOUNDARY: u8 = 0x08;
const RT_PATH: u8 = 0x09;
const RT_LAYER: u8 = 0x0D;
const RT_DATATYPE: u8 = 0x0E;
const RT_WIDTH: u8 = 0x0F;
const RT_XY: u8 = 0x10;
const RT_ENDEL: u8 = 0x11;
const DT_STRING: u8 = 0x06;

/// Builds one GDS record: `[len_hi, len_lo, rectype, datatype, payload...]`.
fn record(rectype: u8, datatype: u8, payload: &[u8]) -> Vec<u8> {
    let len = (4 + payload.len()) as u16;
    let mut out = len.to_be_bytes().to_vec();
    out.push(rectype);
    out.push(datatype);
    out.extend_from_slice(payload);
    out
}

/// Big-endian bytes of an XY coordinate list.
fn xy_bytes(points: &[(i32, i32)]) -> Vec<u8> {
    points
        .iter()
        .flat_map(|&(x, y)| {
            let mut b = x.to_be_bytes().to_vec();
            b.extend_from_slice(&y.to_be_bytes());
            b
        })
        .collect()
}

/// Encodes a positive GDSII 8-byte real (used to build a UNITS record).
fn gds_real8(value: f64) -> [u8; 8] {
    assert!(value > 0.0);
    let mut exponent = 0i32;
    let mut v = value;
    while v >= 1.0 {
        v /= 16.0;
        exponent += 1;
    }
    while v < 1.0 / 16.0 {
        v *= 16.0;
        exponent -= 1;
    }
    let mantissa = (v * 72_057_594_037_927_936.0).round() as u64;
    let mut out = [0u8; 8];
    out[0] = ((exponent + 64) as u8) & 0x7f;
    for i in 0..7 {
        out[i + 1] = (mantissa >> (8 * (6 - i))) as u8;
    }
    out
}

/// A well-formed library: HEADER, BGNLIB, UNITS (1000 dbu/µm), one struct with two
/// boundaries and one path on distinct layers, spread across a ~12000-DBU world so the
/// pyramid is several levels deep and several finest tiles carry geometry, then
/// ENDSTR/ENDLIB.
fn sample_library() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(record(RT_HEADER, 0x02, &[0, 3]));
    b.extend(record(RT_BGNLIB, 0x02, &[0u8; 24]));
    let mut units = gds_real8(1e-3).to_vec();
    units.extend_from_slice(&gds_real8(1e-9));
    b.extend(record(RT_UNITS, 0x05, &units));
    b.extend(record(RT_BGNSTR, 0x02, &[0u8; 24]));
    b.extend(record(RT_STRNAME, DT_STRING, b"top\0"));

    // Boundary A on 68/20 near the origin.
    b.extend(record(RT_BOUNDARY, 0x00, &[]));
    b.extend(record(RT_LAYER, 0x02, &68i16.to_be_bytes()));
    b.extend(record(RT_DATATYPE, 0x02, &20i16.to_be_bytes()));
    b.extend(record(
        RT_XY,
        0x03,
        &xy_bytes(&[(0, 0), (300, 0), (300, 300), (0, 300), (0, 0)]),
    ));
    b.extend(record(RT_ENDEL, 0x00, &[]));

    // Boundary B on 69/20 in the far corner, so the world spans (0,0)-(12000,12000).
    b.extend(record(RT_BOUNDARY, 0x00, &[]));
    b.extend(record(RT_LAYER, 0x02, &69i16.to_be_bytes()));
    b.extend(record(RT_DATATYPE, 0x02, &20i16.to_be_bytes()));
    b.extend(record(
        RT_XY,
        0x03,
        &xy_bytes(&[
            (11700, 11700),
            (12000, 11700),
            (12000, 12000),
            (11700, 12000),
            (11700, 11700),
        ]),
    ));
    b.extend(record(RT_ENDEL, 0x00, &[]));

    // A path on 70/0 straight across the middle, so its record spans many columns.
    b.extend(record(RT_PATH, 0x00, &[]));
    b.extend(record(RT_LAYER, 0x02, &70i16.to_be_bytes()));
    b.extend(record(RT_DATATYPE, 0x02, &0i16.to_be_bytes()));
    b.extend(record(RT_WIDTH, 0x03, &40i32.to_be_bytes()));
    b.extend(record(
        RT_XY,
        0x03,
        &xy_bytes(&[(300, 6000), (11700, 6000)]),
    ));
    b.extend(record(RT_ENDEL, 0x00, &[]));

    b.extend(record(RT_ENDSTR, 0x00, &[]));
    b.extend(record(RT_ENDLIB, 0x00, &[]));
    b
}

/// Writes `bytes` to a fresh temp `.gds` path, returning the guard.
fn write_gds(bytes: &[u8]) -> TempFile {
    let path = temp_path("in", "gds");
    std::fs::write(&path, bytes).expect("write gds fixture");
    TempFile(path)
}

#[test]
fn convert_is_byte_deterministic() {
    let input = write_gds(&sample_library());
    let out_a = TempFile(temp_path("det_a", "rtla"));
    let out_b = TempFile(temp_path("det_b", "rtla"));

    run_convert(&input.0, &out_a.0).expect("first conversion");
    run_convert(&input.0, &out_b.0).expect("second conversion");

    let bytes_a = std::fs::read(&out_a.0).expect("read archive a");
    let bytes_b = std::fs::read(&out_b.0).expect("read archive b");
    assert!(!bytes_a.is_empty(), "the archive is non-empty");
    assert_eq!(bytes_a, bytes_b, "same input converts to identical bytes");
}

#[test]
fn converted_archive_round_trips_through_mmap_source() {
    let input = write_gds(&sample_library());
    let out = TempFile(temp_path("rt", "rtla"));

    let summary = run_convert(&input.0, &out.0).expect("conversion");
    // Three drawn elements (two boundaries, one path) became three records.
    assert_eq!(summary.record_count, 3);
    assert_eq!(summary.dbu_per_micron, 1000);

    let src = MmapTileSource::open(&out.0).expect("mmap source opens the archive");
    let header = block_on(src.header()).expect("reader parses the header");
    assert_eq!(header.dbu_per_micron, 1000);
    assert_eq!(header.level_count(), summary.level_count);

    // Every drawn element reaches the exact finest level. Sweep its tiles; each must
    // validate as an archived payload, and their union must cover the input layers.
    let finest = header.level_count() as u32 - 1;
    let dims = header.levels.last().expect("at least one level");
    let mut layers = std::collections::BTreeSet::new();
    for row in 0..dims.rows {
        for col in 0..dims.cols {
            let bytes = block_on(src.tile_bytes(TileCoord {
                level: finest,
                col,
                row,
            }))
            .expect("fetch finest tile");
            let tile = rkyv::access::<ArchivedTilePayload, rkyv::rancor::Error>(&bytes)
                .expect("tile validates as an ArchivedTilePayload");
            for rec in tile.records.iter() {
                layers.insert(u16::from(rec.layer));
            }
        }
    }
    assert!(layers.contains(&68), "boundary A's layer is present");
    assert!(layers.contains(&69), "boundary B's layer is present");
    assert!(layers.contains(&70), "the path's layer is present");
}

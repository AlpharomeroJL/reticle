//! In-browser GDS -> `.rtla` conversion core (lane v8-6c).
//!
//! This is the browser counterpart to the native `reticle convert` command (ADR 0072).
//! It runs the same v1 flatten and world-span leveling, but reads the GDS from a byte
//! slice (rather than a file) and builds the archive in memory (rather than spilling to
//! disk), so it runs unchanged inside a `wasm32-unknown-unknown` Web Worker where there
//! is no filesystem. The heavy lifting is the frozen Wave 2 surface: the streaming
//! [`GdsRecordReader`] (ADR 0062) pulls one event at a time, and
//! [`reticle_index::build_rtla_to_vec`] assembles the `.rtla` bytes.
//!
//! # Flatten scope (v1), mirroring ADR 0072
//!
//! Only directly drawn geometry becomes a record: each `BOUNDARY` and `PATH` bounding
//! box is one [`TileRecord`], in database units as authored. `SREF`/`AREF` references
//! are not composed into world space (that needs a whole-file DOM, the opposite of
//! streaming); a referenced cell's own shapes are still captured in its local frame. A
//! flat GDS round-trips faithfully; a deeply hierarchical one drops its placements, as
//! documented for the native converter.
//!
//! The scan/leveling helpers below are a faithful port of the native
//! `reticle_cli::convert` module; they are duplicated rather than shared because that
//! logic lives in a binary crate. The two must stay in step with ADR 0072.

use reticle_geometry::{Point, Rect};
use reticle_index::streaming::ArchivableRect;
use reticle_index::{
    BuildError, LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileRecord, build_rtla_to_vec,
};
use reticle_io::{GdsEvent, GdsRecordReader};

/// Database units per micron assumed when the input carries no usable `UNITS` record,
/// matching the streaming reader's own fallback and the native converter (ADR 0072).
const DEFAULT_DBU_PER_MICRON: i64 = 1000;

/// Upper bound on pyramid levels (ADR 0072).
const MAX_LEVELS: u32 = 12;

/// Target edge, in database units, of a finest-level tile (ADR 0072).
const TARGET_FINEST_TILE_DBU: i64 = 1024;

/// What [`convert_gds_to_rtla`] produced, for progress reporting and tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConvertSummary {
    /// Number of drawn elements (boundaries and paths) written as tile records.
    pub record_count: u64,
    /// The world bounding box the archive tiles, in database units.
    pub world: Rect,
    /// Database units per micron carried in the archive header.
    pub dbu_per_micron: i64,
    /// Number of pyramid levels in the archive.
    pub level_count: usize,
}

/// Why an in-browser conversion failed.
#[derive(Debug)]
pub enum ConvertError {
    /// The GDS byte stream was malformed (the streaming reader rejected a record).
    Gds(reticle_model::ModelError),
    /// The in-memory archive builder failed (an invalid plan or an rkyv error).
    Build(BuildError),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gds(e) => write!(f, "reading GDS: {e}"),
            Self::Build(e) => write!(f, "building archive: {e}"),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Converts a GDSII byte slice into the bytes of a streamable `.rtla` archive.
///
/// Streams the input twice: once to size the world box and pyramid, once to feed records
/// to the in-memory builder. Neither pass holds the layout other than the record count
/// and running world box; the builder holds the finished archive (browser v1 scope).
///
/// Returns the archive bytes and a [`ConvertSummary`] describing what was written.
///
/// # Errors
///
/// Returns [`ConvertError::Gds`] if the GDS stream is malformed and
/// [`ConvertError::Build`] if the archive cannot be assembled.
pub fn convert_gds_to_rtla(gds: &[u8]) -> Result<(Vec<u8>, ConvertSummary), ConvertError> {
    let scan = scan_gds(gds)?;
    let world = finalize_world(scan.world);
    let levels = plan_levels(world);
    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: scan.dbu_per_micron,
        levels: levels.clone(),
    };

    // Pass 2: restream the same bytes as a lazy record iterator into the builder.
    let records = GdsRecordIter {
        reader: GdsRecordReader::new(std::io::Cursor::new(gds)),
    };
    let bytes = build_rtla_to_vec(&header, records).map_err(ConvertError::Build)?;

    Ok((
        bytes,
        ConvertSummary {
            record_count: scan.count,
            world,
            dbu_per_micron: scan.dbu_per_micron,
            level_count: levels.len(),
        },
    ))
}

/// The running totals pass 1 accumulates while streaming the input.
struct Scan {
    world: Option<Rect>,
    dbu_per_micron: i64,
    count: u64,
}

/// Pass 1: stream every event once, accumulating the world box, the scale, and the
/// record count without ever holding geometry.
fn scan_gds(gds: &[u8]) -> Result<Scan, ConvertError> {
    let mut reader = GdsRecordReader::new(std::io::Cursor::new(gds));
    let mut world: Option<Rect> = None;
    let mut dbu_per_micron = DEFAULT_DBU_PER_MICRON;
    let mut count = 0u64;

    while let Some(event) = reader.next_event().map_err(ConvertError::Gds)? {
        if let GdsEvent::BeginLibrary {
            dbu_per_micron: dbu,
        } = event
        {
            dbu_per_micron = dbu;
        }
        if let Some(record) = event_record(&event) {
            let rect = record.rect.to_rect();
            world = Some(match world {
                Some(w) => union(w, rect),
                None => rect,
            });
            count += 1;
        }
    }

    Ok(Scan {
        world,
        dbu_per_micron,
        count,
    })
}

/// A lazy iterator of tile records over a GDS byte stream, for pass 2. A malformed
/// record ends iteration cleanly; pass 1 has already validated the stream.
struct GdsRecordIter<'a> {
    reader: GdsRecordReader<std::io::Cursor<&'a [u8]>>,
}

impl Iterator for GdsRecordIter<'_> {
    type Item = TileRecord;

    fn next(&mut self) -> Option<TileRecord> {
        loop {
            match self.reader.next_event() {
                Ok(Some(event)) => {
                    if let Some(record) = event_record(&event) {
                        return Some(record);
                    }
                }
                Ok(None) | Err(_) => return None,
            }
        }
    }
}

/// Maps one event to its tile record, or `None` for events that draw nothing.
fn event_record(event: &GdsEvent) -> Option<TileRecord> {
    match event {
        GdsEvent::Boundary {
            layer,
            datatype,
            xy,
        } => Some(TileRecord {
            layer: *layer,
            datatype: *datatype,
            rect: ArchivableRect::from_rect(vertices_bbox(xy)?),
        }),
        GdsEvent::Path {
            layer,
            datatype,
            width,
            xy,
        } => Some(TileRecord {
            layer: *layer,
            datatype: *datatype,
            rect: ArchivableRect::from_rect(path_bbox(xy, *width)?),
        }),
        _ => None,
    }
}

/// The bounding box of a vertex list, or `None` if it is empty.
fn vertices_bbox(xy: &[(i32, i32)]) -> Option<Rect> {
    let mut points = xy.iter().copied();
    let (fx, fy) = points.next()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (fx, fy, fx, fy);
    for (x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Some(Rect::new(
        Point::new(min_x, min_y),
        Point::new(max_x, max_y),
    ))
}

/// The bounding box of a path: its centreline bbox inflated by half its width on every
/// side. Saturating arithmetic keeps an extreme width from overflowing.
fn path_bbox(xy: &[(i32, i32)], width: i32) -> Option<Rect> {
    let bbox = vertices_bbox(xy)?;
    let half = (width / 2).max(0);
    Some(Rect::new(
        Point::new(
            bbox.min.x.saturating_sub(half),
            bbox.min.y.saturating_sub(half),
        ),
        Point::new(
            bbox.max.x.saturating_add(half),
            bbox.max.y.saturating_add(half),
        ),
    ))
}

/// The union of two rectangles.
fn union(a: Rect, b: Rect) -> Rect {
    Rect::new(
        Point::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
        Point::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
    )
}

/// Turns the scanned world box into one the builder accepts: a positive-area rectangle
/// (empty input -> a unit box; a collinear/degenerate box nudged out one DBU).
fn finalize_world(world: Option<Rect>) -> Rect {
    let Some(w) = world else {
        return Rect::new(Point::new(0, 0), Point::new(1, 1));
    };
    let max_x = if w.max.x > w.min.x {
        w.max.x
    } else {
        w.min.x.saturating_add(1)
    };
    let max_y = if w.max.y > w.min.y {
        w.max.y
    } else {
        w.min.y.saturating_add(1)
    };
    Rect::new(Point::new(w.min.x, w.min.y), Point::new(max_x, max_y))
}

/// Builds the pyramid grid dimensions, coarsest (index 0) to finest (last), sized so a
/// finest-level tile is roughly [`TARGET_FINEST_TILE_DBU`] across (ADR 0072).
fn plan_levels(world: Rect) -> Vec<LevelDims> {
    let width = i64::from(world.max.x) - i64::from(world.min.x);
    let height = i64::from(world.max.y) - i64::from(world.min.y);
    let span = width.max(height).max(1);
    let ideal = (span / TARGET_FINEST_TILE_DBU).max(1) as u64;
    let count = (64 - ideal.leading_zeros()).clamp(1, MAX_LEVELS);
    (0..count)
        .map(|i| {
            let n = 1u32 << i;
            LevelDims { cols: n, rows: n }
        })
        .collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use pollster::block_on;
    use reticle_index::archive::ArchivedTilePayload;
    use reticle_index::tile_source::MmapTileSource;
    use reticle_index::{TileCoord, TileSource};
    use std::sync::atomic::{AtomicU32, Ordering};

    // ----- Minimal GDSII synthesis (the reader's framing), mirroring the native
    // `reticle-cli` convert test so both converters are exercised on the same input.

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

    fn record(rectype: u8, datatype: u8, payload: &[u8]) -> Vec<u8> {
        let len = (4 + payload.len()) as u16;
        let mut out = len.to_be_bytes().to_vec();
        out.push(rectype);
        out.push(datatype);
        out.extend_from_slice(payload);
        out
    }

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

    /// A well-formed library: two boundaries and a path across a ~12000-DBU world, so
    /// the pyramid is several levels deep. Identical to the native converter's fixture.
    fn sample_library() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(record(RT_HEADER, 0x02, &[0, 3]));
        b.extend(record(RT_BGNLIB, 0x02, &[0u8; 24]));
        let mut units = gds_real8(1e-3).to_vec();
        units.extend_from_slice(&gds_real8(1e-9));
        b.extend(record(RT_UNITS, 0x05, &units));
        b.extend(record(RT_BGNSTR, 0x02, &[0u8; 24]));
        b.extend(record(RT_STRNAME, DT_STRING, b"top\0"));

        b.extend(record(RT_BOUNDARY, 0x00, &[]));
        b.extend(record(RT_LAYER, 0x02, &68i16.to_be_bytes()));
        b.extend(record(RT_DATATYPE, 0x02, &20i16.to_be_bytes()));
        b.extend(record(
            RT_XY,
            0x03,
            &xy_bytes(&[(0, 0), (300, 0), (300, 300), (0, 300), (0, 0)]),
        ));
        b.extend(record(RT_ENDEL, 0x00, &[]));

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

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_rtla() -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("web_convert_{}_{n}.rtla", std::process::id()))
    }

    #[test]
    fn converts_gds_bytes_to_a_readable_archive() {
        let (bytes, summary) = convert_gds_to_rtla(&sample_library()).expect("conversion succeeds");

        // Three drawn elements (two boundaries, one path) became three records.
        assert_eq!(summary.record_count, 3);
        assert_eq!(summary.dbu_per_micron, 1000);
        assert!(!bytes.is_empty(), "archive is non-empty");

        // Read the archive back through the real frozen reader. Write to a temp file so
        // this exercises the same MmapTileSource the app uses, mirroring the Wave 2
        // cross-test. (Native test only; wasm has no filesystem, hence the cfg gate.)
        let path = temp_rtla();
        std::fs::write(&path, &bytes).expect("write archive");

        let src = MmapTileSource::open(&path).expect("mmap source opens the archive");
        let header = block_on(src.header()).expect("reader parses the header");
        assert_eq!(header.dbu_per_micron, 1000);
        assert_eq!(header.level_count(), summary.level_count);

        let finest = header.level_count() as u32 - 1;
        let dims = *header.levels.last().expect("at least one level");
        let mut layers = std::collections::BTreeSet::new();
        for row in 0..dims.rows {
            for col in 0..dims.cols {
                let tile = block_on(src.tile_bytes(TileCoord {
                    level: finest,
                    col,
                    row,
                }))
                .expect("fetch finest tile");
                let payload = rkyv::access::<ArchivedTilePayload, rkyv::rancor::Error>(&tile)
                    .expect("tile validates");
                for rec in payload.records.iter() {
                    layers.insert(u16::from(rec.layer));
                }
            }
        }
        assert!(layers.contains(&68), "boundary A layer present");
        assert!(layers.contains(&69), "boundary B layer present");
        assert!(layers.contains(&70), "path layer present");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_input_yields_a_valid_unit_archive() {
        // No structures at all: an empty but valid archive, matching ADR 0072.
        let mut b = Vec::new();
        b.extend(record(RT_HEADER, 0x02, &[0, 3]));
        b.extend(record(RT_BGNLIB, 0x02, &[0u8; 24]));
        let mut units = gds_real8(1e-3).to_vec();
        units.extend_from_slice(&gds_real8(1e-9));
        b.extend(record(RT_UNITS, 0x05, &units));
        b.extend(record(RT_ENDLIB, 0x00, &[]));

        let (bytes, summary) = convert_gds_to_rtla(&b).expect("empty conversion succeeds");
        assert_eq!(summary.record_count, 0);
        assert_eq!(&bytes[0..8], &RTLA_MAGIC, "still a valid .rtla");
    }
}

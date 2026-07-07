//! `reticle convert <in.gds> <out.rtla>`: a GDSII file to a streamable `.rtla` archive.
//!
//! The conversion never holds the whole layout in memory. It uses the forward-only
//! [`GdsRecordReader`] (lane 2A) to pull one record at a time and the external
//! [`build_rtla`] builder (lane 2A), which spills to disk, so peak memory is bounded no
//! matter how large the input die is. Two streaming passes over the file do the work:
//!
//! 1. **Scan** (`scan_gds`): stream every event once to accumulate the world bounding
//!    box, recover `dbu_per_micron`, and count the drawn elements. Only running totals
//!    are held, never the geometry.
//! 2. **Build**: reopen the file as a lazy [`Iterator`] of [`TileRecord`]s
//!    (`GdsRecordIter`) and hand it to [`build_rtla`], which streams and spills. The
//!    header from pass 1 fixes the world box, scale, and pyramid.
//!
//! # Flatten scope (v1)
//!
//! Only *directly drawn* geometry becomes a record: each `BOUNDARY` and `PATH` bounding
//! box is one [`TileRecord`], in database units exactly as authored. Instance and array
//! references (`SREF`/`AREF`) are **not** composed into world space, because expanding a
//! placement needs random access to the referenced structure's geometry, which may be
//! defined anywhere in the file -- that is a whole-file DOM, the very thing the streaming
//! reader exists to avoid. A referenced cell's own shapes are still captured where they
//! are drawn, in that cell's local frame. So a hierarchical GDS converts to the union of
//! every cell's drawn shapes in their own coordinates. See ADR 00XX; true hierarchical
//! flattening is a documented follow-up.
//!
//! # Determinism
//!
//! Records are emitted in document order, the world box is an order-independent union,
//! the pyramid is a pure function of the world span, and [`build_rtla`] carries no
//! timestamps. So the same input produces byte-identical output; a test asserts it.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use reticle_geometry::{Point, Rect};
use reticle_index::streaming::ArchivableRect;
use reticle_index::{LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileRecord, build_rtla};
use reticle_io::{GdsEvent, GdsRecordReader};

use crate::{CliError, Result};

/// Database units per micron assumed when the input carries no usable `UNITS` record,
/// matching the [`reticle_io::gds_stream`] reader's own fallback.
const DEFAULT_DBU_PER_MICRON: i64 = 1000;

/// Upper bound on pyramid levels, so a vast die cannot ask for an unboundedly deep
/// pyramid. At the cap the finest level is `2^(MAX_LEVELS - 1)` tiles per axis.
const MAX_LEVELS: u32 = 12;

/// Target edge, in database units, of a finest-level tile. The pyramid depth is chosen
/// so the finest tile is roughly this size; smaller worlds get shallower pyramids.
const TARGET_FINEST_TILE_DBU: i64 = 1024;

/// What [`run_convert`] produced, for the CLI to print and for tests to assert on.
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

/// Converts the GDSII file at `input` into a `.rtla` archive at `output`.
///
/// Streams the input twice (see the [module docs](self)): once to size the world box
/// and pyramid, once to feed records to the external builder. Neither pass holds the
/// whole layout, so the memory ceiling is the builder's spill budget, not the file size.
///
/// # Errors
///
/// Returns [`CliError::Io`] if the input cannot be opened, [`CliError::Model`] if the
/// GDSII stream is malformed, and [`CliError::Build`] if the archive cannot be written.
pub fn run_convert(input: &Path, output: &Path) -> Result<ConvertSummary> {
    let scan = scan_gds(input)?;
    let world = finalize_world(scan.world);
    let levels = plan_levels(world);
    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: scan.dbu_per_micron,
        levels: levels.clone(),
    };

    let records = GdsRecordIter {
        reader: GdsRecordReader::new(open_reader(input)?),
    };
    build_rtla(&header, records, output).map_err(CliError::Build)?;

    Ok(ConvertSummary {
        record_count: scan.count,
        world,
        dbu_per_micron: scan.dbu_per_micron,
        level_count: levels.len(),
    })
}

/// The running totals pass 1 accumulates while streaming the input.
struct Scan {
    /// Union of every drawn element's bounding box, or `None` if the file drew nothing.
    world: Option<Rect>,
    /// The recovered database resolution (or [`DEFAULT_DBU_PER_MICRON`]).
    dbu_per_micron: i64,
    /// Count of drawn elements turned into records.
    count: u64,
}

/// Pass 1: stream every event once, accumulating the world box, the scale, and the
/// record count without ever holding geometry.
///
/// # Errors
///
/// Propagates a [`CliError::Io`] on open failure and a [`CliError::Model`] if the
/// `GdsRecordReader` reports a malformed record.
fn scan_gds(input: &Path) -> Result<Scan> {
    let mut reader = GdsRecordReader::new(open_reader(input)?);
    let mut world: Option<Rect> = None;
    let mut dbu_per_micron = DEFAULT_DBU_PER_MICRON;
    let mut count = 0u64;

    while let Some(event) = reader.next_event()? {
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

/// A lazy iterator of tile records over a GDSII file, for pass 2. It reopens and
/// restreams the input, yielding one [`TileRecord`] per drawn boundary/path and
/// skipping everything else. A malformed record ends iteration cleanly; pass 1 has
/// already validated the stream, so the builder sees the same records either way.
struct GdsRecordIter {
    /// The forward-only reader over the reopened file.
    reader: GdsRecordReader<BufReader<File>>,
}

impl Iterator for GdsRecordIter {
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

/// Maps one event to its tile record, or `None` for events that draw nothing
/// (library/struct boundaries, references, text). A boundary keeps its vertex bbox; a
/// path inflates its centreline bbox by half its width so the drawn wire is covered.
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
/// side, so the drawn wire (not just its axis) is covered. Saturating arithmetic keeps
/// an extreme width from overflowing.
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

/// Turns the scanned world box into one the builder accepts: a positive-area rectangle.
///
/// An empty input (no drawn geometry) yields a unit box at the origin. A degenerate box
/// (all geometry collinear, so zero width or height) is nudged out by one DBU on the
/// flat axis so [`build_rtla`] does not reject it for non-positive area.
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

/// Builds the pyramid grid dimensions, coarsest (index 0) to finest (last).
///
/// The depth is chosen so a finest-level tile is roughly [`TARGET_FINEST_TILE_DBU`]
/// across, clamped to `[1, MAX_LEVELS]` levels. Each level `i` is a square
/// `2^i x 2^i` grid over the world box, so the finest level is `2^(count-1)` tiles per
/// axis. A square grid over a very non-square world gives non-square tiles, which the
/// builder's tile math handles; a v1 simplification (ADR 00XX).
fn plan_levels(world: Rect) -> Vec<LevelDims> {
    let width = i64::from(world.max.x) - i64::from(world.min.x);
    let height = i64::from(world.max.y) - i64::from(world.min.y);
    let span = width.max(height).max(1);
    // Finest tiles per axis to hit the target tile size, then round to a power of two.
    let ideal = (span / TARGET_FINEST_TILE_DBU).max(1) as u64;
    let count = (64 - ideal.leading_zeros()).clamp(1, MAX_LEVELS);
    (0..count)
        .map(|i| {
            let n = 1u32 << i;
            LevelDims { cols: n, rows: n }
        })
        .collect()
}

/// Opens `input` for buffered reading, tagging any I/O error with the path.
///
/// # Errors
///
/// Returns [`CliError::Io`] if the file cannot be opened.
fn open_reader(input: &Path) -> Result<BufReader<File>> {
    let file = File::open(input).map_err(|source| CliError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    Ok(BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertices_bbox_spans_min_to_max() {
        let bbox = vertices_bbox(&[(3, 7), (1, 9), (5, 2)]).expect("non-empty");
        assert_eq!(bbox, Rect::new(Point::new(1, 2), Point::new(5, 9)));
        assert_eq!(vertices_bbox(&[]), None);
    }

    #[test]
    fn path_bbox_inflates_by_half_width() {
        // A horizontal centreline (0,0)-(100,0) of width 40 covers y in [-20, 20].
        let bbox = path_bbox(&[(0, 0), (100, 0)], 40).expect("non-empty");
        assert_eq!(bbox, Rect::new(Point::new(-20, -20), Point::new(120, 20)));
        // A zero-width path is just its centreline bbox.
        assert_eq!(
            path_bbox(&[(0, 0), (10, 0)], 0).expect("non-empty"),
            Rect::new(Point::new(0, 0), Point::new(10, 0))
        );
    }

    #[test]
    fn finalize_world_gives_positive_area() {
        // Empty input -> a unit box.
        assert_eq!(
            finalize_world(None),
            Rect::new(Point::new(0, 0), Point::new(1, 1))
        );
        // A vertical-line world (zero width) is nudged out on x.
        let flat = Rect::new(Point::new(5, 0), Point::new(5, 10));
        let fixed = finalize_world(Some(flat));
        assert!(fixed.max.x > fixed.min.x && fixed.max.y > fixed.min.y);
    }

    #[test]
    fn plan_levels_scales_with_span_and_caps() {
        // A tiny world gets a single 1x1 level.
        let small = plan_levels(Rect::new(Point::new(0, 0), Point::new(10, 10)));
        assert_eq!(small, vec![LevelDims { cols: 1, rows: 1 }]);

        // A ~12000-DBU span gives four levels: 1,2,4,8 tiles per axis, finest last.
        let mid = plan_levels(Rect::new(Point::new(0, 0), Point::new(12000, 12000)));
        assert_eq!(mid.len(), 4);
        assert_eq!(*mid.last().unwrap(), LevelDims { cols: 8, rows: 8 });

        // A vast world is capped at MAX_LEVELS.
        let huge = plan_levels(Rect::new(Point::new(0, 0), Point::new(1 << 30, 1 << 30)));
        assert_eq!(huge.len(), MAX_LEVELS as usize);
    }

    #[test]
    fn event_record_ignores_non_drawn_events() {
        assert_eq!(event_record(&GdsEvent::EndStruct), None);
        assert_eq!(
            event_record(&GdsEvent::StructRef {
                name: "sub".into(),
                x: 0,
                y: 0
            }),
            None
        );
        let boundary = GdsEvent::Boundary {
            layer: 68,
            datatype: 20,
            xy: vec![(0, 0), (4, 0), (4, 4), (0, 4)],
        };
        let record = event_record(&boundary).expect("a boundary draws");
        assert_eq!(record.layer, 68);
        assert_eq!(
            record.rect.to_rect(),
            Rect::new(Point::new(0, 0), Point::new(4, 4))
        );
    }
}

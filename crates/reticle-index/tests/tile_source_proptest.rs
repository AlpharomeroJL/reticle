//! The headline correctness proof for lane 2B: a streamed viewport query over a
//! [`MemTileSource`] returns exactly the same records as the in-RAM [`RTreeIndex`]
//! query over the same layout, for hundreds of randomized layouts and viewports.
//!
//! [`MemTileSource`] packs randomized rectangles into per-level tiles per the frozen
//! `.rtla` format (ADR 0062); [`query_viewport`] streams the finest level's overlapping
//! tiles and returns the records that intersect the viewport. That result set must equal
//! the exact spatial answer, for which the bulk-loaded R-tree is the oracle (it is
//! itself property-tested against the brute-force `LinearIndex` in `property.rs`). The
//! async trait is driven by `pollster`, keeping `reticle-index` free of a heavy async
//! runtime.
//!
//! Records are compared as a set of `(layer, datatype, min_x, min_y, max_x, max_y)`
//! tuples: both sides deduplicate identical records identically, so a record packed into
//! several tiles (it is placed in every tile it overlaps) and returned once matches the
//! single oracle hit, and two truly identical records collapse on both sides.

use std::collections::BTreeSet;

use proptest::prelude::*;
use reticle_geometry::{Point, Rect, SpatialIndex};
use reticle_index::archive::LevelDims;
use reticle_index::streaming::ArchivableRect;
use reticle_index::tile_source::{MemTileSource, query_viewport};
use reticle_index::{RTreeIndex, TileRecord, TileSource};

/// Coordinate bound for the world and generated shapes. Moderate so random small
/// viewports meaningfully overlap the generated records.
const BOUND: i32 = 1_000;

/// The identity a record is compared by, independent of which tile carried it.
type RecordKey = (u16, u16, i32, i32, i32, i32);

fn world() -> Rect {
    Rect::new(Point::new(0, 0), Point::new(BOUND, BOUND))
}

fn key(record: &TileRecord) -> RecordKey {
    (
        record.layer,
        record.datatype,
        record.rect.min_x,
        record.rect.min_y,
        record.rect.max_x,
        record.rect.max_y,
    )
}

/// A record with a positive-area rectangle inside the world.
fn record_strategy() -> impl Strategy<Value = TileRecord> {
    (0..8u16, 0..4u16, 0..BOUND, 0..BOUND, 1..=120i32, 1..=120i32).prop_map(
        |(layer, datatype, x, y, w, h)| {
            let r = Rect::new(Point::new(x, y), Point::new(x + w, y + h));
            TileRecord {
                layer,
                datatype,
                rect: ArchivableRect::from_rect(r),
            }
        },
    )
}

/// A positive-area viewport, sometimes reaching outside the world.
fn viewport_strategy() -> impl Strategy<Value = Rect> {
    (-200..BOUND + 200, -200..BOUND + 200, 1..=600i32, 1..=600i32)
        .prop_map(|(x, y, w, h)| Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
}

/// The finest-level grid dimensions to tile the world into (1..=6 per axis, so small
/// viewports touch a few tiles and boundary-spanning records are exercised).
fn grid_strategy() -> impl Strategy<Value = (u32, u32)> {
    (1..=6u32, 1..=6u32)
}

/// The oracle answer: the set of records whose rectangle intersects `viewport`, via the
/// bulk-loaded R-tree over the same records.
fn oracle_keys(records: &[TileRecord], viewport: Rect) -> BTreeSet<RecordKey> {
    let tree = RTreeIndex::bulk_load(
        records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.rect.to_rect(), i)),
    );
    tree.query_rect(viewport)
        .into_iter()
        .map(|&i| key(&records[i]))
        .collect()
}

/// The streamed answer: the set of records [`query_viewport`] returns from a
/// [`MemTileSource`] built over the same records and grid.
fn streamed_keys(
    records: &[TileRecord],
    cols: u32,
    rows: u32,
    viewport: Rect,
) -> BTreeSet<RecordKey> {
    // A coarse level plus the finest level, mirroring a real pyramid; queries resolve
    // against the finest (last) level.
    let levels = [LevelDims { cols: 1, rows: 1 }, LevelDims { cols, rows }];
    let source = MemTileSource::from_records(world(), 1000, &levels, records);
    let header = pollster::block_on(source.header()).expect("header");
    let got = pollster::block_on(query_viewport(&source, &header, viewport)).expect("query");
    got.iter().map(key).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    /// The streamed viewport query equals the in-RAM R-tree query over 600 randomized
    /// layouts, grids, and viewports. This is the wave's headline correctness proof.
    #[test]
    fn streamed_viewport_query_equals_rtree_query(
        records in prop::collection::vec(record_strategy(), 0..60),
        (cols, rows) in grid_strategy(),
        viewport in viewport_strategy(),
    ) {
        let expected = oracle_keys(&records, viewport);
        let got = streamed_keys(&records, cols, rows, viewport);
        prop_assert_eq!(got, expected);
    }
}

// --- Explicit edge cases -----------------------------------------------------

#[test]
fn empty_layout_yields_no_records() {
    let expected = oracle_keys(&[], world());
    let got = streamed_keys(&[], 4, 4, world());
    assert!(expected.is_empty());
    assert_eq!(got, expected);
}

#[test]
fn viewport_entirely_outside_the_world_is_empty() {
    let records: Vec<TileRecord> = (0..20)
        .map(|i| {
            let x = (i * 40) % BOUND;
            TileRecord {
                layer: 1,
                datatype: 0,
                rect: ArchivableRect::from_rect(Rect::new(
                    Point::new(x, x),
                    Point::new(x + 10, x + 10),
                )),
            }
        })
        .collect();
    let outside = Rect::new(
        Point::new(BOUND + 100, BOUND + 100),
        Point::new(BOUND + 200, BOUND + 200),
    );
    assert_eq!(
        streamed_keys(&records, 5, 5, outside),
        oracle_keys(&records, outside)
    );
    assert!(streamed_keys(&records, 5, 5, outside).is_empty());
}

#[test]
fn whole_world_viewport_returns_every_record_once() {
    let records: Vec<TileRecord> = (0..30)
        .map(|i| {
            let x = (i * 31) % (BOUND - 50);
            let y = (i * 17) % (BOUND - 50);
            TileRecord {
                layer: (i % 5) as u16,
                datatype: 0,
                rect: ArchivableRect::from_rect(Rect::new(
                    Point::new(x, y),
                    Point::new(x + 40, y + 40),
                )),
            }
        })
        .collect();
    // A grid that forces boundary spanning, so dedup across tiles is exercised.
    let got = streamed_keys(&records, 6, 6, world());
    let expected = oracle_keys(&records, world());
    assert_eq!(got, expected);
    // Distinct records here, so the count matches the deduplicated set exactly.
    let distinct: BTreeSet<_> = records.iter().map(key).collect();
    assert_eq!(got.len(), distinct.len());
}

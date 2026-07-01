//! Tests for the `rkyv` zero-copy streaming of index payloads.

use reticle_geometry::{Point, Rect};
use reticle_index::streaming::{self, ArchivableRect, IndexPayload};

fn sample_entries() -> Vec<(Rect, u32)> {
    vec![
        (Rect::new(Point::new(0, 0), Point::new(10, 10)), 0),
        (Rect::new(Point::new(-50, 20), Point::new(-30, 40)), 1),
        (
            Rect::new(
                Point::new(1_000_000, -2_000_000),
                Point::new(1_000_100, -1_999_900),
            ),
            42,
        ),
    ]
}

#[test]
fn round_trips_through_load() {
    let payload = IndexPayload::from_entries(sample_entries());
    let bytes = streaming::serialize(&payload).expect("serialize");
    let restored = streaming::load(&bytes).expect("load");
    assert_eq!(restored, payload);
    assert_eq!(restored.to_entries(), sample_entries());
}

#[test]
fn zero_copy_access_reads_entries_in_place() {
    let payload = IndexPayload::from_entries(sample_entries());
    let bytes = streaming::serialize(&payload).expect("serialize");

    let archived = streaming::access(&bytes).expect("access");
    assert_eq!(streaming::len(archived), 3);
    for (i, expected) in sample_entries().into_iter().enumerate() {
        assert_eq!(streaming::entry_at(archived, i), Some(expected));
    }
    assert_eq!(streaming::entry_at(archived, 3), None);
}

#[test]
fn empty_payload_round_trips() {
    let payload = IndexPayload::default();
    let bytes = streaming::serialize(&payload).expect("serialize");
    let archived = streaming::access(&bytes).expect("access");
    assert_eq!(streaming::len(archived), 0);
    assert_eq!(streaming::load(&bytes).expect("load"), payload);
}

#[test]
fn corrupt_bytes_are_rejected_not_ub() {
    let payload = IndexPayload::from_entries(sample_entries());
    let mut bytes = streaming::serialize(&payload).expect("serialize");
    // Truncating the buffer must be reported as an error, never a crash.
    bytes.truncate(bytes.len() / 2);
    assert!(streaming::access(&bytes).is_err());
    assert!(streaming::load(&bytes).is_err());
}

#[test]
fn archivable_rect_conversions_are_inverse() {
    let r = Rect::new(Point::new(-7, 3), Point::new(11, 29));
    assert_eq!(ArchivableRect::from_rect(r).to_rect(), r);
}

//! F3 trace-query contract cross-test.
//!
//! The producer (the trace-api query layer, Phase 2) and the consumer (the trace UI) agree
//! on `tests/fixtures/contracts/f3_trace.json`: canned net-at-point, net-extent, and
//! shorts/opens responses, each carrying the `revision` envelope that is the per-document
//! cache key. This test pins the fixture and the revision-sharing invariant, and round-trips
//! every record through serde.

use reticle_extract::query::{NetAtPoint, NetExtent, ShortsOpensReport};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/contracts/f3_trace.json");

#[test]
fn f3_records_parse_and_share_a_revision() {
    let v: Value = serde_json::from_str(FIXTURE).expect("F3 fixture parses");
    let at_point: NetAtPoint = serde_json::from_value(v["net_at_point"].clone()).unwrap();
    let extent: NetExtent = serde_json::from_value(v["net_extent"].clone()).unwrap();
    let report: ShortsOpensReport = serde_json::from_value(v["report"].clone()).unwrap();

    // The revision envelope is the cache key: every record in one query snapshot shares it,
    // so the UI knows the whole snapshot is stale together when the document moves on.
    assert_eq!(at_point.revision, 7);
    assert_eq!(extent.revision, at_point.revision);
    assert_eq!(report.revision, at_point.revision);

    // net-at-point hit: the point is on VDD, over three shapes.
    let net = at_point.net.expect("the point is on a net");
    assert_eq!(net.name, "VDD");
    assert_eq!(net.shape_indices, vec![0, 3, 5]);

    // net-extent: bounding box and shape count.
    assert_eq!(extent.net, "VDD");
    assert_eq!(extent.shape_count, 3);
    assert_eq!(extent.bbox.max_x, 12_000);

    // shorts/opens report: not clean, and navigable (n items).
    assert!(!report.is_clean());
    assert_eq!(report.len(), 2);
    assert_eq!(report.shorts[0].net_a, "VDD");
    assert_eq!(report.shorts[0].net_b, "GND");
    assert_eq!(report.opens[0].net, "CLK");
    assert_eq!(report.opens[0].pieces, 2);

    // Every record round-trips through serde unchanged.
    for value in [&v["net_at_point"], &v["net_extent"], &v["report"]] {
        let text = serde_json::to_string(value).unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value, &back);
    }
}

#[test]
fn f3_empty_report_is_clean() {
    let clean = ShortsOpensReport::default();
    assert!(clean.is_clean());
    assert!(clean.is_empty());
    assert_eq!(clean.len(), 0);
}

//! F1 gallery-manifest contract cross-test.
//!
//! The producer (the content pipeline, Phase 1) and the consumer (the start-screen gallery
//! UI) both build against `tests/fixtures/contracts/f1_manifest.json`. This test pins the
//! fixture: it parses to the schema, validates the contract invariants (sorted unique ids,
//! a verified die is streamable with a real license hash, an excluded die carries no
//! archive), confirms the verified/excluded split, and round-trips through serde.

use reticle_index::gallery_manifest::{DieEntry, GalleryManifest, License, Streaming};

const FIXTURE: &str = include_str!("fixtures/contracts/f1_manifest.json");

#[test]
fn f1_fixture_parses_and_validates() {
    let manifest: GalleryManifest = serde_json::from_str(FIXTURE).expect("F1 fixture parses");
    manifest
        .validate()
        .expect("the fixture must satisfy the F1 contract");
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.dies.len(), 2);

    // Die 0: verified and streamable, with a landmark deep link.
    let alpha = &manifest.dies[0];
    assert_eq!(alpha.id, "sky130.alpha-inverter");
    assert!(matches!(alpha.license, License::Verified { .. }));
    let streaming = alpha.streaming.as_ref().expect("verified die streams");
    assert!(!streaming.archive_key.is_empty());
    assert_eq!(alpha.landmarks.len(), 1);
    assert_eq!(alpha.landmarks[0].view.zoom_milli, 250);

    // Die 1: excluded, so it is ledgered but carries no archive.
    let beta = &manifest.dies[1];
    assert!(matches!(beta.license, License::Excluded { .. }));
    assert!(beta.streaming.is_none(), "an excluded die is not uploaded");

    // The gallery shows only verified, streamable dies.
    let shown = manifest
        .dies
        .iter()
        .filter(|d| matches!(d.license, License::Verified { .. }))
        .count();
    assert_eq!(shown, 1);

    // Serde round-trips exactly.
    let reserialized = serde_json::to_string(&manifest).expect("serialize");
    let reparsed: GalleryManifest = serde_json::from_str(&reserialized).expect("reparse");
    assert_eq!(manifest, reparsed);
}

#[test]
fn f1_validate_rejects_contract_violations() {
    let base: GalleryManifest = serde_json::from_str(FIXTURE).unwrap();
    let verified_die = base.dies[0].clone();

    // A verified die with no streaming archive is rejected.
    let mut no_archive = verified_die.clone();
    no_archive.streaming = None;
    assert!(single(no_archive).validate().is_err());

    // A verified die with a bad license text hash is rejected.
    let mut bad_hash = verified_die.clone();
    if let License::Verified { text_sha256, .. } = &mut bad_hash.license {
        *text_sha256 = "NOTHEX".to_owned();
    }
    assert!(single(bad_hash).validate().is_err());

    // An excluded die that still carries an archive is rejected.
    let mut leaky = base.dies[1].clone();
    leaky.streaming = Some(Streaming {
        archive_key: "x/leak.rtla".to_owned(),
        tile_count: 1,
        total_bytes: 1,
    });
    assert!(single(leaky).validate().is_err());

    // Out-of-order ids are rejected.
    let unsorted = GalleryManifest {
        version: 1,
        dies: vec![
            with_id(&verified_die, "z.die"),
            with_id(&verified_die, "a.die"),
        ],
    };
    assert!(unsorted.validate().is_err());
}

fn single(die: DieEntry) -> GalleryManifest {
    GalleryManifest {
        version: 1,
        dies: vec![die],
    }
}

fn with_id(die: &DieEntry, id: &str) -> DieEntry {
    DieEntry {
        id: id.to_owned(),
        ..die.clone()
    }
}

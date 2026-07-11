//! Cross-crate proof that the start-screen gallery ([`reticle_app::gallery`]) renders
//! the F1 manifest fixture generically.
//!
//! `crates/reticle-index/tests/fixtures/contracts/f1_manifest.json` is the frozen F1
//! contract fixture: `crates/reticle-index/tests/f1_manifest.rs`'s own module doc
//! states "the producer (the content pipeline, Phase 1) and the consumer (the
//! start-screen gallery UI) both build against" it. This file is that consumer side:
//! it parses the same committed fixture reticle-index pins, builds the gallery's
//! cards from it with no per-die code, checks the card count and badges against
//! `GalleryManifest::validate`'s invariant, proves the landmark deep-link builder's
//! query string, and drives `gallery::show` through one headless egui pass (no
//! window, no GPU) so the renderer is proven against real manifest data, not only
//! hand-built test fixtures.
//!
//! A note on the fixture path: the lane brief that scoped this file named the
//! fixture's path as `xtask/tests/fixtures/contracts/f1_manifest.json`; no such path
//! exists in this workspace (`xtask/tests/fixtures/` holds unrelated staged-archive
//! fixtures). The F1 contract fixture that actually exists, is committed, and is
//! pinned by `crates/reticle-index/tests/f1_manifest.rs` lives at
//! `crates/reticle-index/tests/fixtures/contracts/f1_manifest.json`, matching every
//! sibling contract fixture's layout (F2 under `reticle-gen`, F3 under
//! `reticle-extract`, F4 under `reticle-sim`, F5 under `reticle-plugin`). This test
//! builds against that real, existing path.

use eframe::egui;

use reticle_app::gallery;
use reticle_app::theme::components::Ctx;
use reticle_app::theme::tokens::Density;
use reticle_index::gallery_manifest::{GalleryManifest, License};

/// The F1 contract fixture shared with `crates/reticle-index/tests/f1_manifest.rs`.
const FIXTURE: &str = include_str!("../../reticle-index/tests/fixtures/contracts/f1_manifest.json");

#[test]
fn gallery_renders_one_card_per_die_from_the_f1_fixture_generically() {
    let manifest: GalleryManifest = serde_json::from_str(FIXTURE).expect("F1 fixture parses");
    manifest
        .validate()
        .expect("the fixture satisfies the F1 contract");

    let views = gallery::cards(&manifest);
    assert_eq!(
        views.len(),
        manifest.dies.len(),
        "one card per DieEntry, generically, with no per-die code"
    );

    // Per validate()'s invariant: a verified die always carries a streaming archive
    // and an SPDX badge; an excluded die never streams and shows the ledger badge.
    for view in &views {
        match &view.die.license {
            License::Verified { spdx, .. } => {
                assert_eq!(view.license_badge, spdx.as_str());
                assert!(view.streaming, "{} is verified, so it streams", view.die.id);
                assert!(view.archive_url.is_some());
            }
            License::Excluded { .. } => {
                assert_eq!(view.license_badge, "Excluded");
                assert!(
                    !view.streaming,
                    "{} is excluded, so it never streams",
                    view.die.id
                );
                assert!(view.archive_url.is_none());
            }
        }
    }

    // The fixture's own shape (pinned independently by reticle-index's contract
    // test): one verified+streaming die, one excluded die.
    let streaming_count = views.iter().filter(|v| v.streaming).count();
    assert_eq!(streaming_count, 1);
    assert_eq!(views.len(), 2);
}

#[test]
fn gallery_landmark_deep_link_from_the_fixture_composes_and_round_trips() {
    let manifest: GalleryManifest = serde_json::from_str(FIXTURE).expect("F1 fixture parses");
    let alpha = &manifest.dies[0];
    let streaming = alpha
        .streaming
        .as_ref()
        .expect("die 0 in the fixture streams");
    let archive_url =
        gallery::die_archive_url(gallery::DEFAULT_ARCHIVE_BASE_URL, &streaming.archive_key);
    let landmark = &alpha.landmarks[0];

    let link = gallery::landmark_link("", &archive_url, landmark);
    assert!(link.starts_with("?archive="), "relative link: {link}");
    assert!(
        link.contains("&cell=top&view=6000,4000,0.25&layers=68,69"),
        "cell/view/layers composed from the fixture's landmark: {link}"
    );

    assert_eq!(
        reticle_app::share::archive_url_from_query(&link).as_deref(),
        Some(archive_url.as_str()),
        "the archive URL round-trips through the app's own ?archive= parser"
    );
    let permalink = reticle_app::share::parse_permalink(&link);
    assert_eq!(permalink.cell.as_deref(), Some(landmark.cell.as_str()));
    assert_eq!(permalink.camera, Some((6000.0, 4000.0, 0.25)));
}

#[test]
fn gallery_die_link_from_the_fixture_is_archive_only_and_round_trips() {
    let manifest: GalleryManifest = serde_json::from_str(FIXTURE).expect("F1 fixture parses");
    let views = gallery::cards(&manifest);
    let verified = views
        .iter()
        .find(|v| v.streaming)
        .expect("the fixture has one streaming die");
    let archive_url = verified
        .archive_url
        .as_deref()
        .expect("streaming die has a URL");

    let link = gallery::die_link("", archive_url);
    assert_eq!(
        reticle_app::share::archive_url_from_query(&link).as_deref(),
        Some(archive_url)
    );
}

#[test]
fn gallery_show_paints_the_fixture_manifest_headlessly() {
    let manifest: GalleryManifest = serde_json::from_str(FIXTURE).expect("F1 fixture parses");
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    egui::Window::new("f1 gallery fixture test").show(&ctx, |ui| {
        let clicked = gallery::show(ui, Ctx::dark(Density::default()), &manifest, "");
        assert!(
            clicked.is_none(),
            "no button was clicked in a synthetic pass"
        );
    });
    let _ = ctx.end_pass();
}

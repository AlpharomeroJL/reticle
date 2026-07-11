//! Integration proof that the F1 "Open silicon library" section wired onto the Start
//! screen (`reticle_app::gallery`) parses+validates the real, committed
//! `library/gallery-manifest.json`, degrades safely on a broken manifest, and paints
//! without panicking.
//!
//! Companion to `f1_gallery_fixture.rs`, which proves the same renderer against the
//! `reticle-index` contract fixture (`crates/reticle-index/tests/fixtures/contracts/
//! f1_manifest.json`); this file proves it against the actual manifest
//! `reticle_app::gallery::bundled_manifest` compiles in and the Start screen shows.

use eframe::egui;

use reticle_app::gallery;
use reticle_app::theme::components::Ctx;
use reticle_app::theme::tokens::Density;

/// The real, committed F1 manifest yields exactly the two dies `library/README.md`
/// documents: one verified, streaming die (the `SkyWater` sky130 inverter) and one
/// deliberately excluded ledger row proving the fail-closed license path.
#[test]
fn bundled_manifest_yields_the_real_verified_die_and_the_excluded_ledger_row() {
    let manifest =
        gallery::bundled_manifest().expect("the committed library manifest parses and validates");
    let views = gallery::cards(manifest);
    assert_eq!(views.len(), 2, "one verified die, one excluded ledger row");

    let verified: Vec<_> = views.iter().filter(|v| v.streaming).collect();
    assert_eq!(verified.len(), 1, "exactly one verified, streaming die");
    assert_eq!(
        verified[0].archive_url.as_deref(),
        Some("https://reticle-archive.josefdean.workers.dev/74a46ee5d3/sky130.inv-1.rtla"),
        "the verified die's archive URL resolves against the live archive host"
    );

    let excluded: Vec<_> = views.iter().filter(|v| !v.streaming).collect();
    assert_eq!(excluded.len(), 1, "exactly one excluded row");
    assert_eq!(excluded[0].license_badge, "Excluded");
    assert!(
        excluded[0].archive_url.is_none(),
        "an excluded die never carries an archive to open"
    );
    // F1 honesty contract: the excluded row is rendered, not filtered out, and its
    // reason stays readable.
    assert!(
        gallery::excluded_reason(&excluded[0].die.license).is_some(),
        "the excluded die's reason is available to render, not dropped"
    );
}

/// A manifest that fails to parse, or parses but fails `GalleryManifest::validate`,
/// falls back to drawing nothing here rather than ever panicking or showing
/// malformed data. This mirrors the exact fallback wired into the app.rs marked
/// block: `parse_manifest` returning `None` means `gallery::show` is never called,
/// so the Start screen's existing example cards (drawn by a separate, private part
/// of the Start screen this test has no access to) are left completely untouched.
#[test]
fn a_broken_manifest_falls_back_to_no_section_without_panicking() {
    let unparsable = "{ this is not valid json";
    assert!(gallery::parse_manifest(unparsable).is_none());

    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    let mut clicked = None;
    egui::Window::new("broken manifest fallback test").show(&ctx, |ui| {
        if let Some(manifest) = gallery::parse_manifest(unparsable) {
            clicked = gallery::show(ui, Ctx::dark(Density::default()), &manifest, "");
        }
    });
    let _ = ctx.end_pass();
    assert!(
        clicked.is_none(),
        "nothing was drawn, so nothing could have been clicked"
    );
}

/// A headless pass over the real, committed manifest paints the new "Open silicon
/// library" section without a window or a GPU (same `ctx.begin_pass`/`end_pass`
/// pattern as `reticle_app::gallery`'s own headless tests).
#[test]
fn show_paints_the_open_silicon_library_section_over_the_real_manifest() {
    let manifest = gallery::bundled_manifest().expect("the committed manifest loads");
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    egui::Window::new("open silicon library section test").show(&ctx, |ui| {
        let clicked = gallery::show(ui, Ctx::dark(Density::default()), manifest, "");
        assert!(
            clicked.is_none(),
            "no button was clicked in a synthetic pass"
        );
    });
    let _ = ctx.end_pass();
}

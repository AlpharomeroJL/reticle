//! The F1-manifest-driven start-screen die gallery (Phase 1).
//!
//! [`reticle_index::gallery_manifest`] is the frozen F1 contract: a JSON manifest of
//! verified (and ledgered-excluded) dies the content pipeline produces. This module
//! is the *consumer* side of that contract: it turns any
//! [`GalleryManifest`] into gallery cards generically, so a die reaches the Start
//! screen by an entry in the manifest, never a code change here. Every field on
//! [`CardView`] is derived from a single [`DieEntry`] alone ([`card_view`]); [`cards`]
//! maps a whole manifest one die at a time, so adding a die to the manifest adds a
//! card with no change to this file.
//!
//! [`crate::startscreen`] still owns the two compiled-in `ExampleChip`s and the
//! Wave-1 [`crate::startscreen::GALLERY`] (a fixed Rust list extended only by a code
//! change); the two galleries are independent, and this module's own [`Landmark`]
//! (a curated `{cell, view, layers}` deep link) is not
//! [`crate::startscreen::Landmark`] (a compiled-in example's "what is this cell"
//! note). See that module's doc comment for the cross-reference.
//!
//! # What is pure, what draws
//!
//! [`CardView`], [`card_view`], [`cards`], [`license_badge`], [`excluded_reason`],
//! [`dims_label`], [`die_archive_url`], [`die_link`], and [`landmark_link`] are pure
//! data/string logic with no window, unit-tested below with no `egui::Context`.
//! [`show`] is the thin egui rendering pass over a manifest, styled entirely from
//! [`crate::theme::components`] and [`crate::theme::tokens`] (the check-style lint
//! bans raw colors and font sizes outside `crate::theme`); a headless-context test
//! drives it too (no window, no GPU).
//!
//! # Deep links reuse the app's existing query parameters
//!
//! A die opens with `?archive=<url>` ([`die_link`]), the same key
//! [`crate::share::archive_url_from_query`] already reads and the browser boot path
//! (`crates/web/src/main.rs`) already streams on page load. A landmark additionally
//! deep-links to `{cell, view, layers}` ([`landmark_link`]): `cell=`/`view=` are
//! composed with [`crate::share::emit_permalink`], the exact writer
//! [`crate::share::parse_permalink`] already reads (so a landmark link's cell and
//! camera restore through the app's existing, unmodified permalink machinery); see
//! [`landmark_link`]'s own doc comment for why `layers=` is a deliberate exception.
//!
//! # Why no fixture data is wired into the live Start screen this wave
//!
//! The committed F1 contract fixture
//! (`crates/reticle-index/tests/fixtures/contracts/f1_manifest.json`) is this wave's
//! only manifest source; the real, published manifest is Gate 1 integration work.
//! That fixture's dies carry placeholder provenance (`example.org` sources,
//! synthetic license-text hashes) for the sole purpose of exercising the F1 schema's
//! shapes (a verified+streaming die and an excluded die). Showing that placeholder
//! provenance to a real visitor, unlabeled, would read as fabricated content, so
//! [`show`] is exercised only by this crate's own tests
//! (`crates/reticle-app/tests/f1_gallery_fixture.rs` builds it from the real fixture
//! path) rather than being called from the unconditional Start-screen path in
//! `crate::app`. Wiring it in is then a single call once a real manifest source
//! exists; every function here is already generic over any [`GalleryManifest`].

use std::fmt::Write as _;

use eframe::egui::{self, Sense, Vec2};

use reticle_index::gallery_manifest::{DieEntry, GalleryManifest, Landmark, License};

use crate::theme::components::Ctx;

/// The default archive host a die's `archive_key` resolves against:
/// `{DEFAULT_ARCHIVE_BASE_URL}/{archive_key}` (see [`die_archive_url`]).
///
/// This is the one archive host the workspace has actually deployed today, the same
/// host [`crate::startscreen::DEMO_ARCHIVE_URL`] names. Gate 1 should confirm this is
/// where the content pipeline publishes every verified die's `.rtla` object; if a
/// die ever needs a different host, thread a manifest-level base URL through
/// [`die_archive_url`] rather than hard-coding a second constant here.
pub const DEFAULT_ARCHIVE_BASE_URL: &str = "https://reticle-archive.josefdean.workers.dev";

/// Composes the fetchable archive URL for a die's `archive_key`: `{base_url}/{archive_key}`,
/// with exactly one slash between them regardless of a trailing slash on `base_url`.
#[must_use]
pub fn die_archive_url(base_url: &str, archive_key: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/{archive_key}")
}

/// The badge text for a license verdict: the SPDX id when [`License::Verified`], or
/// the literal `"Excluded"` when [`License::Excluded`] (the reason is kept off the
/// badge itself; read it with [`excluded_reason`]).
#[must_use]
pub fn license_badge(license: &License) -> &str {
    match license {
        License::Verified { spdx, .. } => spdx.as_str(),
        License::Excluded { .. } => "Excluded",
    }
}

/// The reason a die is excluded, when its license is [`License::Excluded`]; `None`
/// for a [`License::Verified`] die.
#[must_use]
pub fn excluded_reason(license: &License) -> Option<&str> {
    match license {
        License::Excluded { reason } => Some(reason.as_str()),
        License::Verified { .. } => None,
    }
}

/// A human dimension label from a die's DBU bounding box, e.g. `"12000 x 8000 DBU"`.
///
/// No unit conversion: the F1 contract carries only integer DBU coordinates (see
/// [`reticle_index::gallery_manifest`]'s module docs), so this never introduces a
/// float or an invented micron scale.
#[must_use]
pub fn dims_label(die: &DieEntry) -> String {
    format!("{} x {} DBU", die.width_dbu, die.height_dbu)
}

/// A rendering-ready view of one gallery card, built generically from a [`DieEntry`]
/// by [`card_view`]. Borrows the die (and so its landmarks) rather than cloning it.
#[derive(Clone, Debug)]
pub struct CardView<'a> {
    /// The die this card renders.
    pub die: &'a DieEntry,
    /// The badge text for the die's `license` field (see [`license_badge`]).
    pub license_badge: &'a str,
    /// A human dimension label built from the die's DBU bounding box (see
    /// [`dims_label`]).
    pub dims: String,
    /// Whether the streaming badge shows. Mirrors `die.streaming.is_some()`, which
    /// [`GalleryManifest::validate`] ties one-to-one to a verified license: a
    /// verified die always streams, an excluded die never does.
    pub streaming: bool,
    /// The die's fetchable archive URL (see [`die_archive_url`]), when it streams
    /// one. `None` for an excluded die, which carries no archive to open.
    pub archive_url: Option<String>,
}

/// Builds the rendering-ready [`CardView`] for one die, generically: every field is
/// derived from `die` alone, so a new manifest entry needs no change here.
#[must_use]
pub fn card_view(die: &DieEntry) -> CardView<'_> {
    let archive_url = die
        .streaming
        .as_ref()
        .map(|s| die_archive_url(DEFAULT_ARCHIVE_BASE_URL, &s.archive_key));
    CardView {
        die,
        license_badge: license_badge(&die.license),
        dims: dims_label(die),
        streaming: die.streaming.is_some(),
        archive_url,
    }
}

/// Builds one [`CardView`] per die in `manifest`, in manifest order (the F1 contract
/// keeps `dies` sorted and unique, enforced by [`GalleryManifest::validate`]):
/// adding a die to the manifest adds a card with no code change here.
#[must_use]
pub fn cards(manifest: &GalleryManifest) -> Vec<CardView<'_>> {
    manifest.dies.iter().map(card_view).collect()
}

/// Composes the page URL that opens `archive_url` as a plain streamed die (no
/// particular landmark), hosted at page origin `base_page`.
///
/// A thin, gallery-named wrapper over [`crate::share::emit_archive_link`]: the exact
/// `?archive=` query the browser's boot path
/// ([`crate::share::archive_url_from_query`], `crates/web/src/main.rs`) already
/// parses on page load, so this link works today with no other code change.
#[must_use]
pub fn die_link(base_page: &str, archive_url: &str) -> String {
    crate::share::emit_archive_link(base_page, archive_url)
}

/// Composes the page URL that opens `archive_url` and deep-links straight to
/// `landmark`: the same `?archive=` [`die_link`] carries, plus the landmark's `cell`
/// and `view`, composed exactly as [`crate::share::emit_permalink`] already writes
/// and [`crate::share::parse_permalink`] already reads (so `cell=`/`view=` round-trip
/// through the app's existing permalink parser), and, only when the landmark
/// restricts layers, a `layers=` value.
///
/// # `layers=` is not the permalink's `layer/datatype` shape
///
/// `crate::share::Permalink`'s `layers` field pairs each entry with a datatype: an
/// editable document's per-shape visibility toggle. The frozen F1 `Landmark`'s own
/// `layers` field is bare GDS layer numbers with no datatype: a streamed die's layer
/// filter, a different concept. So this emits a bare comma-separated `layers=` list
/// in the F1 shape (for example `layers=68,69`), and omits the key entirely when
/// `landmark.layers` is empty, matching the F1 contract's own stated meaning ("empty
/// means all visible") rather than the permalink's opposite convention (an
/// empty-but-present `layers=` there means "hide everything"). Wiring a live
/// streamed-viewer layer filter that consumes this bare-number shape is Gate 1
/// work; today's `parse_permalink` would skip each bare number as a malformed
/// `layer/datatype` entry rather than apply it.
#[must_use]
pub fn landmark_link(base_page: &str, archive_url: &str, landmark: &Landmark) -> String {
    let archive_part = crate::share::emit_archive_link("", archive_url);
    let cell = Some(landmark.cell.clone()).filter(|c| !c.is_empty());
    let camera = Some((
        landmark.view.x_dbu as f64,
        landmark.view.y_dbu as f64,
        landmark.view.zoom_milli as f64 / 1000.0,
    ));
    let permalink_part = crate::share::emit_permalink(
        "",
        None,
        &crate::share::Permalink {
            cell,
            camera,
            layers: None,
        },
    );
    let mut query = format!(
        "{}&{}",
        archive_part.trim_start_matches('?'),
        permalink_part.trim_start_matches('?')
    );
    if !landmark.layers.is_empty() {
        let csv = landmark
            .layers
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(query, "&layers={csv}");
    }
    join_query(base_page, &query)
}

/// Joins a composed `query` (no leading `?`) onto `base_page`, mirroring
/// [`crate::share::emit_permalink`]'s own base-page joining: an empty `base_page`
/// yields a relative `?query`, otherwise `base/?query`. Kept local because
/// `landmark_link` composes a query [`crate::share::emit_permalink`] cannot produce
/// on its own (see that function's doc comment), so it cannot delegate the final
/// join either.
fn join_query(base_page: &str, query: &str) -> String {
    let base = base_page.trim().trim_end_matches('/');
    if base.is_empty() {
        format!("?{query}")
    } else {
        format!("{base}/?{query}")
    }
}

/// One small metadata badge (a filled pill with a short label): technology, dims,
/// license, and the streaming flag on a card. Mirrors the Wave-1 gallery's private
/// `App::badge` pixel-for-pixel (that one is not reachable from here: `app.rs` is a
/// shared file this wave, touched only through a small marked block, so this module
/// keeps its own copy), so a manifest-driven card looks identical to the compiled-in
/// examples' cards. Every color comes from `cx.tokens`, never a literal, so
/// check-style's no-raw-color rule holds.
fn badge(ui: &mut egui::Ui, text: &str, fill: egui::Color32, fg: egui::Color32) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
    let pad = Vec2::new(6.0, 2.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().galley(rect.min + pad, galley, fg);
}

/// Draws the F1-manifest die library: one card per die in `manifest`, generically
/// (name, technology/dims/license badges, a streaming badge iff the die streams, and
/// a landmarks list). `base_page` is forwarded to [`die_link`]/[`landmark_link`]
/// (empty for a same-origin relative link).
///
/// Every card carries a `Copy link` action instead of an in-session "Open": clicking
/// copies the die's [`die_link`] (or, for a landmark row, its [`landmark_link`]) to
/// the clipboard, so a visitor pastes it into the address bar (or a new tab) and the
/// existing, unmodified boot path opens straight to that die and view. This module
/// has no access to the private `App` methods (`open_archive_demo`,
/// `apply_permalink`) an in-session click would need, and reusing the app's own
/// query-driven boot path exercises the exact same code a shared/bookmarked link
/// already does, rather than a second, parallel open path.
///
/// An excluded die (no [`CardView`] `archive_url`) renders its metadata with its
/// [`excluded_reason`] in place of the `Copy link` action, and its landmarks (if
/// any) show their label with no link (there is no archive to link to).
pub fn show(ui: &mut egui::Ui, cx: Ctx, manifest: &GalleryManifest, base_page: &str) {
    let t = cx.tokens;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong("Die library");
        ui.label(
            egui::RichText::new("Verified open-silicon dies from the F1 manifest.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);
        if manifest.dies.is_empty() {
            ui.label(
                egui::RichText::new("No dies in the manifest yet.")
                    .weak()
                    .small(),
            );
            return;
        }
        for view in cards(manifest) {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.strong(&view.die.name);
                        ui.horizontal_wrapped(|ui| {
                            badge(ui, &view.die.technology, t.accent_muted, t.text);
                            badge(ui, &view.dims, t.widget_bg, t.text_weak);
                            badge(ui, view.license_badge, t.widget_bg, t.text_weak);
                            if view.streaming {
                                badge(ui, "Streaming", t.success, t.accent_text);
                            }
                        });
                        // Landmarks dropdown, scoped by the die's id (validate()
                        // guarantees it is unique) so identically-labelled headers
                        // across dies never collide.
                        if !view.die.landmarks.is_empty() {
                            ui.push_id(&view.die.id, |ui| {
                                ui.collapsing("What am I looking at?", |ui| {
                                    for lm in &view.die.landmarks {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(&lm.label).strong().small(),
                                            );
                                            if let Some(archive_url) = &view.archive_url
                                                && ui.small_button("Copy link").clicked()
                                            {
                                                let link =
                                                    landmark_link(base_page, archive_url, lm);
                                                ui.ctx().copy_text(link);
                                            }
                                        });
                                    }
                                });
                            });
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(archive_url) = &view.archive_url {
                            if ui.button("Copy link").clicked() {
                                let link = die_link(base_page, archive_url);
                                ui.ctx().copy_text(link);
                            }
                        } else if let Some(reason) = excluded_reason(&view.die.license) {
                            ui.label(egui::RichText::new(reason).weak().small());
                        }
                    });
                });
            });
            ui.add_space(4.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_index::gallery_manifest::{Provenance, Source, Streaming, View};

    /// A verified die fixture, streaming iff `streaming`, carrying one landmark.
    fn verified_die(id: &str, streaming: bool) -> DieEntry {
        DieEntry {
            id: id.to_owned(),
            name: format!("{id} die"),
            technology: "sky130".to_owned(),
            width_dbu: 1000,
            height_dbu: 2000,
            source: Source {
                repo: "example/repo".to_owned(),
                commit: "abc123".to_owned(),
                url: "https://example.test/repo".to_owned(),
            },
            license: License::Verified {
                spdx: "Apache-2.0".to_owned(),
                text_sha256: "a".repeat(64),
            },
            streaming: streaming.then(|| Streaming {
                archive_key: format!("{id}/archive.rtla"),
                tile_count: 4,
                total_bytes: 4096,
            }),
            landmarks: vec![Landmark {
                label: "Top".to_owned(),
                cell: "TOP".to_owned(),
                view: View {
                    x_dbu: 500,
                    y_dbu: 1000,
                    zoom_milli: 250,
                },
                layers: vec![68, 69],
            }],
            provenance: Provenance {
                fetched_utc: "2026-01-01T00:00:00Z".to_owned(),
                converter: "test".to_owned(),
                notice_path: "NOTICE".to_owned(),
            },
        }
    }

    /// An excluded die fixture: no streaming, no landmarks (matches the F1 fixture's
    /// own shape, though `validate()` does not require an excluded die to be
    /// landmark-free).
    fn excluded_die(id: &str) -> DieEntry {
        DieEntry {
            id: id.to_owned(),
            name: format!("{id} die"),
            technology: "gf180".to_owned(),
            width_dbu: 10,
            height_dbu: 20,
            source: Source {
                repo: "example/repo2".to_owned(),
                commit: "def456".to_owned(),
                url: "https://example.test/repo2".to_owned(),
            },
            license: License::Excluded {
                reason: "unidentified license".to_owned(),
            },
            streaming: None,
            landmarks: vec![],
            provenance: Provenance {
                fetched_utc: "2026-01-01T00:00:00Z".to_owned(),
                converter: "test".to_owned(),
                notice_path: "NOTICE".to_owned(),
            },
        }
    }

    #[test]
    fn dims_label_formats_the_dbu_bbox_with_no_unit_conversion() {
        let die = verified_die("a", true);
        assert_eq!(dims_label(&die), "1000 x 2000 DBU");
    }

    #[test]
    fn license_badge_shows_spdx_when_verified_and_excluded_otherwise() {
        let verified = verified_die("a", true);
        assert_eq!(license_badge(&verified.license), "Apache-2.0");
        assert_eq!(excluded_reason(&verified.license), None);

        let excluded = excluded_die("b");
        assert_eq!(license_badge(&excluded.license), "Excluded");
        assert_eq!(
            excluded_reason(&excluded.license),
            Some("unidentified license")
        );
    }

    #[test]
    fn die_archive_url_joins_base_and_key_with_one_slash() {
        assert_eq!(
            die_archive_url("https://host.example", "abc/die.rtla"),
            "https://host.example/abc/die.rtla"
        );
        assert_eq!(
            die_archive_url("https://host.example/", "abc/die.rtla"),
            "https://host.example/abc/die.rtla"
        );
    }

    #[test]
    fn cards_render_one_per_die_generically_with_badges_matching_validate() {
        let manifest = GalleryManifest {
            version: 1,
            dies: vec![excluded_die("a.excluded"), verified_die("b.verified", true)],
        };
        manifest.validate().expect("well-formed test manifest");
        let views = cards(&manifest);
        assert_eq!(views.len(), manifest.dies.len(), "one card per DieEntry");

        let excluded = &views[0];
        assert_eq!(excluded.license_badge, "Excluded");
        assert!(
            !excluded.streaming,
            "an excluded die never streams (validate())"
        );
        assert!(excluded.archive_url.is_none());

        let verified = &views[1];
        assert_eq!(verified.license_badge, "Apache-2.0");
        assert!(
            verified.streaming,
            "a verified die always streams (validate())"
        );
        assert_eq!(
            verified.archive_url.as_deref(),
            Some("https://reticle-archive.josefdean.workers.dev/b.verified/archive.rtla")
        );
    }

    #[test]
    fn adding_a_die_to_the_manifest_adds_a_card_with_no_code_change() {
        // The generic guarantee itself: N arbitrary dies in, N cards out, in order,
        // for any N (including zero).
        for n in 0..5usize {
            let dies: Vec<DieEntry> = (0..n)
                .map(|i| verified_die(&format!("die.{i}"), true))
                .collect();
            let manifest = GalleryManifest { version: 1, dies };
            let views = cards(&manifest);
            assert_eq!(views.len(), n);
            for (i, view) in views.iter().enumerate() {
                assert_eq!(view.die.id, format!("die.{i}"));
            }
        }
    }

    #[test]
    fn die_link_is_archive_only_and_round_trips() {
        let url = "https://reticle-archive.josefdean.workers.dev/k/die.rtla";
        let link = die_link("", url);
        assert_eq!(
            link,
            crate::share::emit_archive_link("", url),
            "a thin, named wrapper over the app's own archive-link writer"
        );
        assert_eq!(
            crate::share::archive_url_from_query(&link).as_deref(),
            Some(url)
        );
    }

    #[test]
    fn landmark_link_composes_archive_cell_view_and_layers_in_order() {
        let lm = Landmark {
            label: "Output driver".to_owned(),
            cell: "top".to_owned(),
            view: View {
                x_dbu: 6000,
                y_dbu: 4000,
                zoom_milli: 250,
            },
            layers: vec![68, 69],
        };
        let link = landmark_link("", "k", &lm);
        assert_eq!(link, "?archive=k&cell=top&view=6000,4000,0.25&layers=68,69");
    }

    #[test]
    fn landmark_link_percent_encodes_the_archive_url_and_round_trips() {
        let archive_url = "https://reticle-archive.josefdean.workers.dev/k/die.rtla?x=1";
        let lm = Landmark {
            label: "Output driver".to_owned(),
            cell: "top".to_owned(),
            view: View {
                x_dbu: 6000,
                y_dbu: 4000,
                zoom_milli: 250,
            },
            layers: vec![],
        };
        let link = landmark_link("https://reticle.example", archive_url, &lm);
        assert!(
            link.starts_with("https://reticle.example/?archive="),
            "absolute base page joins with /?: {link}"
        );
        // Mirrors crate::share's own `archive_link_round_trips_through_the_query`
        // test: archive_url_from_query/parse_permalink read a query string (like
        // `window.location.search`), not a full page URL, so the query is extracted
        // after the '?' first.
        let query = link.split_once('?').expect("link has a query").1;
        assert_eq!(
            crate::share::archive_url_from_query(query).as_deref(),
            Some(archive_url),
            "the archive URL round-trips through the app's own parser even with \
             reserved characters"
        );
        let permalink = crate::share::parse_permalink(query);
        assert_eq!(permalink.cell.as_deref(), Some("top"));
        assert_eq!(permalink.camera, Some((6000.0, 4000.0, 0.25)));
        assert!(
            !link.contains("layers="),
            "no layers key when the landmark names none (F1: empty means all visible): {link}"
        );
    }

    #[test]
    fn landmark_link_omits_cell_when_the_landmark_names_none() {
        let lm = Landmark {
            label: "Whole die".to_owned(),
            cell: String::new(),
            view: View {
                x_dbu: 0,
                y_dbu: 0,
                zoom_milli: 1000,
            },
            layers: vec![],
        };
        let link = landmark_link("", "k", &lm);
        assert_eq!(link, "?archive=k&view=0,0,1");
        assert_eq!(crate::share::parse_permalink(&link).cell, None);
    }

    #[test]
    fn show_paints_one_card_per_die_without_a_window() {
        // A GPU-free headless pass (no wgpu adapter, matching the codebase's other
        // headless egui tests): the primary assertion is that painting a manifest
        // never panics, complementing the data-level assertions above.
        let manifest = GalleryManifest {
            version: 1,
            dies: vec![excluded_die("a.excluded"), verified_die("b.verified", true)],
        };
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::Window::new("gallery test").show(&ctx, |ui| {
            show(
                ui,
                Ctx::dark(crate::theme::tokens::Density::default()),
                &manifest,
                "",
            );
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn show_paints_an_empty_manifest_without_a_window() {
        let manifest = GalleryManifest {
            version: 1,
            dies: vec![],
        };
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::Window::new("gallery empty test").show(&ctx, |ui| {
            show(
                ui,
                Ctx::dark(crate::theme::tokens::Density::default()),
                &manifest,
                "",
            );
        });
        let _ = ctx.end_pass();
    }
}

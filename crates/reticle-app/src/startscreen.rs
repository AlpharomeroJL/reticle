//! The Start-screen model: the example-chip gallery and the recent-files shape.
//!
//! The Start screen (drawn in [`crate::app`]) is a first-time visitor's first
//! contact with Reticle. Beyond the four worked [`UseCase`](crate::usecases::UseCase)
//! scenarios it offers two more ways in that this module supplies the model for:
//!
//! * an **example-chip gallery** of redistribution-cleared real designs, each
//!   compiled into the binary with [`include_bytes!`] and opened through the
//!   document-open seam ([`crate::open`]); and
//! * a **recent-files** list the Start screen displays (the persistence behind it,
//!   `IndexedDB` on the web, is a sibling concern; this module only defines the shape
//!   the list is rendered from).
//!
//! # Why the designs are compiled in
//!
//! There is no filesystem on `wasm32`, and the gallery is the whole no-install
//! story: a visitor to the web build must be able to open a real chip with one
//! click. So each [`ExampleChip`] carries its bytes via [`include_bytes!`] and is
//! opened with [`ExampleChip::open`], which runs the same hardened seam
//! ([`crate::open::open_document_bytes`]) the file-open button and drag-and-drop use.
//! This mirrors how [`crate::usecases`] embeds its SKY130 cell and how
//! [`crate::store`] embeds its transcript, so the module builds identically on native
//! and on the web and is unit-tested without a window or a filesystem.
//!
//! # Portability and testing
//!
//! Everything here is window-free, GPU-free glue over the frozen open seam and
//! `reticle-io`, so the interesting behavior (which chips are offered, that each
//! embedded design opens cleanly through the seam, what top cell it frames) is proven
//! in plain code by the tests at the bottom of the file.

use crate::open::{DocFormat, OpenError, OpenOutcome, open_document_bytes};

/// The minimized real Tiny Tapeout sample, compiled in so the gallery offers a real
/// chip on the web where there is no filesystem.
///
/// This is the Apache-2.0 `real_tinytapeout_min.gds` committed under
/// `corpus/tinytapeout/` (see that directory's `NOTICE.md` for provenance): a few
/// real `SkyWater` standard cells from a published Tiny Tapeout 03 design under a
/// small synthesized top, re-exported compact. It imports as four cells cleanly.
const TINYTAPEOUT_MIN_GDS: &[u8] =
    include_bytes!("../../../corpus/tinytapeout/real_tinytapeout_min.gds");

/// The `SkyWater` SKY130 inverter (`inv_1`) GDSII stream, the same file
/// [`crate::usecases`] embeds for its inspect scenario, offered here as a second,
/// smaller gallery example (a single real standard cell).
const SKY130_INV_GDS: &[u8] = include_bytes!("../assets/sky130_fd_sc_hd__inv_1.gds");

/// One redistribution-cleared design offered in the Start-screen gallery.
///
/// Each variant names a compiled-in layout the gallery can open with one click. The
/// bytes never come from disk, so the gallery works on the web build; opening always
/// routes through the hardened [`crate::open`] seam.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ExampleChip {
    /// The minimized real Tiny Tapeout 03 sample (a few real `SkyWater` cells under a
    /// small top). Apache-2.0.
    TinyTapeoutMin,
    /// A single real `SkyWater` SKY130 standard cell, the `inv_1` inverter.
    Sky130Inverter,
}

impl ExampleChip {
    /// Every gallery chip, in the order the Start screen offers them.
    ///
    /// The real multi-cell Tiny Tapeout sample leads (it is the "real chip" example),
    /// followed by the single standard cell.
    pub const ALL: [ExampleChip; 2] = [ExampleChip::TinyTapeoutMin, ExampleChip::Sky130Inverter];

    /// Maps an `?e2e-example=<id>` value to a chip, for the headed examples guard that
    /// opens each embedded design straight from a URL. The Start-screen cards are
    /// egui-canvas-painted, so a browser test cannot click them; this lets the guard
    /// boot directly into one. `tt03` selects the Tiny Tapeout sample, `sky130` the
    /// inverter cell. Case-insensitive; `None` for an unknown id.
    #[must_use]
    pub fn from_e2e_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "tt03" | "tinytapeout" => Some(ExampleChip::TinyTapeoutMin),
            "sky130" | "inv" | "inv1" => Some(ExampleChip::Sky130Inverter),
            _ => None,
        }
    }

    /// A short title for the gallery card.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            ExampleChip::TinyTapeoutMin => "Tiny Tapeout sample",
            ExampleChip::Sky130Inverter => "SKY130 inverter cell",
        }
    }

    /// A one-line description of the design, shown under the title.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            ExampleChip::TinyTapeoutMin => {
                "A real Tiny Tapeout 03 design: a few SkyWater standard cells under a \
                 small top. Apache-2.0."
            }
            ExampleChip::Sky130Inverter => {
                "A single real SkyWater SKY130 standard cell, the inv_1 inverter, \
                 straight from its GDSII."
            }
        }
    }

    /// A short license/provenance tag shown on the card, so the source is always
    /// visible next to the design.
    #[must_use]
    pub fn attribution(self) -> &'static str {
        match self {
            ExampleChip::TinyTapeoutMin => "Tiny Tapeout 03, Apache-2.0",
            ExampleChip::Sky130Inverter => "SkyWater SKY130, Apache-2.0",
        }
    }

    /// The compiled-in bytes for this design.
    #[must_use]
    pub fn bytes(self) -> &'static [u8] {
        match self {
            ExampleChip::TinyTapeoutMin => TINYTAPEOUT_MIN_GDS,
            ExampleChip::Sky130Inverter => SKY130_INV_GDS,
        }
    }

    /// The format the bytes are in (every bundled example is GDSII today).
    #[must_use]
    pub fn format(self) -> DocFormat {
        match self {
            ExampleChip::TinyTapeoutMin | ExampleChip::Sky130Inverter => DocFormat::Gds,
        }
    }

    /// Opens this example through the document-open seam.
    ///
    /// Runs the exact same hardened path ([`open_document_bytes`]) the file-open
    /// button and drag-and-drop use, so the gallery has no privileged loading route:
    /// on success it yields an [`OpenOutcome`] (document, top cell, and any non-fatal
    /// warnings) the app installs with
    /// [`App::open_outcome`](crate::app::App::open_outcome).
    ///
    /// # Errors
    ///
    /// Returns the seam's [`OpenError`] if a committed design ever fails to import
    /// (which a unit test guards against, so no user can observe it); the app routes
    /// any such error to its notification surface.
    pub fn open(self) -> Result<OpenOutcome, OpenError> {
        open_document_bytes(self.bytes(), self.format())
    }
}

/// One notable feature of a gallery design, listed in the card's landmarks
/// dropdown so a first-time visitor knows what they are looking at (catalog 96).
#[derive(Clone, Copy, Debug)]
pub struct Landmark {
    /// The landmark's short name (a cell, a region, a structure).
    pub name: &'static str,
    /// A one-line "what it is" for the dropdown row.
    pub detail: &'static str,
}

/// How a [`GalleryCard`]'s primary action opens its design.
#[derive(Clone, Copy, Debug)]
pub enum GalleryAction {
    /// Open a compiled-in [`ExampleChip`] through the document-open seam.
    Example(ExampleChip),
    /// Open a served `.rtla` archive by URL through the streaming `?archive=` path
    /// (web only). The string is the archive URL.
    Archive(&'static str),
}

/// A Start-screen gallery card (catalog 14/96): a real design with its name,
/// technology, size, source, license, an optional streaming badge, and a landmarks
/// dropdown, plus how its primary action opens it.
#[derive(Clone, Copy, Debug)]
pub struct GalleryCard {
    /// The card title.
    pub title: &'static str,
    /// A one-line description under the title.
    pub description: &'static str,
    /// The process/technology badge (for example `SkyWater SKY130`).
    pub technology: &'static str,
    /// A human size label (cells or approximate byte size).
    pub size: &'static str,
    /// The design's source/provenance.
    pub source: &'static str,
    /// The redistribution license.
    pub license: &'static str,
    /// Whether this design streams from a served archive (shows a Streaming badge).
    pub streaming: bool,
    /// The notable landmarks listed in the card's dropdown.
    pub landmarks: &'static [Landmark],
    /// How the card's primary action opens the design.
    pub action: GalleryAction,
}

const TT_LANDMARKS: &[Landmark] = &[
    Landmark {
        name: "TT_MIN_TOP",
        detail: "the synthesized top cell tying the standard cells together",
    },
    Landmark {
        name: "SkyWater standard cells",
        detail: "a few real published SKY130 cells from a Tiny Tapeout 03 design",
    },
];

const INV_LANDMARKS: &[Landmark] = &[
    Landmark {
        name: "sky130_fd_sc_hd__inv_1",
        detail: "the single inverter standard cell, straight from its GDSII",
    },
    Landmark {
        name: "diffusion, poly, and metal1",
        detail: "the transistor layers you can toggle in the Layers panel",
    },
];

const STREAMED_LANDMARKS: &[Landmark] = &[
    Landmark {
        name: "progressive residency",
        detail: "coarse cell boxes fill in to full geometry as tiles stream over HTTP",
    },
    Landmark {
        name: "streaming HUD",
        detail: "the top-left overlay reports fetched tiles and residency live",
    },
];

/// The URL of the served `.rtla` demo archive wired into the streaming gallery card
/// (the same fixture the served-archive e2e streams).
pub const DEMO_ARCHIVE_URL: &str =
    "https://reticle-archive.josefdean.workers.dev/f04af90fbb06786c.rtla";

/// The Start-screen gallery, in display order: the two compiled-in examples that
/// open with one click on any build, then the streaming served-archive demo that
/// shows the browser's `?archive=` residency path (catalog 14/96).
pub const GALLERY: &[GalleryCard] = &[
    GalleryCard {
        title: "Tiny Tapeout sample",
        description: "A real Tiny Tapeout 03 design: a few SkyWater standard cells under a small top.",
        technology: "SkyWater SKY130",
        size: "4 cells",
        source: "Tiny Tapeout 03",
        license: "Apache-2.0",
        streaming: false,
        landmarks: TT_LANDMARKS,
        action: GalleryAction::Example(ExampleChip::TinyTapeoutMin),
    },
    GalleryCard {
        title: "SKY130 inverter cell",
        description: "A single real SkyWater SKY130 standard cell, the inv_1 inverter.",
        technology: "SkyWater SKY130",
        size: "1 cell",
        source: "SkyWater SKY130 PDK",
        license: "Apache-2.0",
        streaming: false,
        landmarks: INV_LANDMARKS,
        action: GalleryAction::Example(ExampleChip::Sky130Inverter),
    },
    GalleryCard {
        title: "Streamed die (served archive)",
        description: "A larger die streamed tile by tile over HTTP Range, showing progressive residency.",
        technology: "SkyWater SKY130",
        size: "streamed",
        source: "Reticle demo archive",
        license: "Apache-2.0",
        streaming: true,
        landmarks: STREAMED_LANDMARKS,
        action: GalleryAction::Archive(DEMO_ARCHIVE_URL),
    },
];

// The recent-files list is displayed by the Start screen but its entry type and
// persistence live in `crate::webopen` (Lane 1B): `webopen::RecentFile` carries the
// name, byte size, and optional source URL, and `RecentFiles` owns dedup, cap, and
// IndexedDB persistence. The Start screen renders `App::recent_files()` directly.

/// The stable identity a [`RecentFile`](crate::webopen::RecentFile) is pinned by:
/// its source URL when it has one, else its display name (matching the dedup
/// identity Lane 1B's `RecentFiles` uses, so a pin follows the same entry).
#[must_use]
pub fn recent_key(recent: &crate::webopen::RecentFile) -> &str {
    recent.url.as_deref().unwrap_or(&recent.name)
}

/// Lane 2D's pinning state for the Start-screen recent-files list (catalog 9).
///
/// The recent-file *model* (dedup, cap, persistence) is frozen Lane 1B code, so
/// pinning is a sibling concern kept here: a set of pinned [`recent_key`]s and the
/// pure ordering that floats pinned entries to the top of the list, each group
/// keeping the recency order Lane 1B produced.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct RecentPins {
    pinned: std::collections::BTreeSet<String>,
}

impl RecentPins {
    /// An empty pin set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `key` (from [`recent_key`]) is pinned.
    #[must_use]
    pub fn is_pinned(&self, key: &str) -> bool {
        self.pinned.contains(key)
    }

    /// Toggles the pin for `key`, returning the new pinned state.
    pub fn toggle(&mut self, key: impl Into<String>) -> bool {
        let key = key.into();
        if self.pinned.remove(&key) {
            false
        } else {
            self.pinned.insert(key);
            true
        }
    }

    /// Orders `recent` for display: pinned entries first, then the rest, each group
    /// preserving the input (most-recent-first) order.
    #[must_use]
    pub fn order<'a>(
        &self,
        recent: &'a [crate::webopen::RecentFile],
    ) -> Vec<&'a crate::webopen::RecentFile> {
        let (mut pinned, mut rest): (Vec<_>, Vec<_>) =
            recent.iter().partition(|r| self.is_pinned(recent_key(r)));
        pinned.append(&mut rest);
        pinned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_chips_are_enumerable_with_distinct_titles() {
        assert_eq!(ExampleChip::ALL.len(), 2);
        let mut titles: Vec<&str> = ExampleChip::ALL.iter().map(|c| c.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), 2, "titles are distinct");
        for chip in ExampleChip::ALL {
            assert!(!chip.title().is_empty());
            assert!(!chip.description().is_empty());
            assert!(!chip.attribution().is_empty());
            // Style gate: no em dash in any gallery copy.
            assert!(!chip.title().contains('\u{2014}'));
            assert!(!chip.description().contains('\u{2014}'));
            assert!(!chip.attribution().contains('\u{2014}'));
            // The bytes are actually compiled in (non-empty).
            assert!(!chip.bytes().is_empty(), "{chip:?} has embedded bytes");
        }
    }

    #[test]
    fn every_bundled_chip_opens_cleanly_through_the_seam() {
        // The core promise: each embedded design opens through the hardened seam and
        // yields a usable document with a real top cell. This is the "bytes to opened
        // document via the seam" path the gallery button drives.
        for chip in ExampleChip::ALL {
            let outcome = chip
                .open()
                .unwrap_or_else(|e| panic!("{chip:?} must open: {e}"));
            assert!(
                !outcome.top_cell.is_empty(),
                "{chip:?} frames a named top cell"
            );
            assert!(
                outcome.document.cell(&outcome.top_cell).is_some(),
                "{chip:?} top cell {} is present",
                outcome.top_cell
            );
            assert!(
                outcome.document.cell_count() >= 1,
                "{chip:?} has at least one cell"
            );
        }
    }

    #[test]
    fn the_tinytapeout_sample_is_the_multi_cell_real_chip() {
        // The lead example is the real, multi-cell Tiny Tapeout design (four cells,
        // no warnings), framing its synthesized top.
        let outcome = ExampleChip::TinyTapeoutMin.open().expect("opens");
        assert_eq!(outcome.top_cell, "TT_MIN_TOP");
        assert_eq!(outcome.document.cell_count(), 4);
        assert!(
            !outcome.has_warnings(),
            "the committed sample is a clean import"
        );
    }

    #[test]
    fn the_inverter_example_is_a_single_real_standard_cell() {
        let outcome = ExampleChip::Sky130Inverter.open().expect("opens");
        assert_eq!(outcome.top_cell, "sky130_fd_sc_hd__inv_1");
        let cell = outcome.document.cell(&outcome.top_cell).expect("top cell");
        assert!(!cell.shapes.is_empty(), "the inverter has geometry");
    }

    #[test]
    fn all_bundled_examples_are_gds_today() {
        for chip in ExampleChip::ALL {
            assert_eq!(chip.format(), DocFormat::Gds);
        }
    }

    #[test]
    fn from_e2e_id_maps_the_examples_guard_ids() {
        assert_eq!(
            ExampleChip::from_e2e_id("tt03"),
            Some(ExampleChip::TinyTapeoutMin)
        );
        assert_eq!(
            ExampleChip::from_e2e_id("TinyTapeout"),
            Some(ExampleChip::TinyTapeoutMin)
        );
        assert_eq!(
            ExampleChip::from_e2e_id("sky130"),
            Some(ExampleChip::Sky130Inverter)
        );
        assert_eq!(ExampleChip::from_e2e_id("nope"), None);
        assert_eq!(ExampleChip::from_e2e_id(""), None);
    }

    #[test]
    fn gallery_cards_carry_full_metadata_and_landmarks() {
        assert!(
            !GALLERY.is_empty(),
            "the gallery offers at least one design"
        );
        for card in GALLERY {
            for field in [
                card.title,
                card.description,
                card.technology,
                card.size,
                card.source,
                card.license,
            ] {
                assert!(!field.is_empty(), "{} has a blank field", card.title);
                // Style gate: no em dash in any gallery copy.
                assert!(!field.contains('\u{2014}'), "{} has an em dash", card.title);
            }
            assert!(
                !card.landmarks.is_empty(),
                "{} lists at least one landmark (catalog 96)",
                card.title
            );
            for lm in card.landmarks {
                assert!(!lm.name.is_empty() && !lm.detail.is_empty());
                assert!(!lm.detail.contains('\u{2014}'));
            }
        }
        // Exactly one streaming card, and it opens an archive URL, not an example.
        let streaming: Vec<&GalleryCard> = GALLERY.iter().filter(|c| c.streaming).collect();
        assert_eq!(streaming.len(), 1, "one streaming demo is offered");
        assert!(
            matches!(streaming[0].action, GalleryAction::Archive(_)),
            "a streaming card opens an archive"
        );
    }

    #[test]
    fn every_embedded_gallery_card_opens_through_the_seam() {
        // Each non-streaming card names an ExampleChip that opens cleanly.
        for card in GALLERY.iter().filter(|c| !c.streaming) {
            let GalleryAction::Example(chip) = card.action else {
                panic!(
                    "{} is not streaming, so it must be an embedded example",
                    card.title
                );
            };
            chip.open()
                .unwrap_or_else(|e| panic!("{} must open: {e}", card.title));
        }
    }

    #[test]
    fn recent_pins_toggle_and_float_pinned_entries_first() {
        use crate::webopen::RecentFile;
        let recent = [
            RecentFile::local("newest.gds", 2048),
            RecentFile::remote("mid.gds", 1024, "https://example.test/mid.gds"),
            RecentFile::local("oldest.gds", 512),
        ];
        let mut pins = RecentPins::new();
        // Nothing pinned: order is unchanged (most-recent-first).
        let order: Vec<&str> = pins
            .order(&recent)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(order, ["newest.gds", "mid.gds", "oldest.gds"]);

        // Pin the oldest (by key): it floats to the top; the rest keep their order.
        assert!(pins.toggle(recent_key(&recent[2])), "toggling pins it");
        assert!(pins.is_pinned("oldest.gds"));
        let order: Vec<&str> = pins
            .order(&recent)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(order, ["oldest.gds", "newest.gds", "mid.gds"]);

        // The remote entry pins by its URL key, and toggling again unpins.
        let mid_key = recent_key(&recent[1]).to_owned();
        assert!(pins.toggle(&mid_key));
        assert!(pins.is_pinned("https://example.test/mid.gds"));
        assert!(!pins.toggle(&mid_key), "toggling again unpins");
        assert!(!pins.is_pinned("https://example.test/mid.gds"));
    }
}

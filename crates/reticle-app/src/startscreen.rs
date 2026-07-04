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

/// One entry in the Start screen's recent-files list.
///
/// The Start screen only *displays* this; the persistence behind it (`IndexedDB` on
/// the web, and any native store) is a separate concern that feeds the app's
/// `recent_files` list. Keeping the shape here, minimal and owned, lets the Start
/// screen render a recent list before any persistence backend exists, and lets that
/// backend fill the same shape later without changing the rendering.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentFile {
    /// The display name of the file (typically its base name, for example
    /// `adder.gds`).
    pub name: String,
    /// The format the file was opened as, so the row can show a `GDSII`/`OASIS` tag.
    pub format: DocFormat,
}

impl RecentFile {
    /// A recent-file entry with a display name and its format.
    #[must_use]
    pub fn new(name: impl Into<String>, format: DocFormat) -> Self {
        Self {
            name: name.into(),
            format,
        }
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
    fn recent_file_carries_a_name_and_format() {
        let r = RecentFile::new("adder.gds", DocFormat::Gds);
        assert_eq!(r.name, "adder.gds");
        assert_eq!(r.format, DocFormat::Gds);
        assert_eq!(r.format.label(), "GDSII");
    }
}

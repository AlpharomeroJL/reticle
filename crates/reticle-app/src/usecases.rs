//! The four bundled worked use-case scenarios offered from the Start screen.
//!
//! Reticle ships several deep capabilities (layer inspection and a 3D stack, the
//! DRC engine, the agent replay theater, and the draw/boolean/array/via-stack
//! editing tools), but a first-time visitor does not know where to begin. This
//! module packages four *worked scenarios* that each drop the user straight into a
//! prepared starting point for one capability, so the value is one click away
//! rather than behind a blank document.
//!
//! Each scenario is described by a [`UseCase`] variant carrying a
//! [`title`](UseCase::title) and a [`description`](UseCase::description), and it
//! [`prepare`](UseCase::prepare)s a [`Scenario`] telling the app what to do:
//! either load a specific starting document (with its top cell) or open the replay
//! theater. Everything here is deliberately *model-free glue* over the frozen
//! `reticle-model`/`reticle-io`/`reticle-drc` types, in the spirit of [`crate::demo`],
//! so the interesting behavior is unit-tested without a window or a GPU.
//!
//! # Scenarios
//!
//! * [`UseCase::InspectCell`] loads a real `SkyWater` SKY130 standard cell (the
//!   `inv_1` inverter) from a bundled GDSII stream and grafts the committed SKY130
//!   technology onto it, so its layers are named and colored and the 3D stack has
//!   real elevations to extrude. See [`inspect_document`].
//! * [`UseCase::FindAndFixViolation`] builds an in-code document that *deliberately*
//!   violates the SKY130 `m1.1` minimum-width rule (a met1 wire narrower than
//!   140 nm), with the SKY130 rule subset carried in its technology so a DRC run
//!   flags it. See [`violation_document`].
//! * [`UseCase::WatchTheAgent`] opens the replay theater on the bundled scripted
//!   run (see [`crate::store`]); it prepares no document because the theater drives
//!   its own model-free session.
//! * [`UseCase::BuildWithTools`] loads a small starter document with a couple of
//!   labeled shapes on the SKY130 metal layers, a blank canvas of sorts for trying
//!   draw, boolean, array, and via-stack. See [`starter_document`].
//!
//! # Portability
//!
//! The SKY130 cell and technology are compiled into the binary
//! ([`include_bytes!`]/[`include_str!`]), never read from disk, so every scenario
//! builds identically on native and on `wasm32` where there is no filesystem. This
//! mirrors the bundled-transcript pattern in [`crate::store`].

use reticle_geometry::{Endcap, LayerId, Path, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, Importer, ShapeKind, Technology};

/// The `SkyWater` SKY130 inverter (`inv_1`) GDSII stream, compiled in so the
/// inspect scenario loads a real production cell with no filesystem (see the
/// module docs and `assets/NOTICE.md` for attribution and licensing).
const SKY130_INV_GDS: &[u8] = include_bytes!("../assets/sky130_fd_sc_hd__inv_1.gds");

/// The committed SKY130 technology file, compiled in so the imported cell can be
/// given named, colored layers and a real physical stack for the 3D view. This is
/// the same file the CLI and cross-section tests parse; embedding it keeps the wasm
/// build free of any runtime path, exactly as `reticle-drc` embeds its rule table.
const SKY130_TECH: &str = include_str!("../../../tech/sky130.tech");

/// The top-cell name of the bundled SKY130 inverter, as it appears in the stream.
const SKY130_INV_TOP: &str = "sky130_fd_sc_hd__inv_1";

/// The SKY130 met1 drawing layer (`68/20`), used by the seeded-violation and
/// starter documents.
const MET1: LayerId = LayerId::new(68, 20);

/// The SKY130 met2 drawing layer (`69/20`), used by the starter document so a
/// via-stack has a second metal to reach.
const MET2: LayerId = LayerId::new(69, 20);

/// One of the four bundled worked use-case scenarios.
///
/// The variant order is the order the Start screen offers them. Every scenario is
/// enumerable through [`UseCase::ALL`] and prepared through [`UseCase::prepare`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum UseCase {
    /// Inspect a SKY130 standard cell: load the `inv_1` inverter with named layers,
    /// measurement, and a 3D stack to explore.
    InspectCell,
    /// Find and fix a violation: a document seeded with a met1 minimum-width
    /// violation to run DRC on, edit, and re-check clean.
    FindAndFixViolation,
    /// Watch the agent work: open the replay theater and play the bundled scripted
    /// run with its narration.
    WatchTheAgent,
    /// Build with the new tools: a small starter document for trying draw, boolean,
    /// array, and via-stack, guided.
    BuildWithTools,
}

impl UseCase {
    /// Every scenario, in the order the Start screen offers them.
    pub const ALL: [UseCase; 4] = [
        UseCase::InspectCell,
        UseCase::FindAndFixViolation,
        UseCase::WatchTheAgent,
        UseCase::BuildWithTools,
    ];

    /// A short title for the chooser button.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            UseCase::InspectCell => "Inspect a SKY130 cell",
            UseCase::FindAndFixViolation => "Find and fix a violation",
            UseCase::WatchTheAgent => "Watch the agent work",
            UseCase::BuildWithTools => "Build with the new tools",
        }
    }

    /// A one-line description of what the scenario sets up and invites the user to
    /// do, shown under the title in the chooser.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            UseCase::InspectCell => {
                "Load a real SkyWater SKY130 inverter. Toggle layers, measure \
                 features, and open the 3D stack to see the metals extruded."
            }
            UseCase::FindAndFixViolation => {
                "Open a layout with a seeded design-rule error. Run DRC to find the \
                 too-narrow met1 wire, widen it, and re-check until it clears."
            }
            UseCase::WatchTheAgent => {
                "Open the replay theater and watch a recorded agent run draw a clean \
                 met1 wire, with step, play, and speed controls and live narration."
            }
            UseCase::BuildWithTools => {
                "Start from a small seed layout and try the new editing tools: draw, \
                 boolean, array, and via-stack, on the SKY130 metal layers."
            }
        }
    }

    /// Prepares the scenario: what the app should do to enter it.
    ///
    /// Document-backed scenarios return [`Scenario::LoadDocument`] with the starting
    /// document and its top cell; the agent scenario returns
    /// [`Scenario::OpenReplayTheater`].
    #[must_use]
    pub fn prepare(self) -> Scenario {
        match self {
            UseCase::InspectCell => Scenario::LoadDocument {
                document: inspect_document(),
                top_cell: SKY130_INV_TOP.to_owned(),
            },
            UseCase::FindAndFixViolation => Scenario::LoadDocument {
                document: violation_document(),
                top_cell: VIOLATION_TOP.to_owned(),
            },
            UseCase::WatchTheAgent => Scenario::OpenReplayTheater,
            UseCase::BuildWithTools => Scenario::LoadDocument {
                document: starter_document(),
                top_cell: STARTER_TOP.to_owned(),
            },
        }
    }
}

/// What the app must do to enter a chosen [`UseCase`].
///
/// The app consumes this once, right after the user picks a scenario: it either
/// installs the given document as the live layout (replacing the demo) or opens the
/// replay theater. Keeping this an explicit value (rather than the scenario mutating
/// the app directly) is what lets the whole module stay window-free and unit-tested.
#[derive(Clone, Debug)]
pub enum Scenario {
    /// Load `document` as the live editing layout, framing `top_cell`.
    LoadDocument {
        /// The prepared starting document.
        document: Document,
        /// The top cell to display and check.
        top_cell: String,
    },
    /// Open the replay theater on the bundled scripted run; do not touch the
    /// document.
    OpenReplayTheater,
}

/// The top-cell name of the seeded-violation document.
const VIOLATION_TOP: &str = "DRC_DEMO";

/// The top-cell name of the build-with-tools starter document.
const STARTER_TOP: &str = "SANDBOX";

/// The bundled SKY130 inverter, imported and given the committed SKY130 technology.
///
/// The GDSII stream carries geometry but only a synthesized, unnamed layer table
/// (GDSII has no layer names). Grafting [`sky130_technology`] onto it names and
/// colors the layers, adds the physical stack the 3D view extrudes, and carries the
/// SKY130 DRC rule subset so a check on the real cell resolves the periphery rules.
///
/// # Panics
///
/// Panics only if the compiled-in GDSII asset fails to import, which can happen
/// solely if the committed file is corrupt; a unit test guards against that so no
/// caller can observe it.
#[must_use]
pub fn inspect_document() -> Document {
    let mut doc = reticle_io::Gds
        .import(SKY130_INV_GDS)
        .expect("bundled SKY130 inverter GDS must import");
    doc.set_technology(sky130_technology());
    doc
}

/// The committed SKY130 technology (named, colored layers plus the physical stack),
/// with the SKY130 DRC rule subset attached.
///
/// The layer table and stack come from the embedded `tech/sky130.tech`; the rules
/// come from [`reticle_drc::sky130_drc_rules`] (which embeds `tech/sky130-drc-subset.toml`).
/// The tech file itself declares no rules, so attaching them here is what makes a
/// DRC run over the inspect or violation scenarios resolve the real periphery rules
/// rather than the fallback width deck.
///
/// # Panics
///
/// Panics only if the compiled-in `tech/sky130.tech` fails to parse, which can
/// happen solely if the committed file is malformed; a unit test guards against
/// that.
#[must_use]
pub fn sky130_technology() -> Technology {
    let mut tech =
        reticle_io::parse_technology(SKY130_TECH).expect("bundled tech/sky130.tech must parse");
    tech.rules = reticle_drc::sky130_drc_rules();
    tech
}

/// A small document that deliberately violates the SKY130 `m1.1` minimum met1
/// width rule, for the find-and-fix-a-violation scenario.
///
/// The top cell holds two met1 wires: one comfortably wider than the 140 nm
/// minimum (so not everything is broken) and one only 80 nm wide, which violates
/// `m1.1`. The SKY130 rule subset is carried in the technology, so a DRC run
/// resolves the real periphery rules and reports the narrow wire; widening it to at
/// least 140 nm and re-running clears the check.
#[must_use]
pub fn violation_document() -> Document {
    let mut cell = Cell::new(VIOLATION_TOP);

    // A compliant met1 wire: 200 nm wide, well over the 0.14 um minimum. Present so
    // the scenario is a realistic layout with a good wire next to the bad one.
    cell.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(2000, 200))),
    ));

    // The seeded violation: an 80 nm-wide met1 wire, under the m1.1 minimum of
    // 140 nm. The DRC engine flags the short (width) dimension of this rectangle.
    cell.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Rect(Rect::new(Point::new(0, 600), Point::new(2000, 680))),
    ));

    let mut doc = Document::new();
    doc.set_technology(sky130_technology());
    doc.insert_cell(cell);
    doc.set_top_cells(vec![VIOLATION_TOP.to_owned()]);
    doc
}

/// A small starter document for the build-with-tools scenario.
///
/// It is deliberately sparse: two short met1 wires with room between them, so the
/// user has real geometry to select and something to draw next to, array, union
/// with a boolean, or bridge to met2 with a via stack. It carries the SKY130
/// technology so the metal layers are named and a via-stack targets real layers.
#[must_use]
pub fn starter_document() -> Document {
    let mut cell = Cell::new(STARTER_TOP);

    // Two parallel met1 wires, 200 nm wide, spaced apart. A boolean union or an
    // array has obvious inputs; drawing a met2 strap over them plus a via stack has
    // a clear target.
    cell.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(1600, 0)],
            200,
            Endcap::Square,
        )),
    ));
    cell.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 1000), Point::new(1600, 1000)],
            200,
            Endcap::Square,
        )),
    ));
    // A single met2 landing pad, so a via stack from met1 to met2 has a place to go.
    cell.shapes.push(DrawShape::new(
        MET2,
        ShapeKind::Rect(Rect::new(Point::new(700, 400), Point::new(900, 600))),
    ));

    let mut doc = Document::new();
    doc.set_technology(sky130_technology());
    doc.insert_cell(cell);
    doc.set_top_cells(vec![STARTER_TOP.to_owned()]);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drc_panel::DrcResults;
    use reticle_model::RuleKind;

    /// The SKY130 `m1.1` minimum met1 width, in DBU (1 dbu = 1 nm), i.e. 0.14 um.
    /// The seeded-violation wire is narrower than this; the fix widens it to meet it.
    const M1_MIN_WIDTH_DBU: i64 = 140;

    #[test]
    fn all_four_are_enumerable_and_unique() {
        assert_eq!(UseCase::ALL.len(), 4);
        // Titles and descriptions are all present, non-empty, and distinct.
        let mut titles: Vec<&str> = UseCase::ALL.iter().map(|c| c.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), 4, "titles must be distinct");
        for uc in UseCase::ALL {
            assert!(!uc.title().is_empty(), "{uc:?} has a title");
            assert!(!uc.description().is_empty(), "{uc:?} has a description");
            // No em dash slips into the copy (the style gate forbids U+2014).
            assert!(!uc.title().contains('\u{2014}'));
            assert!(!uc.description().contains('\u{2014}'));
        }
    }

    #[test]
    fn every_scenario_prepares_something_valid() {
        for uc in UseCase::ALL {
            match uc.prepare() {
                Scenario::LoadDocument { document, top_cell } => {
                    // The named top cell exists and is registered as a top.
                    assert!(
                        document.cell(&top_cell).is_some(),
                        "{uc:?}: top cell {top_cell} present"
                    );
                    assert!(
                        document.top_cells().contains(&top_cell),
                        "{uc:?}: {top_cell} is a top cell"
                    );
                    // A real technology with a positive resolution came along.
                    assert!(
                        document.technology().dbu_per_micron > 0,
                        "{uc:?}: has a database resolution"
                    );
                }
                Scenario::OpenReplayTheater => {
                    assert_eq!(uc, UseCase::WatchTheAgent);
                }
            }
        }
    }

    #[test]
    fn only_the_agent_scenario_opens_the_theater() {
        let theater: Vec<UseCase> = UseCase::ALL
            .into_iter()
            .filter(|uc| matches!(uc.prepare(), Scenario::OpenReplayTheater))
            .collect();
        assert_eq!(theater, vec![UseCase::WatchTheAgent]);
    }

    #[test]
    fn inspect_cell_loads_the_real_inverter_with_named_layers() {
        let doc = inspect_document();
        // The bundled inverter imports as its upstream top cell at 1 nm units.
        assert_eq!(doc.top_cells(), &[SKY130_INV_TOP.to_owned()]);
        let cell = doc.cell(SKY130_INV_TOP).expect("inverter top cell");
        assert!(!cell.shapes.is_empty(), "the inverter has geometry");
        assert!(!cell.labels.is_empty(), "the inverter has pin labels");
        assert_eq!(doc.technology().dbu_per_micron, 1000);

        // The grafted SKY130 technology names and colors met1 (not the "L68D20"
        // placeholder the bare GDS import would synthesize).
        let met1 = doc
            .technology()
            .layers
            .iter()
            .find(|l| l.id == MET1)
            .expect("met1 in the SKY130 layer table");
        assert_eq!(met1.name, "met1");
        // The physical stack came along so the 3D view has elevations to extrude.
        assert!(
            doc.technology().stack_for(MET1).is_some(),
            "met1 has a stack entry for the 3D view"
        );
    }

    #[test]
    fn violation_document_actually_reports_a_met1_width_violation() {
        let doc = violation_document();
        let mut drc = DrcResults::new();
        let n = drc.run(&doc, VIOLATION_TOP);
        assert!(n >= 1, "the seeded narrow met1 wire must violate DRC");

        // At least one violation is the SKY130 m1.1 width rule on met1, and the
        // measured width is under the required 140 nm minimum.
        let m1 = drc
            .violations()
            .iter()
            .find(|v| v.layer == MET1 && v.kind == RuleKind::Width)
            .expect("a met1 width violation");
        assert_eq!(m1.rule, "m1.1", "the SKY130 min-width rule id");
        assert_eq!(m1.required, M1_MIN_WIDTH_DBU);
        assert!(
            m1.measured < M1_MIN_WIDTH_DBU,
            "measured {} should be under {M1_MIN_WIDTH_DBU}",
            m1.measured
        );
    }

    #[test]
    fn violation_document_clears_when_the_wire_is_widened() {
        // Rebuild the same document but with the narrow wire widened to the minimum,
        // proving the scenario has a reachable clean state (what the user does by
        // hand: widen the wire and re-run).
        let mut cell = Cell::new(VIOLATION_TOP);
        cell.shapes.push(DrawShape::new(
            MET1,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(2000, 200))),
        ));
        // Widened: 140 nm tall now, meeting m1.1 exactly.
        cell.shapes.push(DrawShape::new(
            MET1,
            ShapeKind::Rect(Rect::new(Point::new(0, 600), Point::new(2000, 740))),
        ));
        let mut doc = Document::new();
        doc.set_technology(sky130_technology());
        doc.insert_cell(cell);
        doc.set_top_cells(vec![VIOLATION_TOP.to_owned()]);

        let mut drc = DrcResults::new();
        let n = drc.run(&doc, VIOLATION_TOP);
        // No met1 width violation remains once both wires meet the minimum.
        assert!(
            !drc.violations()
                .iter()
                .any(|v| v.layer == MET1 && v.kind == RuleKind::Width),
            "widened met1 wires should clear m1.1 (got {n} total violations)"
        );
    }

    #[test]
    fn starter_document_is_a_small_valid_seed() {
        let doc = starter_document();
        let cell = doc.cell(STARTER_TOP).expect("sandbox top cell");
        // A handful of shapes on the real metal layers, and a nonempty bbox so the
        // camera can frame it.
        assert!(
            (2..=8).contains(&cell.shapes.len()),
            "a small seed, not empty and not a pile"
        );
        assert!(cell.shapes.iter().any(|s| s.layer == MET1));
        assert!(cell.shapes.iter().any(|s| s.layer == MET2));
        let bbox = doc.cell_bbox(STARTER_TOP).expect("sandbox has a bbox");
        assert!(bbox.width() > 0 && bbox.height() > 0);
    }

    #[test]
    fn sky130_technology_carries_the_real_rule_subset() {
        let tech = sky130_technology();
        // The committed subset has 26 rules; m1.1 (met1 min width, 140) is among
        // them, which is what the violation scenario depends on.
        assert_eq!(tech.rules.len(), 26);
        let m1_1 = tech
            .rules
            .iter()
            .find(|r| r.name == "m1.1")
            .expect("m1.1 present");
        assert_eq!(m1_1.layer, MET1);
        assert_eq!(m1_1.value, M1_MIN_WIDTH_DBU);
    }
}

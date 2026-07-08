//! The LEF/DEF import oracle, proven both ways against a real EDA tool.
//!
//! `reticle-lefdef` imports a LEF/DEF pair; `OpenROAD`, running in the pinned
//! `hpretl/iic-osic-tools` container, reads the same two files. This test asserts the two
//! agree on the structural facts (macros, components, pins, die area), and that a
//! deliberately corrupted DEF (one component dropped) makes them DISAGREE, which proves
//! the oracle actually discriminates a faithful import from a corrupted one.
//!
//! Two layers of proof, mirroring the Tiny Tapeout precheck oracle (ADR 0054, 0083):
//!
//! - The parser-level tests always run in the ordinary gate with no Docker: they parse the
//!   committed `oracle_faithful.txt` / `oracle_corrupt.txt` (real `OpenROAD` output
//!   captured from the pinned image) and check the faithful counts agree with the import
//!   while the corrupted counts diverge.
//! - The live container test runs `OpenROAD` over the same files when Docker and the
//!   pinned image are present, and skips honestly (never fails) otherwise.

use reticle_cli::lefdef_oracle::{OracleCounts, OracleOutcome, parse_oracle_output, run_oracle};
use reticle_lefdef::{LefDefDesign, import_lef_def};

const FAITHFUL_LEF: &[u8] = include_bytes!("fixtures/lefdef-oracle/faithful.lef");
const FAITHFUL_DEF: &[u8] = include_bytes!("fixtures/lefdef-oracle/faithful.def");
const CORRUPT_DEF: &[u8] = include_bytes!("fixtures/lefdef-oracle/corrupt.def");
const ORACLE_FAITHFUL: &str = include_str!("fixtures/lefdef-oracle/oracle_faithful.txt");
const ORACLE_CORRUPT: &str = include_str!("fixtures/lefdef-oracle/oracle_corrupt.txt");

/// Both sides read DEF database units directly, so the die area is compared exactly.
const DIE_TOLERANCE_DBU: i64 = 0;

/// The structural facts of an imported design, in the same shape the oracle reports.
///
/// The macro count is every cell other than the top design cell (each LEF `MACRO` lowers
/// to a cell); components are the top cell's placed instances; pins are the external DEF
/// pins; the die area is the `DIEAREA` box in database units.
fn counts_from_design(design: &LefDefDesign) -> OracleCounts {
    let top = design.top_cell();
    let macros = design.document.cells().filter(|c| c.name != top).count();
    let components = design.document.cell(top).map_or(0, |c| c.instances.len());
    let die_area = design.die_area.map(|r| {
        [
            i64::from(r.min.x),
            i64::from(r.min.y),
            i64::from(r.max.x),
            i64::from(r.max.y),
        ]
    });
    OracleCounts {
        macros: Some(macros),
        components,
        pins: design.pins.len(),
        die_area,
    }
}

/// The faithful fixture imports to the ground-truth counts the oracle is compared against:
/// two macros, three components, two pins, and a `20000 x 20000` DBU die.
#[test]
fn faithful_import_has_the_expected_counts() {
    let design = import_lef_def(FAITHFUL_LEF, FAITHFUL_DEF).expect("faithful import");
    let counts = counts_from_design(&design);
    assert_eq!(counts.macros, Some(2));
    assert_eq!(counts.components, 3);
    assert_eq!(counts.pins, 2);
    assert_eq!(counts.die_area, Some([0, 0, 20_000, 20_000]));
}

/// Parser-level two-way, always run (no Docker): the committed faithful oracle output
/// agrees with the import, and the corrupted oracle output diverges by exactly the dropped
/// component. This proves the discrimination deterministically in the ordinary gate.
#[test]
fn committed_oracle_output_agrees_and_corrupt_diverges() {
    let design = import_lef_def(FAITHFUL_LEF, FAITHFUL_DEF).expect("faithful import");
    let imported = counts_from_design(&design);

    let oracle_ok = parse_oracle_output(ORACLE_FAITHFUL);
    assert!(
        imported.agrees_with(&oracle_ok, DIE_TOLERANCE_DBU),
        "faithful import must agree with the oracle: {imported:?} vs {oracle_ok:?}"
    );

    let oracle_bad = parse_oracle_output(ORACLE_CORRUPT);
    assert!(
        !imported.agrees_with(&oracle_bad, DIE_TOLERANCE_DBU),
        "a corrupted DEF must diverge: {imported:?} vs {oracle_bad:?}"
    );
    // The divergence is precisely the one dropped component.
    assert_eq!(oracle_ok.components, 3);
    assert_eq!(oracle_bad.components, 2);
    assert_eq!(
        oracle_bad.macros, oracle_ok.macros,
        "only components changed"
    );
}

/// Live container cross-check, adapter-gated: runs `OpenROAD` over the same LEF/DEF when
/// Docker and the pinned image are present, and skips honestly otherwise. When it runs, a
/// faithful import must agree with the oracle and a corrupted DEF must diverge.
#[test]
fn container_cross_check_runs_or_skips_honestly() {
    let design = import_lef_def(FAITHFUL_LEF, FAITHFUL_DEF).expect("faithful import");
    let imported = counts_from_design(&design);

    match run_oracle(FAITHFUL_LEF, FAITHFUL_DEF).expect("run oracle over faithful") {
        OracleOutcome::Skipped(reason) => {
            eprintln!("lefdef oracle cross-check SKIPPED (honest not-run): {reason}");
        }
        OracleOutcome::Ran(counts) => {
            assert!(
                imported.agrees_with(&counts, DIE_TOLERANCE_DBU),
                "faithful import must agree with the live oracle: {imported:?} vs {counts:?}"
            );
            // Two-way: the same oracle over the corrupted DEF must disagree.
            match run_oracle(FAITHFUL_LEF, CORRUPT_DEF).expect("run oracle over corrupt") {
                OracleOutcome::Ran(bad) => {
                    assert!(
                        !imported.agrees_with(&bad, DIE_TOLERANCE_DBU),
                        "the live oracle must discriminate the corrupted DEF: \
                         {imported:?} vs {bad:?}"
                    );
                    assert_eq!(
                        bad.components,
                        counts.components - 1,
                        "the corrupted DEF drops exactly one component"
                    );
                }
                OracleOutcome::Skipped(reason) => {
                    panic!("faithful ran but corrupt skipped, which cannot happen: {reason}")
                }
            }
        }
    }
}

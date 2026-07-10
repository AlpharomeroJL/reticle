//! F4 waveform-record contract cross-test.
//!
//! Both the producer (a bounded solver, Phase 3) and the consumer (the waveform UI) build
//! against `tests/fixtures/contracts/f4_rc_transient.json`. This test pins the fixture:
//! it deserializes to the [`WaveformSet`] schema, checks the contract invariant, confirms
//! the values ARE a first-order RC charging transient within tolerance (so the fixture
//! has a defined physical meaning, not arbitrary numbers), and round-trips through serde.

use reticle_sim::waveform::{AnalysisKind, Quantity, WaveformSet};

/// The committed fixture: a first-order RC charging transient, `V(t) = 1 - exp(-t / RC)`
/// with `RC = 1 ns` and `V0 = 1 V`, sampled at 0, 0.5, 1, 2, 3 ns on node `n_out`.
const FIXTURE: &str = include_str!("fixtures/contracts/f4_rc_transient.json");

/// The RC time constant the fixture was generated with (1 ns), for the analytic check.
const RC_NS: f64 = 1.0;

#[test]
fn f4_fixture_is_a_well_formed_rc_transient() {
    let set: WaveformSet = serde_json::from_str(FIXTURE).expect("F4 fixture parses");

    // Contract invariant + shape.
    assert!(
        set.is_well_formed(),
        "the fixture must satisfy the F4 invariant"
    );
    assert_eq!(set.analysis, AnalysisKind::Transient);
    assert_eq!(set.time_fs.len(), 5);
    assert_eq!(set.probes.len(), 1);
    let probe = &set.probes[0];
    assert_eq!(probe.quantity, Quantity::Voltage);
    assert_eq!(probe.samples_nano.len(), set.time_fs.len());

    // Physical meaning: each sample matches the analytic RC charging curve within 1 nV,
    // so a consumer can trust the fixture is a real transient, not filler.
    for (t_fs, &sample_nano) in set.time_fs.iter().zip(&probe.samples_nano) {
        let t_ns = *t_fs as f64 / 1.0e6;
        let analytic_v = 1.0 - (-t_ns / RC_NS).exp();
        let recorded_v = sample_nano as f64 / 1.0e9;
        assert!(
            (recorded_v - analytic_v).abs() < 1.0e-9,
            "at t={t_ns} ns: recorded {recorded_v} V vs analytic {analytic_v} V"
        );
    }

    // Bounds match the data extents.
    assert_eq!(set.bounds.t_min_fs, 0);
    assert_eq!(set.bounds.t_max_fs, 3_000_000);
    assert_eq!(set.bounds.y_min_nano, 0);
    assert_eq!(set.bounds.y_max_nano, 950_212_932);

    // Serde round-trips exactly (integer-scaled records are byte-stable).
    let reserialized = serde_json::to_string(&set).expect("serialize");
    let reparsed: WaveformSet = serde_json::from_str(&reserialized).expect("reparse");
    assert_eq!(
        set, reparsed,
        "the record must round-trip through serde unchanged"
    );
}

#[test]
fn f4_operating_point_shape_is_distinct() {
    // An operating point has an empty time axis and one sample per probe; a transient
    // shape (populated axis) must not validate as one, and vice versa. This documents the
    // invariant the UI relies on to pick a rendering.
    let op = WaveformSet {
        analysis: AnalysisKind::OperatingPoint,
        time_fs: vec![],
        probes: vec![reticle_sim::waveform::Probe {
            id: "vdd".to_owned(),
            node: "n_vdd".to_owned(),
            quantity: Quantity::Voltage,
            samples_nano: vec![1_800_000_000],
        }],
        bounds: reticle_sim::waveform::Bounds {
            t_min_fs: 0,
            t_max_fs: 0,
            y_min_nano: 1_800_000_000,
            y_max_nano: 1_800_000_000,
        },
    };
    assert!(op.is_well_formed());

    let mut bad = op.clone();
    bad.time_fs = vec![0]; // an OP must not carry a time axis
    assert!(!bad.is_well_formed());
}

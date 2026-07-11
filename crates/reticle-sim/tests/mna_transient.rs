//! Solver cross-tests: the pure-Rust dense-MNA engine must reproduce the F4
//! `f4_rc_transient.json` fixture exactly, be deterministic, and handle the DC
//! operating point and the other linear elements (inductor, current source).

use reticle_sim::circuit::{Circuit, GROUND};
use reticle_sim::transient::{ProbeSpec, TransientOptions, solve_operating_point, solve_transient};
use reticle_sim::waveform::{AnalysisKind, Quantity, WaveformSet};

/// The committed F4 contract fixture: a first-order RC charging transient.
const FIXTURE: &str = include_str!("fixtures/contracts/f4_rc_transient.json");

/// Build the reference RC circuit: a 1 V step source driving a 1 kOhm resistor into
/// a 1 pF capacitor to ground (`RC = 1 ns`), node `n_out` across the capacitor.
fn rc_circuit() -> (Circuit, usize) {
    let mut circuit = Circuit::new();
    let vin = circuit.node("vin");
    let out = circuit.node("n_out");
    circuit.vsource(vin, GROUND, 1.0);
    circuit.resistor(vin, out, 1_000.0);
    circuit.capacitor(out, GROUND, 1.0e-12);
    (circuit, out)
}

/// Transient options that record at the fixture's sample times with a 20 fs internal
/// step. Trapezoidal error at that step is ~0.01 nV, far under the half-nano-unit
/// rounding boundary, so the quantised curve equals the fixture exactly.
fn rc_options(out: usize) -> TransientOptions {
    TransientOptions {
        dt_fs: 20,
        sample_times_fs: vec![0, 500_000, 1_000_000, 2_000_000, 3_000_000],
        probes: vec![ProbeSpec::voltage("out", out, "n_out")],
    }
}

#[test]
fn rc_transient_reproduces_the_f4_fixture_exactly() {
    let fixture: WaveformSet = serde_json::from_str(FIXTURE).expect("fixture parses");
    let (circuit, out) = rc_circuit();
    let produced = solve_transient(&circuit, &rc_options(out)).expect("transient solves");

    assert!(
        produced.is_well_formed(),
        "solver output must satisfy the F4 invariant"
    );
    assert_eq!(produced.analysis, AnalysisKind::Transient);

    // Primary bar: every sample within 1 nV of the fixture.
    let probe = &produced.probes[0];
    let ref_probe = &fixture.probes[0];
    assert_eq!(probe.samples_nano.len(), ref_probe.samples_nano.len());
    for (&got, &want) in probe.samples_nano.iter().zip(&ref_probe.samples_nano) {
        assert!(
            (got - want).abs() <= 1,
            "sample {got} nV differs from fixture {want} nV by more than 1 nV"
        );
    }

    // Stronger: the whole record is byte-for-byte the committed fixture (axis,
    // probe identity, samples, and bounds all match).
    assert_eq!(
        produced, fixture,
        "solver output must equal the committed F4 fixture record"
    );
}

#[test]
fn transient_is_deterministic_byte_for_byte() {
    let (circuit, out) = rc_circuit();
    let first = solve_transient(&circuit, &rc_options(out)).expect("solve 1");
    let second = solve_transient(&circuit, &rc_options(out)).expect("solve 2");

    assert_eq!(first, second, "same circuit must solve to the same record");
    let a = serde_json::to_vec(&first).expect("serialize 1");
    let b = serde_json::to_vec(&second).expect("serialize 2");
    assert_eq!(a, b, "serialized records must be byte-identical");
}

#[test]
fn dc_operating_point_solves_a_resistor_divider() {
    let mut circuit = Circuit::new();
    let top = circuit.node("top");
    let mid = circuit.node("mid");
    circuit.vsource(top, GROUND, 10.0);
    circuit.resistor(top, mid, 1_000.0);
    circuit.resistor(mid, GROUND, 1_000.0);

    let set = solve_operating_point(&circuit, &[ProbeSpec::voltage("mid", mid, "n_mid")])
        .expect("operating point solves");

    assert_eq!(set.analysis, AnalysisKind::OperatingPoint);
    assert!(set.time_fs.is_empty());
    assert!(set.is_well_formed());
    // A 10 V source across two equal resistors puts 5 V on the midpoint.
    assert_eq!(set.probes[0].samples_nano, vec![5_000_000_000]);
    assert_eq!(set.probes[0].quantity, Quantity::Voltage);
}

#[test]
fn rl_transient_decays_toward_the_analytic_curve() {
    // 1 V step, R = 1 kOhm into an inductor L = 1 uH to ground: the resistor node
    // decays as v(t) = exp(-t / tau) with tau = L/R = 1 ns.
    let mut circuit = Circuit::new();
    let vin = circuit.node("vin");
    let n1 = circuit.node("n1");
    circuit.vsource(vin, GROUND, 1.0);
    circuit.resistor(vin, n1, 1_000.0);
    circuit.inductor(n1, GROUND, 1.0e-6);

    let options = TransientOptions {
        dt_fs: 20,
        sample_times_fs: vec![0, 1_000_000, 2_000_000],
        probes: vec![ProbeSpec::voltage("n1", n1, "n1")],
    };
    let set = solve_transient(&circuit, &options).expect("RL transient solves");
    assert!(set.is_well_formed());

    let tau_ns = 1.0_f64; // L/R = 1e-6 / 1e3 s = 1 ns.
    for (&t_fs, &got) in set.time_fs.iter().zip(&set.probes[0].samples_nano) {
        let t_ns = t_fs as f64 / 1.0e6;
        let analytic_nano = ((-t_ns / tau_ns).exp() * 1.0e9 + 0.5) as i64;
        assert!(
            (got - analytic_nano).abs() <= 1,
            "at t={t_ns} ns: {got} nV vs analytic {analytic_nano} nV",
        );
    }
}

#[test]
fn operating_point_handles_a_current_source() {
    // 1 mA driven into a node with a 1 kOhm resistor to ground develops 1 V.
    let mut circuit = Circuit::new();
    let n1 = circuit.node("n1");
    circuit.isource(GROUND, n1, 1.0e-3);
    circuit.resistor(n1, GROUND, 1_000.0);

    let set = solve_operating_point(&circuit, &[ProbeSpec::voltage("n1", n1, "n1")])
        .expect("operating point solves");
    assert_eq!(set.probes[0].samples_nano, vec![1_000_000_000]);
}

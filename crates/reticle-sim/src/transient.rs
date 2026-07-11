//! Trapezoidal transient analysis and DC operating point over the dense MNA
//! solver, emitting the F4 [`WaveformSet`] directly.
//!
//! # Integration
//!
//! Reactive elements use the trapezoidal companion model. For a capacitor `C` with
//! branch current `i` (from `a` to `b`) and voltage `v = v(a) - v(b)`, one step of
//! size `h` is
//!
//! ```text
//! i_{n+1} = (2C/h) (v_{n+1} - v_n) - i_n
//! ```
//!
//! which stamps as a conductance `2C/h` in parallel with a history current source.
//! An inductor is the dual (`i_{n+1} = (h/2L) (v_{n+1} + v_n) + i_n`). Trapezoidal
//! is second order, so its error on the reference RC step shrinks as `h^2`; with a
//! small internal step it reproduces the analytic `1 - exp(-t/RC)` curve to well
//! under a nanovolt without the solver itself ever calling `exp` (the whole path is
//! `+ - * /`). Backward Euler, being first order, would need orders of magnitude
//! more steps for the same nanovolt accuracy; see ADR 0114.
//!
//! Independent sources are constant (a step applied at `t = 0`); the capacitor and
//! inductor initial conditions carry the pre-step state. Samples are quantised to
//! `i64` nano-units by rounding half up with `(x + 0.5) as i64`, which uses only
//! addition and a truncating cast and so is bit-identical on native and `wasm32`.

use crate::circuit::{Circuit, Element, NodeId};
use crate::mna::{MnaBuilder, SimError, Solution};
use crate::waveform::{AnalysisKind, Bounds, Probe, Quantity, WaveformSet};

/// The largest number of internal time steps a transient run will take.
pub const MAX_STEPS: i64 = 50_000_000;

/// A node to record into the output [`WaveformSet`].
#[derive(Clone, Debug)]
pub struct ProbeSpec {
    /// Stable id for the produced [`Probe`] (unique within a run).
    pub id: String,
    /// The node whose voltage is recorded.
    pub node: NodeId,
    /// The netlist node name written into the produced [`Probe::node`].
    pub node_name: String,
    /// What the samples measure. Node probes are [`Quantity::Voltage`].
    pub quantity: Quantity,
}

impl ProbeSpec {
    /// A voltage probe on `node` (named `node_name`) recorded under `id`.
    #[must_use]
    pub fn voltage(id: &str, node: NodeId, node_name: &str) -> Self {
        Self {
            id: id.to_owned(),
            node,
            node_name: node_name.to_owned(),
            quantity: Quantity::Voltage,
        }
    }
}

/// Options for a transient run.
#[derive(Clone, Debug)]
pub struct TransientOptions {
    /// The internal integration step in femtoseconds; strictly positive.
    pub dt_fs: i64,
    /// The times to record, in femtoseconds. Must be a strictly ascending set of
    /// non-negative multiples of `dt_fs`.
    pub sample_times_fs: Vec<i64>,
    /// The nodes to record.
    pub probes: Vec<ProbeSpec>,
}

/// Round a value in volts (or amperes) to `i64` nano-units, half up, using only
/// addition and a truncating cast so the result is identical on native and
/// `wasm32`.
fn to_nano(value: f64) -> i64 {
    let scaled = value * 1.0e9;
    if scaled >= 0.0 {
        (scaled + 0.5) as i64
    } else {
        (scaled - 0.5) as i64
    }
}

/// Trapezoidal companion state for a capacitor.
#[derive(Clone, Copy, Debug)]
struct CapState {
    a: NodeId,
    b: NodeId,
    farads: f64,
    v_prev: f64,
    i_prev: f64,
}

/// Trapezoidal companion state for an inductor.
#[derive(Clone, Copy, Debug)]
struct IndState {
    a: NodeId,
    b: NodeId,
    henries: f64,
    v_prev: f64,
    i_prev: f64,
}

/// Build the consistent `t = 0` state: pin each capacitor to its IC voltage (a
/// voltage source) and each inductor to its IC current (a current source), solve,
/// and recover the branch currents and voltages the trapezoidal history needs.
fn initial_solve(
    circuit: &Circuit,
    n_nodes: usize,
) -> Result<(Solution, Vec<CapState>, Vec<IndState>), SimError> {
    let mut caps: Vec<CapState> = Vec::new();
    let mut inds: Vec<IndState> = Vec::new();
    let mut cap_branch: Vec<usize> = Vec::new();
    let mut ic = MnaBuilder::new(n_nodes);
    for element in circuit.elements() {
        match *element {
            Element::Resistor { a, b, ohms } => ic.add_conductance(a, b, 1.0 / ohms),
            Element::Capacitor {
                a,
                b,
                farads,
                ic_volts,
            } => {
                cap_branch.push(ic.add_vsource(a, b, ic_volts));
                caps.push(CapState {
                    a,
                    b,
                    farads,
                    v_prev: ic_volts,
                    i_prev: 0.0,
                });
            }
            Element::Inductor {
                a,
                b,
                henries,
                ic_amps,
            } => {
                ic.add_current(b, a, ic_amps);
                inds.push(IndState {
                    a,
                    b,
                    henries,
                    v_prev: 0.0,
                    i_prev: ic_amps,
                });
            }
            Element::VSource { p, n, volts } => {
                ic.add_vsource(p, n, volts);
            }
            Element::ISource { p, n, amps } => ic.add_current(n, p, amps),
        }
    }
    let solution = ic.solve()?;
    for (cap, &branch) in caps.iter_mut().zip(&cap_branch) {
        cap.i_prev = solution.branch_current(branch);
    }
    for ind in &mut inds {
        ind.v_prev = solution.node_voltage(ind.a) - solution.node_voltage(ind.b);
    }
    Ok((solution, caps, inds))
}

/// Stamp the time-invariant part of the circuit (resistors and independent
/// sources) into `builder`.
fn stamp_static(builder: &mut MnaBuilder, circuit: &Circuit) {
    for element in circuit.elements() {
        match *element {
            Element::Resistor { a, b, ohms } => builder.add_conductance(a, b, 1.0 / ohms),
            Element::Capacitor { .. } | Element::Inductor { .. } => {}
            Element::VSource { p, n, volts } => {
                builder.add_vsource(p, n, volts);
            }
            Element::ISource { p, n, amps } => builder.add_current(n, p, amps),
        }
    }
}

/// Build one trapezoidal step's MNA system from the current reactive state.
fn build_step(
    circuit: &Circuit,
    n_nodes: usize,
    dt: f64,
    caps: &[CapState],
    inds: &[IndState],
) -> MnaBuilder {
    let mut builder = MnaBuilder::new(n_nodes);
    stamp_static(&mut builder, circuit);
    for cap in caps {
        let geq = 2.0 * cap.farads / dt;
        builder.add_conductance(cap.a, cap.b, geq);
        builder.add_current(cap.a, cap.b, geq * cap.v_prev + cap.i_prev);
    }
    for ind in inds {
        let geq = dt / (2.0 * ind.henries);
        builder.add_conductance(ind.a, ind.b, geq);
        builder.add_current(ind.b, ind.a, ind.i_prev + geq * ind.v_prev);
    }
    builder
}

/// Advance the reactive companion state after a step solve.
fn advance_states(solution: &Solution, dt: f64, caps: &mut [CapState], inds: &mut [IndState]) {
    for cap in caps {
        let geq = 2.0 * cap.farads / dt;
        let v_new = solution.node_voltage(cap.a) - solution.node_voltage(cap.b);
        // Update the history current using the pre-step voltage, then store v_new.
        cap.i_prev = geq * (v_new - cap.v_prev) - cap.i_prev;
        cap.v_prev = v_new;
    }
    for ind in inds {
        let geq = dt / (2.0 * ind.henries);
        let v_new = solution.node_voltage(ind.a) - solution.node_voltage(ind.b);
        ind.i_prev += geq * (v_new + ind.v_prev);
        ind.v_prev = v_new;
    }
}

/// Validate the time grid, returning the internal step count.
fn validate_grid(options: &TransientOptions) -> Result<i64, SimError> {
    if options.probes.is_empty() {
        return Err(SimError::NoProbes);
    }
    let dt_fs = options.dt_fs;
    if dt_fs <= 0 {
        return Err(SimError::BadSampleTimes);
    }
    let samples = &options.sample_times_fs;
    if samples.is_empty() {
        return Err(SimError::BadSampleTimes);
    }
    let mut prev = -1_i64;
    for &t in samples {
        if t < 0 || t <= prev || t % dt_fs != 0 {
            return Err(SimError::BadSampleTimes);
        }
        prev = t;
    }
    let n_steps = prev / dt_fs;
    if n_steps > MAX_STEPS {
        return Err(SimError::TooLarge);
    }
    Ok(n_steps)
}

/// Solve a transient analysis, producing the F4 [`WaveformSet`] directly.
///
/// The capacitor/inductor initial conditions seed a consistent `t = 0` state, then
/// the trapezoidal companion models step the circuit forward, recording each node
/// probe at the requested sample times.
///
/// # Errors
///
/// Returns [`SimError::NoProbes`] for an empty probe list,
/// [`SimError::BadSampleTimes`] if the sample times are not a sorted set of
/// non-negative multiples of `dt_fs`, [`SimError::TooLarge`] if the step count
/// exceeds [`MAX_STEPS`], and any solver error from a singular system.
pub fn solve_transient(
    circuit: &Circuit,
    options: &TransientOptions,
) -> Result<WaveformSet, SimError> {
    let n_steps = validate_grid(options)?;
    let dt_fs = options.dt_fs;
    let samples = &options.sample_times_fs;
    let dt = dt_fs as f64 * 1.0e-15;
    let n_nodes = circuit.num_nodes();

    let (ic_solution, mut caps, mut inds) = initial_solve(circuit, n_nodes)?;

    let mut series: Vec<Vec<i64>> = vec![Vec::with_capacity(samples.len()); options.probes.len()];
    let mut next = 0_usize;
    if samples[next] == 0 {
        for (probe, out) in options.probes.iter().zip(&mut series) {
            out.push(to_nano(ic_solution.node_voltage(probe.node)));
        }
        next += 1;
    }

    for step in 1..=n_steps {
        let t_fs = step * dt_fs;
        let solution = build_step(circuit, n_nodes, dt, &caps, &inds).solve()?;
        advance_states(&solution, dt, &mut caps, &mut inds);
        if next < samples.len() && samples[next] == t_fs {
            for (probe, out) in options.probes.iter().zip(&mut series) {
                out.push(to_nano(solution.node_voltage(probe.node)));
            }
            next += 1;
        }
    }

    let probes = build_probes(&options.probes, series);
    let bounds = transient_bounds(samples, &probes);
    Ok(WaveformSet {
        analysis: AnalysisKind::Transient,
        time_fs: samples.clone(),
        probes,
        bounds,
    })
}

/// Solve the DC operating point (capacitors open, inductors shorted), producing an
/// [`AnalysisKind::OperatingPoint`] [`WaveformSet`] with one sample per probe.
///
/// # Errors
///
/// Returns [`SimError::NoProbes`] for an empty probe list and any solver error from
/// a singular system.
pub fn solve_operating_point(
    circuit: &Circuit,
    probes: &[ProbeSpec],
) -> Result<WaveformSet, SimError> {
    if probes.is_empty() {
        return Err(SimError::NoProbes);
    }
    let n_nodes = circuit.num_nodes();
    let mut builder = MnaBuilder::new(n_nodes);
    for element in circuit.elements() {
        match *element {
            Element::Resistor { a, b, ohms } => builder.add_conductance(a, b, 1.0 / ohms),
            // A capacitor is an open circuit at DC; contribute nothing.
            Element::Capacitor { .. } => {}
            // An inductor is a short at DC: a zero-volt source.
            Element::Inductor { a, b, .. } => {
                builder.add_vsource(a, b, 0.0);
            }
            Element::VSource { p, n, volts } => {
                builder.add_vsource(p, n, volts);
            }
            Element::ISource { p, n, amps } => builder.add_current(n, p, amps),
        }
    }
    let solution = builder.solve()?;

    let mut y_min = i64::MAX;
    let mut y_max = i64::MIN;
    let out_probes: Vec<Probe> = probes
        .iter()
        .map(|spec| {
            let nano = to_nano(solution.node_voltage(spec.node));
            y_min = y_min.min(nano);
            y_max = y_max.max(nano);
            Probe {
                id: spec.id.clone(),
                node: spec.node_name.clone(),
                quantity: spec.quantity,
                samples_nano: vec![nano],
            }
        })
        .collect();

    Ok(WaveformSet {
        analysis: AnalysisKind::OperatingPoint,
        time_fs: Vec::new(),
        probes: out_probes,
        bounds: Bounds {
            t_min_fs: 0,
            t_max_fs: 0,
            y_min_nano: y_min,
            y_max_nano: y_max,
        },
    })
}

/// Attach each probe spec's identity to its recorded series.
fn build_probes(specs: &[ProbeSpec], series: Vec<Vec<i64>>) -> Vec<Probe> {
    specs
        .iter()
        .zip(series)
        .map(|(spec, samples_nano)| Probe {
            id: spec.id.clone(),
            node: spec.node_name.clone(),
            quantity: spec.quantity,
            samples_nano,
        })
        .collect()
}

/// Compute the axis bounds for a transient set from its time axis and probes.
fn transient_bounds(samples: &[i64], probes: &[Probe]) -> Bounds {
    let mut y_min = i64::MAX;
    let mut y_max = i64::MIN;
    for probe in probes {
        for &sample in &probe.samples_nano {
            y_min = y_min.min(sample);
            y_max = y_max.max(sample);
        }
    }
    Bounds {
        t_min_fs: *samples.first().unwrap_or(&0),
        t_max_fs: *samples.last().unwrap_or(&0),
        y_min_nano: y_min,
        y_max_nano: y_max,
    }
}

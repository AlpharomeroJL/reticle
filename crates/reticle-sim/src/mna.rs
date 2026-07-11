//! Dense modified-nodal-analysis (MNA) assembly and a hand-rolled dense linear
//! solve.
//!
//! The system is `A x = z`, where the first `n_nodes` unknowns are non-ground node
//! voltages and the remaining unknowns are branch currents (one per voltage
//! source stamped). [`MnaBuilder`] accumulates conductance, current-injection and
//! voltage-source stamps, then [`MnaBuilder::solve`] assembles a dense row-major
//! matrix and runs Gaussian elimination with partial pivoting.
//!
//! # Determinism
//!
//! The solve uses only `+ - * /` and magnitude comparisons over `f64`; there is no
//! transcendental function, no fused multiply-add, and no external BLAS. IEEE-754
//! double arithmetic and comparisons are bit-identical between the native and
//! `wasm32` targets, so the same system yields byte-identical results on both.
//! Partial pivoting only reorders rows by an exact `f64` magnitude comparison, so it
//! stays deterministic while keeping MNA systems (whose voltage-source rows have a
//! zero diagonal) solvable.

/// The largest MNA system the solver will assemble, guarding against a pathological
/// circuit exhausting memory. A "bounded" small-circuit solver never approaches it.
pub const MAX_UNKNOWNS: usize = 4096;

/// An error from assembling or solving an MNA system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimError {
    /// The assembled matrix was singular (a zero pivot survived pivoting): the
    /// circuit is under-constrained (for example a floating node or a voltage-source
    /// loop).
    SingularSystem,
    /// The system had no unknowns (no non-ground nodes and no sources).
    EmptySystem,
    /// The system exceeded [`MAX_UNKNOWNS`], or a requested step or sample count
    /// exceeded its bound.
    TooLarge,
    /// The requested sample times were not a sorted set of non-negative multiples of
    /// the internal step.
    BadSampleTimes,
    /// The analysis was asked to produce no probes.
    NoProbes,
}

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            SimError::SingularSystem => "MNA system is singular (under-constrained circuit)",
            SimError::EmptySystem => "MNA system has no unknowns",
            SimError::TooLarge => "circuit or time grid exceeds the solver bound",
            SimError::BadSampleTimes => "sample times must be sorted non-negative multiples of dt",
            SimError::NoProbes => "analysis requested no probes",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for SimError {}

use crate::circuit::{GROUND, NodeId};

/// Accumulates MNA stamps and assembles the dense system.
#[derive(Clone, Debug)]
pub struct MnaBuilder {
    n_nodes: usize,
    conductances: Vec<(NodeId, NodeId, f64)>,
    currents: Vec<(NodeId, NodeId, f64)>,
    vsources: Vec<(NodeId, NodeId, f64)>,
}

impl MnaBuilder {
    /// A builder for a system with `n_nodes` non-ground node unknowns.
    #[must_use]
    pub fn new(n_nodes: usize) -> Self {
        Self {
            n_nodes,
            conductances: Vec::new(),
            currents: Vec::new(),
            vsources: Vec::new(),
        }
    }

    /// Stamp a conductance `g` (siemens) between nodes `a` and `b`.
    pub fn add_conductance(&mut self, a: NodeId, b: NodeId, g: f64) {
        self.conductances.push((a, b, g));
    }

    /// Stamp a current source of `amps` flowing from `out` into `into` (it adds to
    /// the KCL right-hand side at `into` and subtracts at `out`).
    pub fn add_current(&mut self, into: NodeId, out: NodeId, amps: f64) {
        self.currents.push((into, out, amps));
    }

    /// Stamp an independent voltage source holding `volts = v(p) - v(n)`, returning
    /// the index of its branch-current unknown (for reading the current back out of
    /// a [`Solution`]).
    pub fn add_vsource(&mut self, p: NodeId, n: NodeId, volts: f64) -> usize {
        let branch = self.vsources.len();
        self.vsources.push((p, n, volts));
        branch
    }

    /// The number of branch (voltage-source) unknowns stamped so far.
    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.vsources.len()
    }

    /// Assemble and solve the system, returning the solution vector wrapped for
    /// node/branch access.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::EmptySystem`] if there are no unknowns,
    /// [`SimError::TooLarge`] if the unknown count exceeds [`MAX_UNKNOWNS`], and
    /// [`SimError::SingularSystem`] if the matrix is singular.
    pub fn solve(self) -> Result<Solution, SimError> {
        let size = self.n_nodes + self.vsources.len();
        if size == 0 {
            return Err(SimError::EmptySystem);
        }
        if size > MAX_UNKNOWNS {
            return Err(SimError::TooLarge);
        }

        let mut a = vec![0.0_f64; size * size];
        let mut z = vec![0.0_f64; size];

        // Node index into the matrix, or `None` for ground (which is dropped).
        let ni =
            |node: NodeId| -> Option<usize> { if node == GROUND { None } else { Some(node - 1) } };

        for &(a_node, b_node, g) in &self.conductances {
            let ia = ni(a_node);
            let ib = ni(b_node);
            if let Some(r) = ia {
                a[r * size + r] += g;
            }
            if let Some(r) = ib {
                a[r * size + r] += g;
            }
            if let (Some(r), Some(c)) = (ia, ib) {
                a[r * size + c] -= g;
                a[c * size + r] -= g;
            }
        }

        for &(into, out, amps) in &self.currents {
            if let Some(r) = ni(into) {
                z[r] += amps;
            }
            if let Some(r) = ni(out) {
                z[r] -= amps;
            }
        }

        for (branch, &(p, n, volts)) in self.vsources.iter().enumerate() {
            let br = self.n_nodes + branch;
            if let Some(r) = ni(p) {
                a[r * size + br] += 1.0;
                a[br * size + r] += 1.0;
            }
            if let Some(r) = ni(n) {
                a[r * size + br] -= 1.0;
                a[br * size + r] -= 1.0;
            }
            z[br] += volts;
        }

        let x = gaussian_solve(a, z, size)?;
        Ok(Solution {
            n_nodes: self.n_nodes,
            x,
        })
    }
}

/// The solved unknown vector, with helpers to read node voltages and branch
/// currents.
#[derive(Clone, Debug)]
pub struct Solution {
    n_nodes: usize,
    x: Vec<f64>,
}

impl Solution {
    /// The voltage at `node` (zero for [`GROUND`]).
    #[must_use]
    pub fn node_voltage(&self, node: NodeId) -> f64 {
        if node == GROUND {
            0.0
        } else {
            self.x[node - 1]
        }
    }

    /// The current through the voltage-source branch with the given index (as
    /// returned by [`MnaBuilder::add_vsource`]).
    #[must_use]
    pub fn branch_current(&self, branch: usize) -> f64 {
        self.x[self.n_nodes + branch]
    }
}

/// The absolute value of `x` using only comparison and negation, so the pivot
/// search stays inside the `+ - * /` (and compare) determinism envelope.
fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}

/// Solve the dense row-major system `A x = z` (size `n`) by Gaussian elimination
/// with partial pivoting. Uses only `+ - * /` and magnitude comparisons.
///
/// # Errors
///
/// Returns [`SimError::SingularSystem`] if a column has no non-zero pivot.
fn gaussian_solve(mut a: Vec<f64>, mut z: Vec<f64>, n: usize) -> Result<Vec<f64>, SimError> {
    for col in 0..n {
        // Partial pivot: pick the row with the largest magnitude in this column.
        let mut pivot_row = col;
        let mut best = abs(a[col * n + col]);
        for row in (col + 1)..n {
            let candidate = abs(a[row * n + col]);
            if candidate > best {
                best = candidate;
                pivot_row = row;
            }
        }
        if best > 0.0 {
            // A usable pivot exists.
        } else {
            return Err(SimError::SingularSystem);
        }
        if pivot_row != col {
            for k in 0..n {
                a.swap(col * n + k, pivot_row * n + k);
            }
            z.swap(col, pivot_row);
        }

        let pivot = a[col * n + col];
        for row in (col + 1)..n {
            let factor = a[row * n + col] / pivot;
            // Eliminate the sub-diagonal entry; columns left of `col` are already zero.
            for k in col..n {
                let above = a[col * n + k];
                a[row * n + k] -= factor * above;
            }
            z[row] -= factor * z[col];
        }
    }

    // Back-substitution.
    let mut x = vec![0.0_f64; n];
    for row in (0..n).rev() {
        let mut acc = z[row];
        for k in (row + 1)..n {
            acc -= a[row * n + k] * x[k];
        }
        x[row] = acc / a[row * n + row];
    }
    Ok(x)
}

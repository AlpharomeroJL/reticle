//! The linear circuit model the solver consumes: named nodes and two-terminal
//! elements (resistor, capacitor, inductor, independent voltage and current
//! sources).
//!
//! Node `0` is always ground (the reference node); every other node is interned
//! by name so a probe can name the netlist node it follows. Values are SI
//! (ohms, farads, henries, volts, amperes, seconds); the [`transient`](crate::transient)
//! stepper converts the femtosecond time axis to seconds internally.
//!
//! This is the model for a pure-Rust modified-nodal-analysis (MNA) solver, not a
//! wrapper over any external simulator. Every device here is a generic linear
//! element; there are no process- or vendor-specific device models.

use std::collections::HashMap;

/// Index of a circuit node into a [`Circuit`]. Node [`GROUND`] (`0`) is the
/// reference node and never appears in the MNA matrix.
pub type NodeId = usize;

/// The ground / reference node. Its voltage is fixed at zero and it contributes
/// no row or column to the MNA system.
pub const GROUND: NodeId = 0;

/// A linear two-terminal circuit element. All values are SI units.
///
/// Capacitors and inductors carry an initial condition (`ic_volts` / `ic_amps`)
/// used to seed a consistent state at `t = 0` for a transient analysis.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// A resistor of `ohms` ohms between nodes `a` and `b`.
    Resistor {
        /// First terminal.
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Resistance in ohms; must be strictly positive.
        ohms: f64,
    },
    /// A capacitor of `farads` farads between nodes `a` and `b`, with an initial
    /// voltage `ic_volts` across it (`v(a) - v(b)` at `t = 0`).
    Capacitor {
        /// First terminal (the `+` reference for `ic_volts`).
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Capacitance in farads; must be strictly positive.
        farads: f64,
        /// Initial voltage across the capacitor at `t = 0`.
        ic_volts: f64,
    },
    /// An inductor of `henries` henries between nodes `a` and `b`, with an
    /// initial current `ic_amps` flowing from `a` to `b` at `t = 0`.
    Inductor {
        /// First terminal.
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Inductance in henries; must be strictly positive.
        henries: f64,
        /// Initial current from `a` to `b` at `t = 0`.
        ic_amps: f64,
    },
    /// An independent voltage source holding `volts = v(p) - v(n)`.
    VSource {
        /// Positive terminal.
        p: NodeId,
        /// Negative terminal.
        n: NodeId,
        /// Enforced voltage `v(p) - v(n)`.
        volts: f64,
    },
    /// An independent current source driving `amps` from `p` to `n` (current
    /// leaves node `p` and enters node `n`).
    ISource {
        /// Node the current leaves.
        p: NodeId,
        /// Node the current enters.
        n: NodeId,
        /// Source current in amperes.
        amps: f64,
    },
}

/// A linear circuit: interned nodes plus a list of [`Element`]s.
///
/// Build one with [`Circuit::new`], intern nodes with [`Circuit::node`], then add
/// elements with the builder methods. The names `"0"` and `"gnd"` both resolve to
/// [`GROUND`].
#[derive(Clone, Debug)]
pub struct Circuit {
    node_names: Vec<String>,
    node_ids: HashMap<String, NodeId>,
    elements: Vec<Element>,
    vsource_count: usize,
}

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}

impl Circuit {
    /// A new circuit containing only the ground node.
    #[must_use]
    pub fn new() -> Self {
        let mut node_ids = HashMap::new();
        node_ids.insert("0".to_owned(), GROUND);
        node_ids.insert("gnd".to_owned(), GROUND);
        Self {
            node_names: vec!["0".to_owned()],
            node_ids,
            elements: Vec::new(),
            vsource_count: 0,
        }
    }

    /// Intern `name`, returning its stable [`NodeId`]. `"0"` and `"gnd"` map to
    /// [`GROUND`]; any other name gets a fresh id on first use.
    pub fn node(&mut self, name: &str) -> NodeId {
        if let Some(&id) = self.node_ids.get(name) {
            return id;
        }
        let id = self.node_names.len();
        self.node_names.push(name.to_owned());
        self.node_ids.insert(name.to_owned(), id);
        id
    }

    /// The display name of `id`, or `"0"` for an out-of-range id.
    #[must_use]
    pub fn node_name(&self, id: NodeId) -> &str {
        self.node_names.get(id).map_or("0", String::as_str)
    }

    /// The number of non-ground nodes, i.e. the count of MNA node unknowns.
    #[must_use]
    pub fn num_nodes(&self) -> usize {
        self.node_names.len() - 1
    }

    /// The number of independent voltage sources (each adds a branch unknown).
    #[must_use]
    pub fn vsource_count(&self) -> usize {
        self.vsource_count
    }

    /// The elements in insertion order.
    #[must_use]
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Add a resistor of `ohms` between `a` and `b`.
    pub fn resistor(&mut self, a: NodeId, b: NodeId, ohms: f64) {
        self.elements.push(Element::Resistor { a, b, ohms });
    }

    /// Add a capacitor of `farads` between `a` and `b` with zero initial voltage.
    pub fn capacitor(&mut self, a: NodeId, b: NodeId, farads: f64) {
        self.capacitor_with_ic(a, b, farads, 0.0);
    }

    /// Add a capacitor of `farads` between `a` and `b` with initial voltage
    /// `ic_volts = v(a) - v(b)` at `t = 0`.
    pub fn capacitor_with_ic(&mut self, a: NodeId, b: NodeId, farads: f64, ic_volts: f64) {
        self.elements.push(Element::Capacitor {
            a,
            b,
            farads,
            ic_volts,
        });
    }

    /// Add an inductor of `henries` between `a` and `b` with zero initial current.
    pub fn inductor(&mut self, a: NodeId, b: NodeId, henries: f64) {
        self.inductor_with_ic(a, b, henries, 0.0);
    }

    /// Add an inductor of `henries` between `a` and `b` with initial current
    /// `ic_amps` from `a` to `b` at `t = 0`.
    pub fn inductor_with_ic(&mut self, a: NodeId, b: NodeId, henries: f64, ic_amps: f64) {
        self.elements.push(Element::Inductor {
            a,
            b,
            henries,
            ic_amps,
        });
    }

    /// Add an independent voltage source holding `volts = v(p) - v(n)`.
    pub fn vsource(&mut self, p: NodeId, n: NodeId, volts: f64) {
        self.elements.push(Element::VSource { p, n, volts });
        self.vsource_count += 1;
    }

    /// Add an independent current source driving `amps` from `p` to `n`.
    pub fn isource(&mut self, p: NodeId, n: NodeId, amps: f64) {
        self.elements.push(Element::ISource { p, n, amps });
    }
}

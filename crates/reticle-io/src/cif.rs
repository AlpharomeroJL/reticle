//! CIF (Caltech Intermediate Format) reader: the classic subset (DS/DF cell
//! definitions, `L` layers, `B` boxes, `P` polygons, `W` wires, `C` calls with
//! transforms), a documented units convention, and malformed-input tests, with
//! every count-driven allocation capped against the remaining input.
//!
//! Reserved by the Phase 1 pre-fan-out commit and implemented by the `cif` lane.
//! Until then this module is intentionally empty so the workspace and `just ci`
//! stay green and no lane has to touch `lib.rs`.

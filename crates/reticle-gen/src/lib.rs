//! Parameterized layout generators for Reticle.
//!
//! A generator is a pure function from typed parameters plus a technology to
//! geometry: the repetitive structures a layout engineer would otherwise draw by
//! hand (guard rings, via arrays, and, as lanes 2B and 2C add them, more) built once
//! and stamped out from a few numbers. Every generator here is **DRC-clean by
//! construction** against the SKY130 subset: the geometry it emits satisfies the
//! cited width, spacing, enclosure, and area rules for the layers it touches, and a
//! property test proves it by running the real [`reticle_drc`] engine over
//! randomized parameter sweeps and asserting zero violations.
//!
//! # The contract
//!
//! [`Generator`] is the trait a concrete generator implements. Each has a typed,
//! serde-round-trippable [`GenParams`] struct with per-field ranges, defaults, a
//! [`validate`](GenParams::validate), and a [`schema`](GenParams::schema); and a
//! [`generate`](Generator::generate) that appends [`DrawShape`](reticle_model::DrawShape)s
//! to a caller-provided [`Cell`](reticle_model::Cell). A blanket impl exposes every
//! generator through the type-erased [`ErasedGenerator`], which moves parameters as
//! JSON so heterogeneous generators live behind one `dyn` type.
//!
//! The [`Registry`] enumerates and drives generators by id without naming concrete
//! types: [`Registry::infos`] lists ids, titles, descriptions, and schemas (which
//! lane 2D renders as a UI form and a model-facing tool schema), and
//! [`Registry::generate`] invokes one from an id plus JSON parameters.
//!
//! ```
//! use reticle_gen::Registry;
//! use reticle_model::{Cell, Technology};
//!
//! let reg = Registry::with_builtins();
//! assert!(reg.ids().contains(&"guard_ring"));
//!
//! // Seed a form (or a model call) from the schema's defaults, then generate.
//! let params = reg.default_params("via_farm").expect("registered");
//! let mut cell = Cell::new("top");
//! let out = reg
//!     .generate("via_farm", &params, &Technology::default(), &mut cell)
//!     .expect("valid defaults generate");
//! assert_eq!(out.shapes_added, cell.shapes.len());
//! ```
//!
//! # Scope and honesty
//!
//! The generators target the committed [`sky130_drc_rules`](reticle_drc::sky130_drc_rules)
//! subset, not the full SKY130 deck; passing it is not tape-out clean. The
//! [`GuardRing`] draws on the interconnect layers the subset carries rules for
//! (`li1`, `met1`, `met2`, `met3`) and lines an `li1` ring with `licon` taps; the
//! [`ViaFarm`] bridges the adjacent metal-stack pairs the subset carries cut and
//! enclosure rules for (`mcon`, `via`, `via2`). The [`FillGen`] tiles a region on
//! those same interconnect layers, honoring keep-outs and approaching a target
//! coverage density; the subset carries no maximum-density rule, so the target is a
//! fill objective and the achieved density is reported honestly rather than claimed.
//! The [`TestStructure`] emits the classic probe-able tiles (van der Pauw cross,
//! contact chain, comb, serpentine) from axis-aligned rectangles and `mcon` contacts
//! on the subset's layers. Cut-to-cut spacing has no rule in the subset, so an array
//! pitch is a conservative choice rather than a checked constraint (noted at the call
//! site). See the `sky130` numbers module for exactly which values are baked in and
//! the test that ties them to the committed deck.
//!
//! # Purity and portability
//!
//! This crate is pure geometry: no filesystem, GPU, threads, or global state. It
//! depends only on [`reticle_geometry`], [`reticle_model`], [`reticle_drc`] (for the
//! cleanliness oracle), and serde, and it compiles for `wasm32-unknown-unknown`, so
//! lane 2D can run generators directly in the browser Generate panel.

#![forbid(unsafe_code)]

mod error;
mod fill;
mod generator;
mod guard_ring;
mod registry;
mod schema;
mod sky130;
mod test_structure;
mod via_farm;

pub use error::GenError;
pub use fill::{FillGen, FillLayer, FillParams, KeepOut};
pub use generator::{ErasedGenerator, GenOutput, GenParams, Generator};
pub use guard_ring::{GuardRing, GuardRingParams, RingLayer};
pub use registry::{GeneratorInfo, Registry};
pub use schema::{FieldSchema, FieldType, ParamSchema};
pub use test_structure::{StructureKind, StructureLayer, TestStructure, TestStructureParams};
pub use via_farm::{CutKind, ViaFarm, ViaFarmParams};

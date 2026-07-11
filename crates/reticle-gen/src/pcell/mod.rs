//! User-defined parametric cells (PCells): a rhai script plus a typed parameter schema,
//! produced into geometry with a stable content identity.
//!
//! # The pieces (Phase 2 scaffolding; ADR 0107)
//!
//! - [`PCellDef`] (`def.rs`, `pcell-params` lane): the definition (script + [`ParamSchema`],
//!   engine version), its parameter validation, and its F2 provenance.
//! - [`PCellRegistry`] (`registry.rs`, `pcell-params` lane): user PCells addressable by id,
//!   parallel to the built-in generator [`Registry`](crate::Registry).
//! - [`param_hash`] (`hash.rs`, frozen scaffold): the shared content identity every lane
//!   keys on, front-loaded so the parallel lanes agree on one tested implementation.
//! - [`PCellCache`] / [`CacheStats`] (`cache.rs`, `pcell-cache` lane): the produced-cell
//!   cache keyed by [`param_hash`].
//!
//! The sandboxed producer that actually runs a PCell's script under resource limits lives in
//! `reticle_script` (`pcell-produce` lane); it is the first `reticle-script -> reticle-gen`
//! edge (acyclic), and it consumes [`PCellDef`] and stamps the [`ProduceMeta`](crate::ProduceMeta)
//! this module's identity gives it.
//!
//! [`ParamSchema`]: crate::ParamSchema

mod cache;
mod def;
mod hash;
mod registry;

pub use cache::{CacheStats, PCellCache};
pub use def::PCellDef;
pub use hash::param_hash;
pub use registry::PCellRegistry;

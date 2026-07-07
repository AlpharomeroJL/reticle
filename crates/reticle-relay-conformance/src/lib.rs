//! A two-target conformance suite for the Reticle collaboration relay.
//!
//! Reticle ships two relays that must be interchangeable: the native
//! `reticle_server` (axum + tokio) and a Cloudflare Durable Object (`worker/`).
//! The protocol, not the relay, is the asset. This crate expresses the relay
//! contract once, as a table of scripted [`Vector`]s (connect edit/view, send
//! update/presence/text, expect a frame or silence, burst presence), and runs
//! the identical table against either relay through one driver.
//!
//! # Targets
//!
//! * [`Target::native`] spawns the in-process axum relay on an ephemeral port
//!   and drives it with `tokio_tungstenite`, exactly like
//!   `reticle-server/tests/share_live.rs`.
//! * [`Target::external`] drives any relay by URL: the Durable Object under
//!   `wrangler dev --local` (see `tests/conformance_do.rs`, gated by
//!   `RETICLE_CONFORMANCE_DO=1`) or a deployed `wss://...workers.dev`.
//!
//! # What the vectors freeze
//!
//! Every clause of the contract is a vector (see [`vectors::vectors`]): late-join
//! log replay in order; view-mode frames dropped server-side; echo suppression;
//! presence coalescing (the one target-aware behavior, since the free-tier
//! Durable Object collapses presence while the native relay does not, both
//! preserving convergence); updates never coalesced; full-log replay (the room
//! cap observable); two-room isolation; and the binary-only rule. The vector
//! format freezes at the 1B merge (ADR 0061).
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), reticle_relay_conformance::Failure> {
//! use reticle_relay_conformance::{Target, run_vector, vectors};
//!
//! let target = Target::native().await;
//! for vector in vectors() {
//!     run_vector(&target, &vector).await?;
//! }
//! # Ok(())
//! # }
//! ```

pub mod frames;
pub mod runner;
pub mod target;
pub mod vectors;

pub use runner::{Action, Failure, Mode, Payload, Vector, run_vector};
pub use target::Target;
pub use vectors::vectors;

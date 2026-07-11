//! Sandboxed WebAssembly plugin runtime for Reticle: instantiate a plugin from
//! bytes under fuel and memory limits, expose a v0 host-function table of
//! read-only queries plus a staged-edit funnel through the command and undo
//! machinery (so plugins are replayable and undoable by construction), and gate
//! capabilities at instantiation from the manifest permissions.
//!
//! Scaffolded in the v8.2 campaign Phase 0. This crate currently ships the F5
//! plugin manifest + ABI v0 + static-index contract ([`manifest`]); the host,
//! the wasm loader, and the sample plugin land in Phase 4. The ABI is explicitly
//! unstable until the v8.2.0 tag: [`manifest::ABI_VERSION`] is `0`, and a
//! manifest's `api_version` exists so a post-campaign break is honest.
//! Fixture-first: the manager UI builds against the committed index fixture
//! (`tests/fixtures/contracts/f5_index.json`) before the host exists.

pub mod manifest;

// The wasm plugin host. Native-only: the interpreter (`wasmi`) and the
// command/undo machinery it funnels edits through are `cfg(not(wasm32))`, so
// nothing here enters the wasm bundle (ADR 0115/0116). The browser build gets
// browse/preview and a "plugins run in the desktop app" disclaimer instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod host;

pub use manifest::{ABI_VERSION, HostFn, Index, IndexEntry, Manifest, ManifestError, Permission};

#[cfg(not(target_arch = "wasm32"))]
pub use host::{EditDecodeError, Host, HostContext, HostError, Limits, RunOutcome};

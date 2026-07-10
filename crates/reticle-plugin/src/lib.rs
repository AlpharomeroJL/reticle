//! Sandboxed WebAssembly plugin runtime for Reticle: instantiate a plugin from
//! bytes under fuel and memory limits, expose a v0 host-function table of
//! read-only queries plus a staged-edit funnel through the command and undo
//! machinery (so plugins are replayable and undoable by construction), and gate
//! capabilities at instantiation from the manifest permissions.
//!
//! Scaffolded in the v8.2 campaign Phase 0 as the home for the F5 plugin
//! manifest + ABI v0 + static index contract. The ABI is explicitly unstable
//! until the v8.2.0 tag (`api_version` exists so post-campaign breaks are
//! honest). The host, manifest parser, and index land in Phase 4; this crate
//! compiles empty until then so the workspace and `just ci` stay green.

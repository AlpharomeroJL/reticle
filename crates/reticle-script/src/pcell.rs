//! The sandboxed PCell producer: runs a user [`PCellDef`]'s rhai script under strict
//! resource limits and returns the produced [`Cell`] plus its F2 provenance.
//!
//! This is the first `reticle-script -> reticle-gen` edge (acyclic): the producer consumes a
//! [`PCellDef`] (its script and parameter schema) from [`reticle_gen`] and stamps the
//! [`ProduceMeta`] that crate's [`param_hash`](reticle_gen::param_hash) identity gives it.
//!
//! SCAFFOLD OWNED BY THE `pcell-produce` LANE (ADR 0107). The public signature of
//! [`produce`], the [`SandboxLimits`], and the [`ProduceError`] variants are fixed here so
//! the `pcell-cache` and `pcell-harness` lanes and the app compile against a stable
//! interface. The `pcell-produce` lane builds the actual sandbox and body:
//!
//! * a fresh [`rhai::Engine`] with the create/edit/query API registered but **no**
//!   filesystem, plugin-directory, or `import` access (a hostile script cannot read the
//!   disk or exhaust it),
//! * an [`Engine::on_progress`] operation-count limit and an output-size cap enforcing
//!   [`SandboxLimits`], so a runaway or adversarial script is rejected rather than hanging
//!   the tab or exhausting memory (the untrusted-input invariant, applied to scripts),
//! * parameter injection (bind each schema field's value into the script's scope) and
//!   extraction of the produced top [`Cell`].
//!
//! It must NOT route through [`ScriptEngine::run_plugin_dir`](crate::ScriptEngine::run_plugin_dir),
//! whose filesystem loader is the opposite of a sandbox.

use reticle_gen::{PCellDef, ProduceMeta};
use reticle_model::{Cell, Technology};
use serde_json::Value;

/// Resource limits a PCell script runs under, so a runaway or hostile script is rejected
/// rather than hanging or exhausting memory.
///
/// The defaults are the scaffold's conservative starting point; the `pcell-produce` lane
/// tunes them and, crucially, *enforces* them (the scaffold only declares them).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SandboxLimits {
    /// Maximum rhai operations before the run is aborted (via [`rhai::Engine::on_progress`]).
    pub max_operations: u64,
    /// Maximum number of shapes the produced geometry may contain.
    pub max_shapes: usize,
    /// Maximum number of cells the script may create.
    pub max_cells: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            max_operations: 5_000_000,
            max_shapes: 2_000_000,
            max_cells: 4_096,
        }
    }
}

/// Why a PCell produce failed. Every variant is a clean error, never a panic: a bad script
/// is untrusted input and must fail gracefully.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProduceError {
    /// The producer body is not yet implemented (scaffold stub; the `pcell-produce` lane
    /// replaces every return of this variant).
    NotImplemented,
    /// The parameters failed validation against the PCell's schema.
    InvalidParams(String),
    /// The script failed to compile or trapped at runtime (message carries the diagnostic).
    Script(String),
    /// The script exceeded a [`SandboxLimits`] bound (operations, shapes, or cells).
    LimitExceeded(String),
    /// The script produced no usable top cell.
    NoTopCell,
}

impl std::fmt::Display for ProduceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotImplemented => write!(f, "PCell produce is not implemented yet"),
            Self::InvalidParams(m) => write!(f, "invalid PCell parameters: {m}"),
            Self::Script(m) => write!(f, "PCell script error: {m}"),
            Self::LimitExceeded(m) => write!(f, "PCell script exceeded a sandbox limit: {m}"),
            Self::NoTopCell => write!(f, "PCell script produced no top cell"),
        }
    }
}

impl std::error::Error for ProduceError {}

/// Produces the geometry of `def` for `params` under `limits`, returning the produced top
/// [`Cell`] and its F2 [`ProduceMeta`] provenance.
///
/// SCAFFOLD: returns [`ProduceError::NotImplemented`] until the `pcell-produce` lane builds
/// the sandbox and execution. The signature is frozen; the `tech` argument is threaded so a
/// script can resolve named layers against the active technology once the body exists.
///
/// # Errors
///
/// Returns a [`ProduceError`] if the parameters are invalid, the script fails or exceeds a
/// [`SandboxLimits`] bound, or no top cell is produced.
pub fn produce(
    def: &PCellDef,
    params: &Value,
    tech: &Technology,
    limits: SandboxLimits,
) -> Result<(Cell, ProduceMeta), ProduceError> {
    // pcell-produce lane: validate params, build the sandboxed engine, inject params, run
    // the script under `limits`, extract the top cell, and stamp `def.produce_meta(params)`.
    let _ = (def, params, tech, limits);
    Err(ProduceError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::{ProduceError, SandboxLimits, produce};
    use reticle_gen::{PCellDef, ParamSchema};
    use reticle_model::Technology;
    use serde_json::json;

    fn tiny_def() -> PCellDef {
        PCellDef {
            id: "user.tiny".to_owned(),
            title: "Tiny".to_owned(),
            description: "scaffold test".to_owned(),
            schema: ParamSchema::default(),
            script: "create_cell(\"TOP\");".to_owned(),
            engine_version: "8.2.0".to_owned(),
        }
    }

    #[test]
    fn scaffold_produce_reports_not_implemented_without_panicking() {
        // The scaffold interface exists and fails cleanly; the pcell-produce lane replaces
        // this expectation with real production behavior.
        let err = produce(
            &tiny_def(),
            &json!({}),
            &Technology::default(),
            SandboxLimits::default(),
        )
        .expect_err("scaffold is not implemented");
        assert_eq!(err, ProduceError::NotImplemented);
    }

    #[test]
    fn sandbox_limits_have_conservative_defaults() {
        let l = SandboxLimits::default();
        assert!(l.max_operations > 0 && l.max_shapes > 0 && l.max_cells > 0);
    }
}

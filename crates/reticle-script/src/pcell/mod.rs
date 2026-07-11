//! The sandboxed PCell producer: runs a user [`PCellDef`]'s rhai script under strict
//! resource limits and returns the produced [`Cell`] plus its F2 provenance.
//!
//! This is the first `reticle-script -> reticle-gen` edge (acyclic): the producer consumes a
//! [`PCellDef`] (its script and parameter schema) from [`reticle_gen`] and stamps the
//! [`ProduceMeta`] that crate's [`param_hash`](reticle_gen::param_hash) identity gives it.
//!
//! # The sandbox (why a fresh engine, not [`ScriptEngine`](crate::ScriptEngine))
//!
//! A user PCell script is *untrusted input*, exactly like a parsed GDS/OASIS file. It must
//! never read the disk, never exhaust memory, and never hang or panic the process (a wasm
//! panic kills the browser tab). So the producer builds a **fresh** [`rhai::Engine`] hardened
//! by the internal `sandbox` module rather than reusing
//! [`ScriptEngine::run_plugin_dir`](crate::ScriptEngine::run_plugin_dir), whose filesystem
//! loader is the opposite of a sandbox:
//!
//! * the create/edit/query/transform/DRC/export API is registered (none of it touches the
//!   filesystem), but rhai's default `FileModuleResolver` is replaced by a dummy so `import`
//!   cannot read `.rhai` files from disk;
//! * an [`Engine::on_progress`](rhai::Engine::on_progress) callback enforces
//!   [`SandboxLimits::max_operations`] (a runaway loop is aborted, never hangs) together with
//!   the [`max_shapes`](SandboxLimits::max_shapes) / [`max_cells`](SandboxLimits::max_cells)
//!   output caps;
//! * rhai's string/array/map size caps are set so exponential value growth (`s += s`) cannot
//!   exhaust memory in a handful of operations, which the operation cap alone does not catch.
//!
//! Every failure path returns a clean [`ProduceError`]; the producer never panics on any
//! script or parameter input.

mod sandbox;
#[cfg(test)]
mod tests;

use rhai::Scope;

use reticle_gen::{PCellDef, ProduceMeta};
use reticle_model::{Cell, Technology};
use serde_json::Value;

/// Resource limits a PCell script runs under, so a runaway or hostile script is rejected
/// rather than hanging or exhausting memory.
///
/// These three are the *semantic* budget for a produce (how much work, how much output). The
/// producer additionally bounds raw rhai value growth (string/array/map size) internally, an
/// orthogonal memory-safety axis not exposed here because it is not something a caller tunes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SandboxLimits {
    /// Maximum rhai operations before the run is aborted (via [`rhai::Engine::on_progress`]).
    ///
    /// This is the anti-hang bound: an infinite or long-running loop is stopped once its
    /// operation count crosses this value. `0` rejects every script (default-deny); pass a
    /// large value to run effectively unbounded.
    pub max_operations: u64,
    /// Maximum number of shape records the produced document may contain, counted across all
    /// cells the script authored. This is the *stored* geometry, not the flattened expansion
    /// of arrays: counting the expansion would let a small script with a large array multiplier
    /// force a huge allocation during produce, which the caps exist to prevent.
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
    /// Retained for interface compatibility with the Phase-2 scaffold. The implemented
    /// producer never returns this; a real failure is one of the variants below.
    NotImplemented,
    /// The parameters failed validation against the PCell's schema.
    InvalidParams(String),
    /// The script failed to compile or trapped at runtime (message carries the diagnostic).
    /// A blocked `import` or a call to a function the sandbox does not expose lands here.
    Script(String),
    /// The script exceeded a sandbox resource bound: the operation cap, an output cap
    /// ([`max_shapes`](SandboxLimits::max_shapes) / [`max_cells`](SandboxLimits::max_cells)),
    /// a rhai string/array/map size cap, the call-depth cap, or the script-source size cap.
    /// The message names which bound was hit.
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
/// The parameters are validated against the schema, then a hardened, sandboxed
/// [`rhai::Engine`] is built (see the module docs), each schema
/// field's value is injected into the script scope by name, the script runs under `limits`,
/// the produced top cell is extracted, and `def`'s [`produce_meta`](PCellDef::produce_meta) is
/// stamped. `tech` is seeded as the document's active technology so a script can check its
/// geometry against it (e.g. DRC).
///
/// The result is deterministic: the same `def`, `params`, `tech`, and `limits` yield an
/// identical [`Cell`] and an identical [`ProduceMeta::param_hash`].
///
/// # Errors
///
/// Returns a [`ProduceError`] if the parameters are invalid ([`InvalidParams`]), the script
/// fails to compile or traps ([`Script`]), it exceeds a sandbox bound ([`LimitExceeded`]), or
/// it produces no top cell ([`NoTopCell`]). Never panics.
///
/// [`InvalidParams`]: ProduceError::InvalidParams
/// [`Script`]: ProduceError::Script
/// [`LimitExceeded`]: ProduceError::LimitExceeded
/// [`NoTopCell`]: ProduceError::NoTopCell
pub fn produce(
    def: &PCellDef,
    params: &Value,
    tech: &Technology,
    limits: SandboxLimits,
) -> Result<(Cell, ProduceMeta), ProduceError> {
    // 1. Validate parameters against the schema before any script runs.
    def.validate_params(params)
        .map_err(|e| ProduceError::InvalidParams(e.to_string()))?;

    // 1b. Reject a pathologically deep parameter set before anything recurses over it (scope
    //     injection here, or the param-hash canonicalization on the success path). A hand-built
    //     deeply nested value can overflow the stack, so this is a hard boundary check.
    if sandbox::exceeds_param_depth(params) {
        return Err(ProduceError::InvalidParams(format!(
            "parameter nesting exceeds the maximum depth of {}",
            sandbox::MAX_PARAM_DEPTH
        )));
    }

    // 2. Cheap pre-parse guard: reject a pathologically large script source before the parser
    //    runs (parse cost is not covered by the runtime operation cap).
    if def.script.len() > sandbox::MAX_SCRIPT_BYTES {
        return Err(ProduceError::LimitExceeded(format!(
            "script source is {} bytes, over the {}-byte limit",
            def.script.len(),
            sandbox::MAX_SCRIPT_BYTES
        )));
    }

    // 3. Build the sandboxed engine over a fresh, isolated host and seed the active
    //    technology, so runs never leak state into one another.
    let host = crate::host::shared_host();
    host.borrow_mut().set_technology(tech.clone());
    let engine = sandbox::build_engine(&host, limits);

    // 4. Inject each schema parameter as a scope variable, then run the script.
    let mut scope = Scope::new();
    sandbox::inject_params(&mut scope, def, params);
    engine
        .run_with_scope(&mut scope, &def.script)
        .map_err(|e| sandbox::classify_run_error(*e))?;

    // 5. Definitive output-cap check on the final document (the progress callback is the
    //    fail-fast guard during the run; this is authoritative afterwards).
    let doc = host.borrow().snapshot();
    if let Some(msg) = sandbox::over_output_caps(&doc, limits.max_shapes, limits.max_cells) {
        return Err(ProduceError::LimitExceeded(msg));
    }

    // 6. Extract the produced top cell and stamp its F2 provenance.
    let cell = sandbox::extract_top_cell(&doc)?;
    Ok((cell, def.produce_meta(params)))
}

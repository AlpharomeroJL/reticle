//! The sandbox mechanism: a hardened rhai [`Engine`], parameter injection, output
//! accounting, top-cell extraction, and error classification.
//!
//! This is the security core of the PCell producer. Every function here is written so that
//! *any* script or parameter input fails cleanly (a [`ProduceError`]) and never panics,
//! hangs, reads the filesystem, or exhausts memory.

use rhai::module_resolvers::DummyModuleResolver;
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Scope};
use serde_json::Value;

use reticle_gen::PCellDef;
use reticle_model::{Cell, Document};

use super::{ProduceError, SandboxLimits};
use crate::api;
use crate::host::SharedHost;

/// Largest string a script may build. Bounds exponential growth (`s += s` doubles memory per
/// operation, which the operation cap does not catch) without constraining any realistic
/// generator.
const MAX_STRING_BYTES: usize = 1_000_000;
/// Largest array a script may build (same exponential-growth reasoning as strings).
const MAX_ARRAY_ELEMS: usize = 1_000_000;
/// Largest object map a script may build.
const MAX_MAP_ENTRIES: usize = 100_000;
/// Largest script source accepted. A cheap pre-parse guard: parsing cost is bounded by source
/// length and is not covered by the runtime operation cap.
pub(super) const MAX_SCRIPT_BYTES: usize = 1_000_000;
/// Deepest `params` nesting the producer accepts. A hostile deeply nested parameter is
/// rejected up front ([`exceeds_param_depth`]) so nothing downstream recurses over it: neither
/// injection here nor the [`param_hash`](reticle_gen::PCellDef::param_hash) canonicalization on
/// the success path can overflow the native stack (which would abort the tab). Schema fields
/// are scalars, so this never rejects a well-formed parameter set.
pub(super) const MAX_PARAM_DEPTH: u32 = 32;
/// Hard recursion backstop inside [`json_to_dynamic`], above [`MAX_PARAM_DEPTH`] so it never
/// alters an accepted parameter, only guards against a bypass of the up-front depth reject.
const JSON_CONVERT_DEPTH_CAP: u32 = 128;
/// How often (in operations) the progress callback recomputes the output counts. Coarse so
/// the check is negligible; the definitive cap check runs once after the script finishes.
const OUTPUT_CHECK_INTERVAL: u64 = 1 << 14;

/// Builds the hardened engine: the full Reticle API, no filesystem/module access, and every
/// rhai resource limit set so a runaway or hostile script is rejected rather than hanging or
/// exhausting memory.
///
/// The returned engine shares `host` (its registered functions mutate the host document, and
/// the progress callback reads it to enforce the output caps).
pub(super) fn build_engine(host: &SharedHost, limits: SandboxLimits) -> Engine {
    let mut engine = Engine::new();

    // The create/edit/query/transform/DRC/export API. None of these functions touch the
    // filesystem (DRC and export operate purely on in-memory data), so reusing the shared
    // registration is sound for a sandbox.
    api::register_api(&mut engine, host);

    // Neutralize module resolution. `Engine::new` installs a `FileModuleResolver` on native
    // targets, so `import "x"` would read `x.rhai` from disk. The dummy resolver turns every
    // `import` into a clean `ErrorModuleNotFound`: no disk read, no host escape.
    engine.set_module_resolver(DummyModuleResolver::new());

    // Bound rhai value growth so exponential doubling cannot exhaust memory in a few
    // operations (the operation cap does not catch this).
    engine.set_max_string_size(MAX_STRING_BYTES);
    engine.set_max_array_size(MAX_ARRAY_ELEMS);
    engine.set_max_map_size(MAX_MAP_ENTRIES);

    // The operation cap and the output caps, enforced from one progress callback. A runaway
    // loop is aborted the moment its operation count crosses the bound (returning a sentinel
    // that classifies to `LimitExceeded`), so a hostile script can never hang the caller.
    let progress_host = host.clone();
    let max_ops = limits.max_operations;
    let max_shapes = limits.max_shapes;
    let max_cells = limits.max_cells;
    engine.on_progress(move |count| {
        if count > max_ops {
            return Some(Dynamic::from(format!(
                "operation limit exceeded ({max_ops} operations)"
            )));
        }
        // Coarse-grained fail-fast output check on an interval. `try_borrow` never blocks or
        // panics: if the host is momentarily borrowed the check is simply skipped this tick,
        // and the authoritative cap check still runs after the script finishes.
        if count % OUTPUT_CHECK_INTERVAL != 0 {
            return None;
        }
        let Ok(h) = progress_host.try_borrow() else {
            return None;
        };
        over_output_caps(h.document(), max_shapes, max_cells).map(Dynamic::from)
    });

    engine
}

/// Injects each schema field's value into `scope` under the field's name, so the script
/// references its parameters as plain variables (mirroring the leading `let` block of a
/// hand-written generator script).
///
/// Callers pass the `PCellDef::effective_params` object, in which every schema field already
/// carries its provided-or-default value, so each declared parameter is always bound. Only schema
/// fields drive the script (and only schema fields form the parameter-hash identity), so an
/// unrelated key neither introduces a script variable nor changes the identity.
pub(super) fn inject_params(scope: &mut Scope, def: &PCellDef, params: &Value) {
    for field in &def.schema.fields {
        let value = params.get(&field.name).unwrap_or(&field.default);
        scope.push_dynamic(field.name.clone(), json_to_dynamic(value, 0));
    }
}

/// Whether `params` nests deeper than [`MAX_PARAM_DEPTH`].
///
/// The producer rejects such a parameter set up front so nothing downstream recurses over it:
/// not this module's [`json_to_dynamic`], and not `reticle_gen`'s param-hash canonicalization
/// on the success path. A caller can hand-build a `serde_json::Value` deep enough to overflow
/// the stack simply by *dropping* it, so bounding it at the boundary is the only safe policy.
///
/// The check itself descends at most [`MAX_PARAM_DEPTH`] frames: it stops and returns `true`
/// the moment it goes past the budget, so it cannot overflow on the very input it guards.
pub(super) fn exceeds_param_depth(params: &Value) -> bool {
    fn descend(value: &Value, budget: u32) -> bool {
        match value {
            Value::Array(items) => budget == 0 || items.iter().any(|v| descend(v, budget - 1)),
            Value::Object(fields) => budget == 0 || fields.values().any(|v| descend(v, budget - 1)),
            _ => false,
        }
    }
    descend(params, MAX_PARAM_DEPTH)
}

/// Converts a JSON value to a rhai [`Dynamic`], bounding recursion depth so a hostile deeply
/// nested parameter cannot overflow the native stack. Past [`JSON_CONVERT_DEPTH_CAP`] the
/// subtree becomes unit; an integer that does not fit `i64` falls back to `f64`. In normal use
/// the up-front [`exceeds_param_depth`] reject keeps inputs well under the cap.
fn json_to_dynamic(value: &Value, depth: u32) -> Dynamic {
    if depth >= JSON_CONVERT_DEPTH_CAP {
        return Dynamic::UNIT;
    }
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(*b),
        Value::Number(n) => n
            .as_i64()
            .map(Dynamic::from)
            .or_else(|| n.as_f64().map(Dynamic::from))
            .unwrap_or(Dynamic::UNIT),
        Value::String(s) => Dynamic::from(s.clone()),
        Value::Array(items) => {
            let arr: Array = items
                .iter()
                .map(|v| json_to_dynamic(v, depth + 1))
                .collect();
            Dynamic::from(arr)
        }
        Value::Object(fields) => {
            let mut map = Map::new();
            for (k, v) in fields {
                map.insert(k.as_str().into(), json_to_dynamic(v, depth + 1));
            }
            Dynamic::from(map)
        }
    }
}

/// The number of stored output records across all cells: authored shapes plus the instance
/// and array *placement* records.
///
/// This counts the script's *authored* records, not the flattened expansion of arrays. Counting
/// the flattened expansion would let a script with a large array multiplier (a small `add_array`
/// with billions of repetitions) force an enormous allocation during produce, which is exactly
/// the exhaustion the caps exist to prevent; flattening a hierarchy is the consumer's concern
/// and has its own guards. But each `add_instance`/`add_array` call stores a real, permanent
/// record (heap-allocated, and duplicated into the undo log), so those records must be bounded
/// too: a script that loops storing 10,000 instance or array records is exactly the "how much
/// output" exhaustion `max_shapes` exists to reject, and counting only `shapes` left a hole.
pub(super) fn stored_output_count(doc: &Document) -> usize {
    doc.cells()
        .map(|c| c.shapes.len() + c.instances.len() + c.arrays.len())
        .sum()
}

/// Returns a message naming the first output cap `doc` exceeds, or `None` if it is within
/// both the cell and output-record caps.
pub(super) fn over_output_caps(
    doc: &Document,
    max_shapes: usize,
    max_cells: usize,
) -> Option<String> {
    let cells = doc.cell_count();
    if cells > max_cells {
        return Some(format!(
            "cell limit exceeded ({cells} cells, limit {max_cells})"
        ));
    }
    let records = stored_output_count(doc);
    if records > max_shapes {
        return Some(format!(
            "output limit exceeded ({records} stored shape/instance/array records, limit {max_shapes})"
        ));
    }
    None
}

/// Extracts the produced top cell: the first declared top cell that resolves to an existing
/// cell, cloned out of the document.
///
/// # Errors
///
/// Returns [`ProduceError::NoTopCell`] if the script declared no top cell (never called
/// `set_top_cells`), or none of the declared names name an existing cell.
pub(super) fn extract_top_cell(doc: &Document) -> Result<Cell, ProduceError> {
    for name in doc.top_cells() {
        if let Some(cell) = doc.cell(name) {
            return Ok(cell.clone());
        }
    }
    Err(ProduceError::NoTopCell)
}

/// Classifies a rhai evaluation failure into a [`ProduceError`], never panicking.
///
/// Every resource-exhaustion error (operation cap, the output-cap sentinel, a string/array/map
/// size cap, call-depth overflow, or the module-import cap) becomes
/// [`ProduceError::LimitExceeded`] with a message naming the bound. Every other failure (a
/// syntax error, a rejected model edit, a call to a missing or blocked function, a blocked
/// `import`) becomes [`ProduceError::Script`].
pub(super) fn classify_run_error(err: EvalAltResult) -> ProduceError {
    match err {
        // The output-cap / operation-cap sentinel returned by the progress callback. The
        // token is the human-readable message we put there.
        EvalAltResult::ErrorTerminated(token, _) => ProduceError::LimitExceeded(
            token
                .into_string()
                .unwrap_or_else(|_| "sandbox limit exceeded".to_owned()),
        ),
        EvalAltResult::ErrorTooManyOperations(_) => {
            ProduceError::LimitExceeded("operation limit exceeded".to_owned())
        }
        EvalAltResult::ErrorDataTooLarge(what, _) => {
            ProduceError::LimitExceeded(format!("{what} exceeded its size limit"))
        }
        EvalAltResult::ErrorStackOverflow(_) => {
            ProduceError::LimitExceeded("call-depth limit exceeded".to_owned())
        }
        EvalAltResult::ErrorTooManyModules(_) => {
            ProduceError::LimitExceeded("module-import limit exceeded".to_owned())
        }
        other => ProduceError::Script(other.to_string()),
    }
}

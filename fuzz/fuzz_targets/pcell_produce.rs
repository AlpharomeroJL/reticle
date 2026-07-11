#![no_main]
//! Fuzz the sandboxed PCell producer. Building a [`PCellDef`] from arbitrary bytes (an
//! untrusted rhai script and untrusted JSON parameters) and producing it must never panic,
//! hang, or exhaust memory: it either returns a cell or a structured `ProduceError`.
//!
//! Input layout: the bytes up to the first NUL are the rhai script; the bytes after it are
//! parsed as JSON parameters (an unparsable or absent tail becomes an empty object). The
//! operation/output limits are deliberately small so every iteration terminates quickly while
//! still exercising each rejection path (op cap, output caps, memory caps, param-depth reject,
//! blocked `import`, malformed source, no top cell).

use libfuzzer_sys::fuzz_target;

use reticle_gen::{FieldSchema, PCellDef, ParamSchema};
use reticle_model::Technology;
use reticle_script::{SandboxLimits, produce};
use serde_json::{Value, json};

/// A representative schema whose field names cover the seed scripts, so parameter injection by
/// field name is exercised (unlisted params still affect the identity hash).
fn fuzz_schema() -> ParamSchema {
    let int = |name: &str, default: i64| {
        FieldSchema::int(name, name, default, -1_000_000, 1_000_000, "dbu")
    };
    ParamSchema {
        generator_id: "fuzz.pcell".to_owned(),
        title: "fuzz".to_owned(),
        description: "fuzz".to_owned(),
        fields: vec![
            int("w", 400),
            int("pixel_w", 800),
            int("pixel_h", 800),
            int("via", 200),
            int("columns", 8),
            int("rows", 6),
            int("pitch_x", 1000),
            int("pitch_y", 1000),
        ],
    }
}

fuzz_target!(|data: &[u8]| {
    // Split the input into a script and a JSON parameter tail at the first NUL byte.
    let (script_bytes, params_bytes) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let script = String::from_utf8_lossy(script_bytes).into_owned();
    // serde_json enforces its own recursion limit, so even a pathological tail parses to an
    // error (never a stack overflow); fall back to an empty object then.
    let params: Value = serde_json::from_slice(params_bytes).unwrap_or_else(|_| json!({}));

    let def = PCellDef {
        id: "fuzz.pcell".to_owned(),
        title: "fuzz".to_owned(),
        description: "fuzz".to_owned(),
        schema: fuzz_schema(),
        script,
        engine_version: "8.2.0".to_owned(),
    };

    let limits = SandboxLimits {
        max_operations: 100_000,
        max_shapes: 20_000,
        max_cells: 500,
    };

    // The producer must swallow every input as a clean result; a panic here is a finding.
    let _ = produce(&def, &params, &Technology::default(), limits);
});

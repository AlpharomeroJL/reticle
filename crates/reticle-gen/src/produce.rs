//! The F2 produce-metadata contract: the provenance of a produced generator/`PCell`
//! instance, and the canonical input to its parameter hash.
//!
//! The existing [`ParamSchema`](crate::ParamSchema) is the form schema the Inspector
//! renders and the model tool advertises; F2 adds the *provenance* a produced instance
//! carries so a regenerate is reproducible: which generator, which engine version, the
//! optional `.rhai` script reference for a user `PCell`, and a stable `param_hash`.
//!
//! # The param-hash recipe
//!
//! The `param_hash` is `SHA-256` over
//! `generator_id + "\n" + engine_version + "\n" + canonical_params_json(params)`, rendered
//! lowercase hex. This module ships the deterministic input ([`canonical_params_json`], a
//! sorted-key compact JSON so two parameter sets that differ only in key order hash the
//! same); the hash itself is applied by
//! [`param_hash`](crate::param_hash) in the PCell module (the Phase-2 scaffolding
//! front-loaded that primitive so every PCell lane keys on one tested implementation; ADR
//! 0107). The hash keys the instance cache and is the
//! regenerate identity: same generator, engine, and params means the same hash.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The provenance of one produced generator/`PCell` instance.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProduceMeta {
    /// The registry id of the generator (or user `PCell`) that produced the instance.
    pub generator_id: String,
    /// The generation engine version that produced it, part of the hash so a produce from
    /// a different engine version has a different identity.
    pub engine_version: String,
    /// The `.rhai` script reference for a user `PCell`, or `None` for a built-in generator.
    pub script_ref: Option<String>,
    /// The canonical parameter hash (lowercase-hex SHA-256; see the module recipe). Keys
    /// the instance cache and identifies a regenerate.
    pub param_hash: String,
}

impl ProduceMeta {
    /// Whether `param_hash` is a well-formed 64-char lowercase-hex digest.
    #[must_use]
    pub fn has_valid_hash(&self) -> bool {
        is_sha256_hex(&self.param_hash)
    }
}

/// The canonical JSON of a parameter set: object keys sorted recursively, compact, so it is
/// a stable hash input regardless of the key order the params were built with.
///
/// This is the deterministic half of the [`ProduceMeta`] param-hash recipe; the hash itself
/// (SHA-256 over the id, engine version, and this string) is applied by the `pcell-params`
/// lane. Returns an empty string only if `params` cannot serialize, which a JSON `Value`
/// never does in practice.
#[must_use]
pub fn canonical_params_json(params: &Value) -> String {
    serde_json::to_string(&canonicalize(params)).unwrap_or_default()
}

/// Rebuilds `v` with every object's keys sorted, recursively. Independent of `serde_json`'s
/// `preserve_order` feature: sorted keys are inserted in sorted order (a `BTreeMap`-backed
/// map is already sorted; an insertion-ordered map is made sorted here).
fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::with_capacity(map.len());
            for k in keys {
                out.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        scalar => scalar.clone(),
    }
}

/// Whether `s` is a 64-character lowercase hex string (a SHA-256 digest).
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::{canonical_params_json, is_sha256_hex};
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys_and_is_order_independent() {
        let a = json!({ "region_width": 2000, "layer": "li1" });
        let b = json!({ "layer": "li1", "region_width": 2000 });
        assert_eq!(canonical_params_json(&a), canonical_params_json(&b));
        assert_eq!(
            canonical_params_json(&json!({"b": 1, "a": 2})),
            r#"{"a":2,"b":1}"#
        );
        // Nested objects are canonicalized too.
        assert_eq!(
            canonical_params_json(&json!({ "z": { "b": 1, "a": 2 }, "a": 3 })),
            r#"{"a":3,"z":{"a":2,"b":1}}"#
        );
    }

    #[test]
    fn hash_shape_check() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex("ABC")); // uppercase and wrong length
        assert!(!is_sha256_hex(&"g".repeat(64))); // non-hex
    }
}

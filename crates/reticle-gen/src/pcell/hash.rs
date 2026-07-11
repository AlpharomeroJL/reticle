//! The PCell parameter hash: the stable content identity of a produced instance.
//!
//! Implements the F2 recipe documented in the `produce` module: `SHA-256` over
//! `generator_id + "\n" + engine_version + "\n" + canonical_params_json(params)`,
//! rendered lowercase hex.
//!
//! This primitive is *front-loaded* into the Phase-2 scaffolding rather than authored by
//! the `pcell-params` lane (which the F2 module doc originally anticipated), because every
//! PCell lane keys on this identity: the sandboxed producer stamps it into
//! [`ProduceMeta`](crate::ProduceMeta), the cache keys on it, and the harness asserts it is
//! deterministic. Making the parallel lanes agree on one implementation, tested and frozen
//! here, is exactly the shared primitive the fan-out needs. See ADR 0107.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::produce::canonical_params_json;

/// The canonical parameter hash for a produced generator/PCell instance: lowercase-hex
/// `SHA-256` over the generator id, engine version, and canonical params JSON (the F2
/// recipe in the `produce` module).
///
/// Deterministic and key-order independent (the params are canonicalized first), so the
/// same logical parameters always hash the same regardless of how the JSON was built. A
/// different id, engine version, or any parameter value changes the digest.
#[must_use]
pub fn param_hash(generator_id: &str, engine_version: &str, params: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(generator_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(engine_version.as_bytes());
    hasher.update(b"\n");
    hasher.update(canonical_params_json(params).as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        // Two lowercase hex nibbles per byte; the mapping is total, no allocation beyond
        // the pre-sized string.
        const NIBBLES: &[u8; 16] = b"0123456789abcdef";
        hex.push(NIBBLES[(byte >> 4) as usize] as char);
        hex.push(NIBBLES[(byte & 0x0f) as usize] as char);
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::param_hash;
    use serde_json::json;

    #[test]
    fn hash_is_64_char_lowercase_hex_and_deterministic() {
        let h = param_hash("via_farm", "8.2.0", &json!({"rows": 3, "cols": 4}));
        assert_eq!(h.len(), 64, "a SHA-256 digest is 64 hex chars");
        assert!(
            h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "lowercase hex only: {h}"
        );
        assert_eq!(
            h,
            param_hash("via_farm", "8.2.0", &json!({"rows": 3, "cols": 4})),
            "deterministic for identical inputs"
        );
    }

    #[test]
    fn hash_is_key_order_independent_but_input_sensitive() {
        // Canonicalization sorts keys, so key order does not change the identity.
        let a = param_hash("g", "1", &json!({"a": 1, "b": 2}));
        let b = param_hash("g", "1", &json!({"b": 2, "a": 1}));
        assert_eq!(a, b, "key order must not change the hash");

        // Every component of the recipe is part of the identity.
        assert_ne!(a, param_hash("g", "1", &json!({"a": 1, "b": 3})), "value");
        assert_ne!(a, param_hash("h", "1", &json!({"a": 1, "b": 2})), "id");
        assert_ne!(a, param_hash("g", "2", &json!({"a": 1, "b": 2})), "engine");
    }

    #[test]
    fn empty_inputs_still_hash_to_valid_hex() {
        // The recipe is total: even empty id/version/params produce a well-formed digest
        // (the canonical JSON of null is "null", so the input is "\n\nnull").
        let h = param_hash("", "", &serde_json::Value::Null);
        assert_eq!(h.len(), 64);
        assert!(
            h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    // NOTE (pcell-harness lane): add a pinned known-answer vector here (compute the real
    // digest for a fixed input once and freeze it) so a future change to the recipe cannot
    // silently alter the produced-instance identity. Omitted from the scaffold rather than
    // shipped with a fabricated value.
}

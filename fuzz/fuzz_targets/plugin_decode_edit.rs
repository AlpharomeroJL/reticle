#![no_main]
//! Fuzz the v0 plugin edit decoder: `decode_edit_v0` turns untrusted guest bytes
//! (the `stage_edit` payload) into a `reticle_model::Edit`. It must never panic,
//! hang, or over-allocate on arbitrary input; it either returns an `Edit` or a
//! structured `EditDecodeError`. A panic here would abort the host's wasm instance
//! and kill the browser tab, so panic-freedom is the invariant under test.
//!
//! The decoder is native-only (`cfg(not(target_arch = "wasm32"))`), like the whole
//! plugin host; this target builds and runs under the native fuzz toolchain (WSL).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The cap mirrors `Limits::default().max_query_len`. The decoder must stay
    // panic-free for any cap; a fixed value keeps the corpus reproducible and
    // exercises both the name-length cap and the count-against-remaining paths.
    let _ = reticle_plugin::decode_edit_v0(data, 256);
});

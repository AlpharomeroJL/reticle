#![no_main]
//! Fuzz the GDSII importer: it must never panic, hang, or exhibit UB on arbitrary
//! bytes; it either returns a document or an error.

use libfuzzer_sys::fuzz_target;
use reticle_model::Importer;

fuzz_target!(|data: &[u8]| {
    let _ = reticle_io::Gds.import(data);
});

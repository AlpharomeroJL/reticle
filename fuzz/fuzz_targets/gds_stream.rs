#![no_main]
//! Fuzz the forward-only GDSII record reader: driving
//! [`GdsRecordReader::next_event`](reticle_io::GdsRecordReader::next_event) to
//! exhaustion over arbitrary bytes must never panic, hang, or exhibit UB; it either
//! yields events until end of stream or returns an error.
//!
//! The seed corpus (`fuzz/corpus/gds_stream/`) includes the committed GDS crash
//! fixtures (out-of-range dates, zero-length string records) so the streaming path
//! cannot reintroduce a fixed panic class.

use libfuzzer_sys::fuzz_target;
use reticle_io::GdsRecordReader;

fuzz_target!(|data: &[u8]| {
    let mut reader = GdsRecordReader::new(data);
    // Each event consumes at least one four-byte record from the finite input, so this
    // terminates; a bug that failed to advance would trip the fuzzer's timeout.
    while let Ok(Some(_)) = reader.next_event() {}
});

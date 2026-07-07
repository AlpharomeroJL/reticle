//! Reticle's round-trip driver for the interop harness.
//!
//! Usage:
//!   reticle-roundtrip gds       <in.gds> <out.gds>   import a GDS, re-export as GDS
//!   reticle-roundtrip oasis-std <in.gds> <out.oas>   import a GDS, export conformant
//!                                                     OASIS (oasis_std) for KLayout
//!
//! The `gds` mode is Reticle's own GDS round-trip, compared against KLayout and gdspy
//! by the harness. The `oasis-std` mode emits the conformant-OASIS-subset writer's
//! output so KLayout can attempt to read it. Both read the same fixture the other
//! tools read, so any divergence is attributable to Reticle's reader/writer.

use std::process::ExitCode;

use reticle_io::Gds;
use reticle_model::{Exporter, Importer};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!(
            "usage: {} <gds|oasis-std> <in.gds> <out>",
            args.first()
                .map(String::as_str)
                .unwrap_or("reticle-roundtrip")
        );
        return ExitCode::from(2);
    }
    let (mode, input, output) = (&args[1], &args[2], &args[3]);

    let bytes = match std::fs::read(input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let doc = match Gds.import(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: Reticle failed to import {input}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let out_bytes = match mode.as_str() {
        "gds" => Gds.export(&doc),
        "oasis-std" => reticle_io::OasisStd.export(&doc),
        other => {
            eprintln!("error: unknown mode {other:?} (expected `gds` or `oasis-std`)");
            return ExitCode::from(2);
        }
    };

    let out_bytes = match out_bytes {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: Reticle failed to export ({mode}): {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = std::fs::write(output, &out_bytes) {
        eprintln!("error: cannot write {output}: {e}");
        return ExitCode::FAILURE;
    }

    eprintln!(
        "ok: {} -> {} ({} bytes, {} cells)",
        input,
        output,
        out_bytes.len(),
        doc.cells().count()
    );
    ExitCode::SUCCESS
}

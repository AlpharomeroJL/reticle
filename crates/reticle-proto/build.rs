//! Build script for `reticle-proto`.
//!
//! Generates Rust types from `proto/reticle.proto` with `prost-build`, driving
//! the `protoc` compiler shipped by `protoc-bin-vendored` (ADR 0008) so the
//! build needs no system-installed `protoc`. The generated code lands in
//! `OUT_DIR` and is pulled into the crate via `include!` from the `v1` module.

use std::io::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    // Rebuild whenever the frozen schema changes.
    println!("cargo:rerun-if-changed=proto/reticle.proto");

    // Locate the vendored `protoc` binary for the host platform.
    let protoc: PathBuf = protoc_bin_vendored::protoc_bin_path()
        .expect("protoc-bin-vendored has no protoc for this platform");

    // Compile the schema into `$OUT_DIR/reticle.v1.rs` (named after the
    // `reticle.v1` package). `proto/` is the include root for imports.
    prost_build::Config::new()
        .protoc_executable(protoc)
        .compile_protos(&["proto/reticle.proto"], &["proto"])?;

    Ok(())
}

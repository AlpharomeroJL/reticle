//! The Reticle WebAssembly harness.
//!
//! Wave 4 mounts the `egui` application in the browser, runs a WebGPU capability
//! check, and falls back to WebGL2 with a clear message where WebGPU is
//! unavailable (ADR 0009). Trunk builds this crate to `wasm32-unknown-unknown`.
//!
//! Native builds of this crate are a no-op so `cargo build --workspace` on the
//! host stays green; the real entry point is `cfg`-gated to wasm in Wave 4.

fn main() {
    // Wave 4 (wasm): initialize the canvas, select WebGPU or WebGL2, run the app.
}

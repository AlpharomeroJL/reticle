//! Native launcher for the Reticle application.
//!
//! Builds the [`reticle_app::App`] and runs it in an `eframe` window. The whole
//! launcher is gated on non-wasm: on `wasm32` the app is mounted by the separate
//! `web` crate instead, and `eframe::run_native` (and the winit event loop it
//! drives) does not exist there.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use reticle_app::App;

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Reticle")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Reticle",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

/// On wasm the binary target is never the entry point (the `web` crate mounts the
/// library), so `main` is an empty stub to keep the crate building for wasm.
#[cfg(target_arch = "wasm32")]
fn main() {}

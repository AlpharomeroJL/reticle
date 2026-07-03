//! Native launcher for the Reticle application.
//!
//! Builds the [`reticle_app::App`] and runs it in an `eframe` window. The whole
//! launcher is gated on non-wasm: on `wasm32` the app is mounted by the separate
//! `web` crate instead, and `eframe::run_native` (and the winit event loop it
//! drives) does not exist there.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use reticle_app::App;

    let args: Vec<String> = std::env::args().skip(1).collect();
    // Native-only scripted-capture flag. `--screenshot-smoke <path>` loads the editor
    // on the bundled SKY130 cell, captures one full-window screenshot to `<path>`, and
    // exits; it de-risks the egui viewport-screenshot path before the full demo-script
    // harness is built on top of it. Capture runs use a larger 1600x1000 window for
    // legible media.
    let smoke = flag_value(&args, "--screenshot-smoke");
    let capture_mode = smoke.is_some();

    let inner_size = if capture_mode {
        [1600.0, 1000.0]
    } else {
        [1280.0, 800.0]
    };

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Reticle")
            .with_inner_size(inner_size)
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Reticle",
        native_options,
        Box::new(move |_cc| {
            let mut app = App::new();
            if let Some(path) = smoke {
                app.set_screenshot_smoke(std::path::PathBuf::from(path));
            }
            Ok(Box::new(app))
        }),
    )
}

/// Returns the value following `flag` in `args`, if present.
#[cfg(not(target_arch = "wasm32"))]
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// On wasm the binary target is never the entry point (the `web` crate mounts the
/// library), so `main` is an empty stub to keep the crate building for wasm.
#[cfg(target_arch = "wasm32")]
fn main() {}

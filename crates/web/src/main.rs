//! The Reticle WebAssembly harness.
//!
//! Trunk builds this crate to `wasm32-unknown-unknown` and `index.html` mounts it.
//! The wasm entry point starts the `eframe`/`egui` application on the page canvas;
//! `eframe`'s wgpu backend uses WebGPU where available and falls back to WebGL2
//! (ADR 0009). Native builds are a no-op so the workspace build stays green.

/// wasm entry point: start the egui app on the `#reticle-canvas` element.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document on window");
        let canvas = document
            .get_element_by_id("reticle-canvas")
            .expect("index.html is missing a #reticle-canvas element")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#reticle-canvas is not a <canvas>");

        // Hide the loading overlay once we take over the canvas.
        if let Some(overlay) = document.get_element_by_id("overlay") {
            let _ = overlay.set_attribute("style", "display:none");
        }

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|_cc| Ok(Box::new(reticle_app::App::new()))),
            )
            .await
            .expect("failed to start the Reticle web app");
    });
}

/// Native builds of the harness do nothing; the desktop application is the
/// `reticle-app` binary.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}

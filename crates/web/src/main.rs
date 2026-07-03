//! The Reticle WebAssembly harness.
//!
//! Trunk builds this crate to `wasm32-unknown-unknown` and `index.html` mounts it.
//! The wasm entry point starts the `eframe`/`egui` application on the page canvas;
//! `eframe`'s wgpu backend uses WebGPU where available and falls back to WebGL2
//! (ADR 0009). Native builds are a no-op so the workspace build stays green.
//!
//! # Start view
//!
//! A public visitor lands on the replay theater by default (ADR 0026); the page URL
//! selects the view via a `?view=` query parameter: `?view=editor` opens the full
//! editor, `?view=replay` (or an absent parameter, the published default) opens the
//! replay theater. The choice is passed to `reticle_app::App::with_start_view`
//! (not an intra-doc link: `reticle-app` is a wasm-only dependency of this crate,
//! so the symbol is out of scope when the workspace docs build on the host).

/// wasm entry point: start the egui app on the `#reticle-canvas` element.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    let web_options = eframe::WebOptions::default();
    let start_view = start_view_from_url();

    wasm_bindgen_futures::spawn_local(async move {
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
                Box::new(move |_cc| Ok(Box::new(reticle_app::App::with_start_view(start_view)))),
            )
            .await
            .expect("failed to start the Reticle web app");
    });
}

/// Reads the `?view=` query parameter and maps it to a `reticle_app::StartView`.
///
/// An absent parameter defaults to `StartView::ReplayTheater`, so the published
/// public bundle opens to the theater; `?view=editor` opens the editor.
#[cfg(target_arch = "wasm32")]
fn start_view_from_url() -> reticle_app::StartView {
    use reticle_app::StartView;

    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    // `search` is like "?view=editor"; UrlSearchParams accepts the leading '?'.
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else {
        return StartView::ReplayTheater;
    };
    match params.get("view") {
        Some(value) => StartView::from_query_value(&value),
        // No explicit view: the public default is the replay theater.
        None => StartView::ReplayTheater,
    }
}

/// Native builds of the harness do nothing; the desktop application is the
/// `reticle-app` binary.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}

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
        let Some(canvas) = document
            .get_element_by_id("reticle-canvas")
            .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            set_overlay_error(&document, "the page is missing its #reticle-canvas element");
            return;
        };

        // Start the renderer. Only hide the loading overlay AFTER start resolves
        // Ok, i.e. the wgpu backend (WebGPU or its WebGL2 fallback) initialized on
        // the canvas. On Err we surface a visible message rather than panicking
        // into a blank canvas or leaving the spinner up forever.
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(move |_cc| Ok(Box::new(reticle_app::App::with_start_view(start_view)))),
            )
            .await;

        match result {
            Ok(()) => {
                if let Some(overlay) = document.get_element_by_id("overlay") {
                    let _ = overlay.set_attribute("style", "display:none");
                }
            }
            Err(err) => {
                set_overlay_error(&document, &format!("{err:?}"));
            }
        }
    });
}

/// Writes a visible error into the `#status` element (and keeps the overlay up)
/// so a start failure is reported to the visitor instead of a silent spinner or a
/// blank canvas. Mirrors the wording used by the failure handler in `index.html`.
#[cfg(target_arch = "wasm32")]
fn set_overlay_error(document: &web_sys::Document, message: &str) {
    if let Some(status) = document.get_element_by_id("status") {
        status.set_class_name("status error");
        status.set_text_content(Some(&format!(
            "Failed to load Reticle: {message}. Check the console."
        )));
    }
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
    // A read-only viewer link (`?view=viewer&room=...&relay=...`, ADR 0038) opens
    // the editor chrome in read-only mode: the viewer applies the sharer's live
    // frames but never publishes. Recognize and report the target here so the
    // entry acknowledges a viewer link; the live socket-pumping into a
    // `reticle_app::viewer::ViewerSession` is proven by the Wave 1 end-to-end gate.
    if let Some(target) = viewer_target_from_search(&search) {
        web_sys::console::log_1(
            &format!(
                "reticle: read-only viewer link for room '{}' on relay '{}'",
                target.room, target.relay
            )
            .into(),
        );
        return StartView::Editor;
    }

    match params.get("view") {
        Some(value) => StartView::from_query_value(&value),
        // No explicit view: the public default is the replay theater.
        None => StartView::ReplayTheater,
    }
}

/// Recovers the read-only viewer target (room + relay) from the page's query
/// string, or `None` when this is not a viewer link.
///
/// Delegates to `reticle_app::share::parse_viewer_query`, the same pure parser the
/// desktop Share section's viewer link is built with, so the page and the link
/// agree on the format.
#[cfg(target_arch = "wasm32")]
fn viewer_target_from_search(search: &str) -> Option<reticle_app::share::ViewerTarget> {
    reticle_app::share::parse_viewer_query(search)
}

/// Native builds of the harness do nothing; the desktop application is the
/// `reticle-app` binary.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}

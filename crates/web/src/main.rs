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
    let boot = boot_from_url();

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
                Box::new(move |_cc| {
                    let mut app = boot.into_app();
                    // A `?gds=` link may also carry a permalink (`?cell=`/`?view=x,y,z`/
                    // `?layers=`); stash it so it is applied once the document opens (a
                    // no-op for links without view-state params).
                    let search = web_sys::window()
                        .and_then(|w| w.location().search().ok())
                        .unwrap_or_default();
                    app.set_pending_permalink(reticle_app::share::parse_permalink(&search));
                    // `?embed=1` (lane 2D, catalog 94) hides all chrome for iframes; a
                    // modifier that composes with whatever view above was chosen.
                    app.set_embed(reticle_app::share::parse_embed(&search));
                    Ok(app)
                }),
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

/// How the bundle boots, decided from the page URL: a normal start view, or a
/// read-only viewer of a shared session.
#[cfg(target_arch = "wasm32")]
enum Boot {
    /// Open into a normal start view (editor or replay theater), per `?view=`.
    View {
        /// The start view selected by `?view=` (or the theater default).
        start_view: reticle_app::StartView,
        /// Whether `?e2e-autoplay=1` was set: start the replay theater playing on boot so
        /// a headed browser test can assert the wasm replay hash without clicking the
        /// GPU-painted transport (a no-op for the editor start view). The public landing
        /// still waits at Play.
        autoplay: bool,
    },
    /// Render the hidden component gallery full-window (`?gallery=1`, lane 1C): a
    /// deterministic screenshot surface over the theme's component library, used
    /// by the visual-regression suite. Carries no document or session state.
    Gallery,
    /// Browse a served `.rtla` archive streamed over HTTP-range, named by an `?archive=`
    /// link (lane v8-2e, ADR 0062). The bundle opens the archive on its first frame and
    /// paints the read-only streamed die with progressive residency.
    Archive(String),
    /// Open as a read-only viewer of the shared session named by a viewer link
    /// (`?view=viewer&room=..&relay=..`, ADR 0038/0058).
    Viewer(reticle_app::share::ViewerTarget),
    /// Boot straight into the guided tour (`?tour=1`, lane 4c / catalog 20): the
    /// editor opens with the tour running from its first step.
    Tour,
    /// Open the editor and go live automatically for `(relay, room)`, publishing the
    /// session for viewers without a manual "Go live" click. Triggered by a `?share=1`
    /// page flag; the browser share-live e2e uses it as the publisher context.
    Share {
        /// The relay host to publish to.
        relay: String,
        /// The room to publish into.
        room: String,
        /// Whether `?e2e-edit=1` was also set: after going live, place one scripted rect
        /// so lane v8-1e's browser test can observe the edit reach a viewer.
        e2e_edit: bool,
    },
}

#[cfg(target_arch = "wasm32")]
impl Boot {
    /// Constructs the `App` this boot describes.
    ///
    /// A [`Boot::Viewer`] builds the read-only viewer app, which on its first frame
    /// dials the relay room `?mode=view` and mirrors the sharer's live session; a
    /// [`Boot::Share`] builds the editor and goes live automatically; a [`Boot::View`]
    /// builds the ordinary editor/theater app.
    fn into_app(self) -> Box<reticle_app::App> {
        use reticle_app::{App, StartView};
        match self {
            Boot::View {
                start_view,
                autoplay,
            } => {
                let mut app = App::with_start_view(start_view);
                if autoplay {
                    app.set_replay_autoplay();
                }
                Box::new(app)
            }
            Boot::Gallery => Box::new(App::gallery()),
            Boot::Archive(url) => Box::new(App::with_archive(url)),
            Boot::Viewer(target) => Box::new(App::with_viewer(target)),
            Boot::Tour => Box::new(App::with_tour()),
            Boot::Share {
                relay,
                room,
                e2e_edit,
            } => {
                let mut app = App::with_share_on_boot(StartView::Editor, relay, room);
                app.set_e2e_edit(e2e_edit);
                Box::new(app)
            }
        }
    }
}

/// Reads the page URL and decides how the bundle boots.
///
/// A read-only viewer link (`?view=viewer&room=..&relay=..`, ADR 0038/0058) boots the
/// read-only viewer: it dials the relay room `?mode=view` and streams the sharer's live
/// document and presence, never publishing. Otherwise `?view=` selects the start view;
/// an absent parameter defaults to `StartView::ReplayTheater`, so the published public
/// bundle opens to the theater, and `?view=editor` opens the editor.
#[cfg(target_arch = "wasm32")]
fn boot_from_url() -> Boot {
    use reticle_app::StartView;

    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();

    // `?e2e-autoplay=1` starts the replay theater playing on boot so the headed replay
    // guard can drive playback without clicking the GPU-painted transport. Parsed once
    // here so every `?view=` path below carries it; it is a no-op for the editor view.
    let autoplay = reticle_app::share::parse_e2e_replay_autoplay(&search);

    // `?gallery=1` renders the hidden component gallery (lane 1C), a deterministic
    // screenshot surface for the visual-regression suite. Checked first so the dev
    // flag wins over the normal view selection; every other parameter is untouched.
    if let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search)
        && params
            .get("gallery")
            .is_some_and(|v| matches!(v.as_str(), "1" | "true"))
    {
        web_sys::console::log_1(&"reticle: render the component gallery".into());
        return Boot::Gallery;
    }

    // An `?archive=` link boots the served-archive browse: it streams a read-only
    // `.rtla` die rather than opening an editable document (lane v8-2e, ADR 0062).
    if let Some(url) = reticle_app::share::archive_url_from_query(&search) {
        web_sys::console::log_1(&format!("reticle: browse served archive '{url}'").into());
        return Boot::Archive(url);
    }

    // A viewer link wins: it boots the live read-only viewer transport (ADR 0058).
    if let Some(target) = viewer_target_from_search(&search) {
        web_sys::console::log_1(
            &format!(
                "reticle: read-only viewer link for room '{}' on relay '{}'",
                target.room, target.relay
            )
            .into(),
        );
        return Boot::Viewer(target);
    }

    // `search` is like "?view=editor"; UrlSearchParams accepts the leading '?'.
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else {
        return Boot::View {
            start_view: StartView::ReplayTheater,
            autoplay,
        };
    };

    // `?share=1&room=..&relay=..` boots the editor and goes live automatically (the
    // publisher side of the browser share-live e2e). Reuses the same room/relay query
    // keys the viewer link uses, so both contexts point at the same room.
    if params
        .get("share")
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "auto"))
    {
        let relay = params
            .get("relay")
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| reticle_app::share::DEFAULT_SERVER.to_owned());
        let room = params.get("room").unwrap_or_default();
        let room = reticle_app::share::room_id(&room);
        let e2e_edit = reticle_app::share::parse_e2e_edit(&search);
        web_sys::console::log_1(
            &format!("reticle: auto-share editor for room '{room}' on relay '{relay}'").into(),
        );
        return Boot::Share {
            relay,
            room,
            e2e_edit,
        };
    }

    // `?tour=1` boots straight into the guided tour (lane 4c / catalog 20). Checked
    // after the specialized links so a viewer/share/archive link still wins, but a
    // plain deep link (optionally with `?view=editor`) lands in the tour.
    if params
        .get("tour")
        .is_some_and(|v| matches!(v.as_str(), "1" | "true"))
    {
        web_sys::console::log_1(&"reticle: boot into the guided tour".into());
        return Boot::Tour;
    }

    match params.get("view") {
        Some(value) => Boot::View {
            start_view: StartView::from_query_value(&value),
            autoplay,
        },
        // No explicit view: the public default is the replay theater.
        None => Boot::View {
            start_view: StartView::ReplayTheater,
            autoplay,
        },
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

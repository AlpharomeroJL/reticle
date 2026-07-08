//! Native launcher for the Reticle application.
//!
//! Builds the [`reticle_app::App`] and runs it in an `eframe` window. The whole
//! launcher is gated on non-wasm: on `wasm32` the app is mounted by the separate
//! `web` crate instead, and `eframe::run_native` (and the winit event loop it
//! drives) does not exist there.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use reticle_app::App;
    use reticle_app::demoscript::Script;

    let args: Vec<String> = std::env::args().skip(1).collect();
    // Native-only scripted-capture flags (see `reticle_app::demoscript`):
    //   --screenshot-smoke <path>  one full-window screenshot to <path>, then exit
    //                              (de-risks the capture path).
    //   --demo-script <file>       play a committed demo script, capturing frames.
    //   --out <dir>                where captured frames + manifest are written.
    // Capture runs use a larger window (the script's viewport, else 1600x1000) so the
    // media is legible.
    // --gallery renders the hidden component gallery full-window (lane 1C), a
    // deterministic screenshot surface for the visual-regression suite.
    let gallery = args.iter().any(|a| a == "--gallery");
    // --tour boots straight into the guided tour (the native mirror of the `?tour=1`
    // web deep link, lane 4c / catalog 20).
    let tour = args.iter().any(|a| a == "--tour");
    let smoke = flag_value(&args, "--screenshot-smoke");
    let demo_path = flag_value(&args, "--demo-script");
    let out_dir = flag_value(&args, "--out").unwrap_or_else(|| "scratch/ui-frames".to_owned());

    // Parse the demo script up front so the window can be sized to it and a bad script
    // fails before any window opens.
    let demo = demo_path.as_deref().map(|path| {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("cannot read demo script {path}: {e}");
            std::process::exit(2);
        });
        Script::parse(&src).unwrap_or_else(|e| {
            eprintln!("demo script {path}: {e}");
            std::process::exit(2);
        })
    });

    let inner_size = match &demo {
        Some(script) => viewport_size(script),
        None if smoke.is_some() => [1600.0, 1000.0],
        None => [1280.0, 800.0],
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
            let mut app = if gallery {
                App::gallery()
            } else if tour {
                App::with_tour()
            } else {
                App::new()
            };
            if let Some(path) = smoke {
                app.set_screenshot_smoke(std::path::PathBuf::from(path));
            }
            if let Some(script) = demo {
                app.set_demo_script(script, std::path::PathBuf::from(out_dir));
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

/// The window inner size for a demo script, in logical pixels.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::cast_precision_loss)] // window dimensions are small and exact in f32
fn viewport_size(script: &reticle_app::demoscript::Script) -> [f32; 2] {
    [script.viewport.0 as f32, script.viewport.1 as f32]
}

/// On wasm the binary target is never the entry point (the `web` crate mounts the
/// library), so `main` is an empty stub to keep the crate building for wasm.
#[cfg(target_arch = "wasm32")]
fn main() {}

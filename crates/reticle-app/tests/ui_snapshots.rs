//! Automated visual-regression suite (v8.1 Wave 1, lane 1D).
//!
//! Every later interface gate diffs the UI mechanically against the committed
//! PNG baselines under `tests/snapshots/`. Two families of snapshot:
//!
//! * Gallery snapshots: one image per [`GalleryGroup`] per
//!   [`Density`](reticle_app::theme::tokens::Density), rendered through the frozen
//!   [`theme::gallery::ui`](reticle_app::theme::gallery::ui) function without
//!   booting the editor. These are the fast states (no layout canvas).
//! * Full-app snapshots: the real [`App`](reticle_app::App) driven through
//!   `eframe` at three window sizes, plus a palette-open interaction state.
//!
//! ## Renderer and the honest GPU skip
//!
//! `egui_kittest` 0.35 has no CPU rasterizer with the feature set this crate uses
//! (`wgpu`, `snapshot`, `eframe`): its default renderer is the wgpu backend, and
//! it is the *only* renderer available here. Rendering any snapshot, gallery
//! included, therefore needs a GPU adapter. So both families are gated on the
//! same adapter probe and SKIP honestly (a `println!` plus an early return) on
//! adapterless hosts, mirroring `reticle-render/tests/golden.rs`. The probe reuses
//! [`reticle_render::WgpuContext::new_blocking`], the house pattern, so the skip
//! decision matches the rest of the GPU test suite. (The lane brief expected the
//! gallery to need no GPU; that assumption does not hold for kittest 0.35 with
//! these features. Recorded as ADR-W1D in RESULT.md.)
//!
//! ADR 0094 makes GPU suites orchestrator-only at the integration gates; lane
//! development runs (capturing and checking baselines on a real GPU) are the
//! documented exception.
//!
//! ## Resolved versions and feature names
//!
//! * `egui_kittest = "0.35.0"` with features `["wgpu", "snapshot", "eframe"]`
//!   (verified exact against the resolved crate; `wgpu` pulls `egui-wgpu` +
//!   `pollster` + `wgpu`, `snapshot` pulls `dify` + `image`, `eframe` enables the
//!   `build_eframe` harness). Transitively: `kittest 0.4.0`, `dify 0.8.0`.
//! * Matches the `egui`/`eframe` 0.35.0 already in the workspace graph.
//!
//! ## Snapshot files and `UPDATE_SNAPSHOTS`
//!
//! For each `name`, kittest writes/reads `tests/snapshots/{name}.png` (the
//! committed baseline) and, on a mismatch or update, the transient
//! `{name}.new.png`, `{name}.diff.png`, `{name}.old.png` (all git-ignored).
//! `UPDATE_SNAPSHOTS` selects the mode (parsed by kittest):
//!
//! * unset / `false` / `0` / `no` / `off`: compare only (the default; what
//!   `just ui-check` runs).
//! * `true` / `1` / `yes` / `on`: update only the baselines that fail.
//! * `force`: recapture every baseline (comparison threshold drops to 0 so any
//!   difference is rewritten). This is what `just ui-baselines` runs.
//!
//! ## Tolerance policy
//!
//! * Gallery snapshots use kittest's defaults: a per-pixel comparison threshold
//!   of 0.6 and zero pixels allowed to differ. The gallery is pure egui vector
//!   chrome, so it reproduces exactly on a fixed backend.
//! * Full-app states carry the layout canvas (a wgpu paint callback), whose
//!   anti-aliased edges can fringe by a pixel between runs. They keep the default
//!   0.6 per-pixel threshold but allow a failed-pixel count up to 0.1% of the
//!   frame ([`CANVAS_FAILED_PIXEL_PERMILLE`]), mirroring `golden.rs`'s philosophy
//!   but tighter (golden.rs tolerates 5%). A real regression swings far more than
//!   0.1% of pixels, so it still trips.
//!
//! NEVER loosen a threshold to make a diff pass. A comparison that proves flaky
//! is quarantined (`#[ignore]` plus a note here and in RESULT.md), not widened.
//!
//! ## Recapture recipe
//!
//! `just ui-baselines` (sets `UPDATE_SNAPSHOTS = "force"` then runs the binary
//! serially on the GPU). The orchestrator recaptures ALL baselines at Gate 1
//! after the theme/type/component lanes merge: until then the gallery renders a
//! placeholder stub and the app uses `Visuals::dark()`, so these committed
//! baselines are deliberately provisional (see the flip point in
//! `apply_gallery_style`).

#![cfg(not(target_arch = "wasm32"))]

use eframe::egui::{self, Vec2};
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};
use reticle_app::theme::gallery::{self, GalleryGroup, GalleryState};
use reticle_app::theme::tokens::Density;

/// Logical size of a gallery page. Wide enough for the component demos, tall
/// enough that a group fits without scrolling.
const GALLERY_SIZE: Vec2 = Vec2::new(900.0, 700.0);

/// Full-app window sizes to snapshot, as (label, width, height). The three cover
/// the responsive breakpoints the chrome adapts to: the default desktop, a large
/// display, and the compact floor.
const APP_SIZES: [(&str, f32, f32); 3] = [
    ("1280x800", 1280.0, 800.0),
    ("1600x1000", 1600.0, 1000.0),
    ("900x600", 900.0, 600.0),
];

/// The primary size at which the extra interaction states are captured.
const APP_PRIMARY: (&str, f32, f32) = APP_SIZES[0];

/// Frames to advance a full-app harness before snapshotting, so panel layout,
/// the first canvas paint, and any settle-in animation reach a steady frame.
/// A fixed step count keeps the result deterministic and avoids kittest's
/// max-steps panic when the editor requests continuous repaints.
const APP_SETTLE_FRAMES: usize = 8;

/// Permitted failed-pixel budget for canvas-bearing frames, in parts per
/// thousand of the frame (0.1%). See the tolerance policy above.
const CANVAS_FAILED_PIXEL_PERMILLE: usize = 1;

/// Whether a usable GPU adapter is available, using the same probe as the rest
/// of the GPU test suite (`golden.rs`). Every snapshot renders through kittest's
/// wgpu backend, so this gates both families.
fn gpu_available() -> bool {
    reticle_render::WgpuContext::new_blocking().is_some()
}

/// Applies the visual style for a gallery snapshot.
///
/// GATE 1 FLIP POINT: once lane 1A's `theme::apply` API lands on the integration
/// branch, the orchestrator replaces the single body line below with
/// `theme::apply::style(ctx, egui::Theme::Dark, density);` so the gallery
/// baselines pick up the real tokens, fonts, and per-density rhythm. Until then
/// the stub gallery is density-independent, so the Comfortable and Compact
/// baselines of a group are identical, and every baseline here is provisional.
fn apply_gallery_style(ctx: &egui::Context, _density: Density) {
    ctx.set_visuals(egui::Visuals::dark());
}

/// The snapshot name slug for a gallery group (its lower-cased label).
fn group_slug(group: GalleryGroup) -> String {
    group.label().to_ascii_lowercase()
}

/// The snapshot name slug for a density.
fn density_slug(density: Density) -> &'static str {
    match density {
        Density::Comfortable => "comfortable",
        Density::Compact => "compact",
    }
}

/// Fails the test with every collected snapshot error, or passes if there were
/// none. Funnels all snapshots in a test through one [`SnapshotResults`] via the
/// non-panicking `try_*` methods, then handles it explicitly with `into_inner`
/// (which marks it handled), so building several harnesses in one test never
/// trips kittest's "multiple unhandled `SnapshotResults`" guard under any runner.
fn finish(results: SnapshotResults) {
    let errors = results.into_inner();
    assert!(
        errors.is_empty(),
        "{} snapshot(s) failed:\n{}",
        errors.len(),
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Snapshot every gallery group at both densities.
///
/// Ten images (`gallery-<group>-<density>`), one per [`GalleryGroup`] times the
/// two densities. No editor, no document: pure egui over the frozen gallery
/// function, so these are the fast states. They still render through wgpu (see
/// the module note), so the test skips on an adapterless host.
#[test]
fn gallery_snapshots() {
    if !gpu_available() {
        println!("no GPU adapter available; skipping gallery snapshots");
        return;
    }

    let mut results = SnapshotResults::new();
    for group in GalleryGroup::all() {
        for density in [Density::Comfortable, Density::Compact] {
            let name = format!("gallery-{}-{}", group_slug(group), density_slug(density));
            let mut harness = Harness::builder()
                .with_size(GALLERY_SIZE)
                .build_ui(move |ui| {
                    apply_gallery_style(ui.ctx(), density);
                    let mut state = GalleryState {
                        group,
                        ..GalleryState::default()
                    };
                    gallery::ui(ui, &mut state);
                });
            harness.run();
            results.add(harness.try_snapshot(name));
        }
    }
    finish(results);
}

/// Snapshot the full app: `editor-default` at each of the three window sizes,
/// plus `palette-open` at the primary size.
///
/// `App::new` boots straight into the editor with the demo document loaded and
/// the right column at the top of its scroll (its default), so `editor-default`
/// is also the "right panel scrolled top" state the lane brief asked for; a
/// separate identical capture would be redundant, and there is no clean
/// deterministic hook to scroll the column away (recorded as ADR-W1D). The
/// first-run tour is dismissed so the frame shows the steady editor rather than
/// environment-dependent onboarding chrome.
///
/// These are canvas-bearing, so they use the wider [`canvas_options`] tolerance.
#[test]
fn full_app_snapshots() {
    if !gpu_available() {
        println!("no GPU adapter available; skipping full-app snapshots");
        return;
    }

    let mut results = SnapshotResults::new();

    // editor-default at each size.
    for (label, w, h) in APP_SIZES {
        let mut harness = Harness::builder()
            .with_size(Vec2::new(w, h))
            .wgpu()
            .build_eframe(|_cc| reticle_app::App::new());
        harness.state_mut().suppress_onboarding_for_snapshot();
        harness.run_steps(APP_SETTLE_FRAMES);
        results.add(
            harness
                .try_snapshot_options(format!("app-editor-default-{label}"), &canvas_options(w, h)),
        );
    }

    // palette-open at the primary size.
    {
        let (label, w, h) = APP_PRIMARY;
        let mut harness = Harness::builder()
            .with_size(Vec2::new(w, h))
            .wgpu()
            .build_eframe(|_cc| reticle_app::App::new());
        harness.state_mut().suppress_onboarding_for_snapshot();
        harness.state_mut().set_palette_open(true);
        harness.run_steps(APP_SETTLE_FRAMES);
        results.add(
            harness
                .try_snapshot_options(format!("app-palette-open-{label}"), &canvas_options(w, h)),
        );
    }

    finish(results);
}

/// Snapshot options for a canvas-bearing frame of `w` x `h` logical pixels
/// (rendered at 1 pixel per point). Keeps the default per-pixel threshold and
/// allows [`CANVAS_FAILED_PIXEL_PERMILLE`] per thousand pixels to differ.
fn canvas_options(w: f32, h: f32) -> SnapshotOptions {
    let pixels = (w.round() as usize) * (h.round() as usize);
    let allowed = pixels * CANVAS_FAILED_PIXEL_PERMILLE / 1000;
    SnapshotOptions::new().failed_pixel_count_threshold(allowed)
}

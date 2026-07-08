//! Frame-time guard (v8.1 Wave 2, lane 4A).
//!
//! The redesign adds interaction states and functional motion to every panel;
//! this test is the standing proof that none of it made the editor miss frames.
//! It builds the real [`App`](reticle_app::App), drives it through
//! [`MEASURED_FRAMES`] steps on the `egui_kittest` wgpu harness, and asserts the
//! MEDIAN step wall time stays under one 60 Hz budget ([`BUDGET_MS`]).
//!
//! ## Why the median
//!
//! The first frames pay one-time costs the steady state never repeats: the font
//! atlas upload, the initial layout pass, the first canvas paint callback, and
//! the demo document's spatial index warming. A mean would let those outliers
//! dominate; the median reports the frame the user actually lives in. A handful
//! of [`WARMUP_FRAMES`] run untimed first so the measured window is already warm.
//!
//! ## What a "step" measures
//!
//! [`Harness::step`] runs one full egui pass over the whole app (every panel,
//! overlay, and the layout canvas' paint-callback recording) plus the wgpu
//! texture-delta upload. That is the per-frame CPU cost the responsiveness of the
//! UI actually turns on, which is exactly what the states-and-motion work in this
//! lane could regress.
//!
//! ## The honest GPU skip
//!
//! Building the wgpu harness needs a real GPU adapter, so on an adapterless host
//! (headless CI, a machine with no usable device) this test SKIPS honestly: a
//! `println!` plus an early return, mirroring `reticle-render/tests/golden.rs`
//! and `tests/ui_snapshots.rs`. The probe reuses
//! [`reticle_render::WgpuContext::new_blocking`], the house pattern, so the skip
//! decision matches the rest of the GPU suite. ADR 0094 keeps GPU suites
//! orchestrator-only at the integration gates; this is the frame-guard author's
//! documented verify run. It is serialized onto the single GPU by the
//! `.config/nextest.toml` `gpu-serial` group, alongside `ui_snapshots`.
//!
//! NEVER loosen [`BUDGET_MS`] to make a slow run pass. If this proves flaky on
//! the reference GPU it is quarantined (`#[ignore]` plus a note here and in
//! RESULT.md) and demoted to `just perf-check` scope, not widened (ADR-4A).

#![cfg(not(target_arch = "wasm32"))]

use std::time::Instant;

use eframe::egui::Vec2;
use egui_kittest::Harness;

/// The window the guard measures at: the desktop default the app is tuned for
/// and the primary size the visual suite snapshots.
const WINDOW_SIZE: Vec2 = Vec2::new(1280.0, 800.0);

/// Untimed frames run first so the measured window excludes one-time warm-up
/// (font atlas, first layout, first canvas paint). See the module note.
const WARMUP_FRAMES: usize = 12;

/// Timed frames. ~120 steps is two seconds of a 60 Hz session: enough samples
/// for a stable median without making the GPU run long.
const MEASURED_FRAMES: usize = 120;

/// One 60 Hz frame budget, in milliseconds. The median step must come in under
/// this. Held at the honest 16 ms; never widened to force a pass.
const BUDGET_MS: f64 = 16.0;

/// Whether a usable GPU adapter is available, using the same probe as the rest
/// of the GPU test suite (`golden.rs`, `ui_snapshots.rs`). The wgpu harness the
/// guard builds requires an adapter, so this gates the whole test.
fn gpu_available() -> bool {
    reticle_render::WgpuContext::new_blocking().is_some()
}

/// The editor builds a steady-state frame well within the 60 Hz budget.
#[test]
fn editor_median_frame_under_budget() {
    if !gpu_available() {
        println!("no GPU adapter available; skipping frame guard");
        return;
    }

    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .wgpu()
        .build_eframe(|_cc| reticle_app::App::new());
    // Show the steady editor, not environment-dependent first-run onboarding.
    harness.state_mut().suppress_onboarding_for_snapshot();

    harness.run_steps(WARMUP_FRAMES);

    let mut times = Vec::with_capacity(MEASURED_FRAMES);
    for _ in 0..MEASURED_FRAMES {
        let start = Instant::now();
        harness.step();
        times.push(start.elapsed());
    }

    times.sort_unstable();
    let median = times[times.len() / 2];
    let median_ms = median.as_secs_f64() * 1000.0;
    let worst_ms = times[times.len() - 1].as_secs_f64() * 1000.0;
    println!(
        "frame guard: median {median_ms:.3} ms, worst {worst_ms:.3} ms over {MEASURED_FRAMES} \
         steps (budget {BUDGET_MS:.1} ms)"
    );

    assert!(
        median_ms < BUDGET_MS,
        "median step {median_ms:.3} ms exceeds the {BUDGET_MS:.1} ms 60 Hz budget \
         (worst {worst_ms:.3} ms); investigate the regression, do not raise the budget"
    );
}

//! App-level DRC-as-you-type tests: the two-way "draw into a violation, move apart to
//! clear it" behaviour, and the per-edit `check_region` latency at a million shapes.
//!
//! These drive the real app-crate plumbing headlessly (no GPU, no egui): edits go
//! through [`History`] exactly as the editor applies them, the dirtied region is read
//! back with [`History::take_dirty`], and [`LiveDrc::apply_dirty`] runs the same
//! per-frame step the app's frame loop runs on the throttle tick.

use std::time::Instant;

use reticle_app::drc_panel::squiggle_points;
use reticle_app::history::{Dirty, History};
use reticle_app::live_drc::LiveDrc;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, Edit, Rule, RuleKind, ShapeKind, Technology};

const LAYER: LayerId = LayerId {
    layer: 4,
    datatype: 0,
};
const TOP: &str = "TOP";

/// An empty document with one `TOP` cell and a single min-spacing rule on [`LAYER`].
fn doc_with_spacing(min_spacing: i64) -> Document {
    let mut doc = Document::new();
    let tech = Technology {
        rules: vec![Rule {
            name: "met1_spacing".to_owned(),
            kind: RuleKind::Spacing,
            layer: LAYER,
            other_layer: None,
            value: min_spacing,
        }],
        ..Technology::default()
    };
    doc.set_technology(tech);
    doc.insert_cell(Cell::new(TOP));
    doc.set_top_cells(vec![TOP.to_owned()]);
    doc
}

fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
    Rect::new(Point::new(x0, y0), Point::new(x1, y1))
}

fn add_rect(r: Rect) -> Edit {
    Edit::AddShape {
        cell: TOP.to_owned(),
        shape: DrawShape::new(LAYER, ShapeKind::Rect(r)),
    }
}

/// One live-DRC frame on the throttle tick: drain the region the edits dirtied and
/// re-check it against a freshly rebuilt index, mirroring `App::poll_live_drc`.
fn tick(history: &mut History, live: &mut LiveDrc) -> usize {
    let dirty = history.take_dirty();
    let revision = history.revision();
    live.apply_dirty(dirty, history.document(), TOP, revision, true)
}

#[test]
fn drawing_two_rects_too_close_underlines_a_live_violation_then_moving_apart_clears_it() {
    let mut history = History::new(doc_with_spacing(100));
    let mut live = LiveDrc::new();

    // Draw the first rect, then a second one only 40 DBU away: closer than the 100-DBU
    // minimum spacing, so a violation should appear the moment it is drawn.
    let a = rect(0, 0, 50, 50);
    let b_close = rect(90, 0, 140, 50);
    history.apply(add_rect(a)).unwrap();
    history.apply(add_rect(b_close)).unwrap();
    assert_eq!(
        tick(&mut history, &mut live),
        1,
        "the spacing violation appears"
    );
    assert!(!live.is_empty(), "a live violation is underlined");
    assert_eq!(live.violations()[0].kind, RuleKind::Spacing);

    // The underline itself is drawable: the violation's location yields a squiggle of
    // at least two points, so the canvas paints something at the edit.
    let loc = live.violations()[0].location;
    let squiggle = squiggle_points(loc.min.x as f32, loc.max.x as f32, 0.0, 3.0, 8.0);
    assert!(
        squiggle.len() >= 2,
        "the live violation is underlined on screen"
    );

    // Now move the second rect far away (index 1 is `b_close`): a remove-then-add group,
    // exactly as a drag commits. The union of the old and new positions is dirtied.
    let b_far = rect(900, 0, 950, 50);
    history
        .apply_group(vec![
            Edit::RemoveShape {
                cell: TOP.to_owned(),
                index: 1,
            },
            add_rect(b_far),
        ])
        .unwrap();
    assert_eq!(
        tick(&mut history, &mut live),
        0,
        "no violation remains once the rects are far apart"
    );
    assert!(live.is_empty(), "the live underline is cleared");
}

#[test]
fn undo_dirties_full_and_reclears_the_live_set() {
    // A structural/undo edit dirties `Full`; the tick then sweeps the whole indexed
    // area rather than a single region, so undoing the close rect clears its underline.
    let mut history = History::new(doc_with_spacing(100));
    let mut live = LiveDrc::new();
    history.apply(add_rect(rect(0, 0, 50, 50))).unwrap();
    history.apply(add_rect(rect(90, 0, 140, 50))).unwrap();
    assert_eq!(tick(&mut history, &mut live), 1);

    assert!(history.undo(), "the close rect is undone");
    assert_eq!(
        history.take_dirty(),
        Dirty::Full,
        "an undo dirties the whole cell"
    );
    // Feed that Full dirt through the same tick (rebuild + sweep the indexed bounds).
    let revision = history.revision();
    live.apply_dirty(Dirty::Full, history.document(), TOP, revision, true);
    assert!(live.is_empty(), "the violation is gone after the undo");
}

/// Per-edit `check_region` latency at a million shapes.
///
/// Builds a 1000x1000 grid of clean, well-spaced rects (a million shapes), prepares
/// the live index once (the throttled, off-hot-path step), then times the synchronous
/// per-edit re-check over small edit-sized windows. The per-edit path must stay well
/// under a millisecond; see `PERF.md` for the recorded numbers and methodology.
#[test]
fn per_edit_recheck_under_one_millisecond_at_one_million_shapes() {
    const SIDE: i32 = 1000; // 1000 x 1000 = 1_000_000 shapes
    const PITCH: i32 = 100;
    const SIZE: i32 = 50; // 50-DBU rects on a 100-DBU pitch: 50-DBU gaps, spacing-clean
    const SAMPLES: usize = 500;
    const WINDOW: i32 = 250;

    let mut cell = Cell::new(TOP);
    cell.shapes.reserve((SIDE * SIDE) as usize);
    for gy in 0..SIDE {
        for gx in 0..SIDE {
            let x = gx * PITCH;
            let y = gy * PITCH;
            cell.shapes.push(DrawShape::new(
                LAYER,
                ShapeKind::Rect(rect(x, y, x + SIZE, y + SIZE)),
            ));
        }
    }
    // A 10-DBU minimum spacing that the 50-DBU gaps satisfy, so the grid is clean and
    // each windowed re-check returns no violations (measuring the query, not reporting).
    let mut doc = doc_with_spacing(10);
    doc.insert_cell(cell);
    let shape_count = doc.cell(TOP).map_or(0, |c| c.shapes.len());
    assert_eq!(shape_count, 1_000_000, "a million shapes under test");

    let history = History::new(doc);
    let mut live = LiveDrc::new();

    // The one-time, throttled prepare over the whole cell (not the per-edit hot path).
    let t_prepare = Instant::now();
    live.reprepare(history.document(), TOP, 1);
    let prepare = t_prepare.elapsed();
    assert!(live.has_index());

    // Time the synchronous per-edit re-check over small, edit-sized windows scattered
    // across the populated area (each covers a handful of rects, like a real edit).
    let mut durations = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Deterministic spread across the grid interior (no RNG in tests).
        let gx = ((i * 137) % (SIDE as usize - 4) + 2) as i32;
        let gy = ((i * 71) % (SIDE as usize - 4) + 2) as i32;
        let cx = gx * PITCH;
        let cy = gy * PITCH;
        let region = rect(
            cx - WINDOW / 2,
            cy - WINDOW / 2,
            cx + WINDOW / 2,
            cy + WINDOW / 2,
        );
        let t = Instant::now();
        let n = live.recheck(region);
        durations.push(t.elapsed());
        assert_eq!(n, 0, "the well-spaced grid is spacing-clean");
    }

    durations.sort_unstable();
    let median = durations[durations.len() / 2];
    let p99 = durations[durations.len() * 99 / 100];
    let max = *durations.last().unwrap();
    eprintln!(
        "per-edit check_region @1M: prepare {:.1} ms, median {:.1} us, p99 {:.1} us, max {:.1} us ({} samples)",
        prepare.as_secs_f64() * 1e3,
        median.as_secs_f64() * 1e6,
        p99.as_secs_f64() * 1e6,
        max.as_secs_f64() * 1e6,
        SAMPLES,
    );

    // The per-edit budget: comfortably sub-millisecond. Median guards the typical edit;
    // p99 guards the tail without being hostage to one scheduler hiccup.
    assert!(
        median.as_secs_f64() < 1e-3,
        "median per-edit re-check must be under 1 ms, was {median:?}"
    );
    assert!(
        p99.as_secs_f64() < 1e-3,
        "p99 per-edit re-check must be under 1 ms, was {p99:?}"
    );
}

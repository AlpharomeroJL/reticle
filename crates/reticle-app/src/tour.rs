//! The embedded first-run tour state machine.
//!
//! A dismissable overlay walks a new user through the editor's real panels in
//! order, one step at a time: the canvas and pan/zoom, the layer manager, the
//! measure tool, running DRC and click-to-zoom, net highlighting, the minimap, and
//! the agent/replay theater. Each step names the actual control it points at so the
//! egui layer can draw a highlight box around that region. A second, optional
//! chapter covers the Wave 2 tools (drawing, boolean/transform, productivity,
//! snapping, layer/technology editing, search, and view/export).
//!
//! This module is deliberately **pure and egui-free**: it is the ordered list of
//! steps, the current position, and the transitions between them (next, skip,
//! finish), plus the first-run-versus-relaunched distinction and whether the second
//! chapter is included. All of that is unit-tested here without a window. The egui
//! glue (drawing the overlay card, wiring Next/Skip buttons, and highlighting the
//! named region) lives in [`crate::app`], which owns one [`Tour`] field.
//!
//! ## Persistence
//!
//! Whether the tour has been seen is a single bit persisted with the rest of the
//! view state (see [`crate::session::SessionState::tour_seen`]). On the first launch
//! with no saved session, [`Tour::first_run`] starts the tour automatically; on
//! every later launch it stays dormant until the user relaunches it from the Help
//! menu with [`Tour::relaunch`].
//!
//! ## Chapters
//!
//! The steps are grouped into two [`Chapter`]s. Chapter 1 (the core walkthrough)
//! always runs. Chapter 2 (the Wave 2 tools) is optional: [`Tour::skip`] on the last
//! core step ends the tour when the user declines it, and advancing past it enters
//! the second chapter. A relaunch can request either the core chapter alone or both.
//!
//! Nothing here depends on `egui`, the GPU, or the filesystem, so it compiles
//! unchanged on `wasm32` and is exercised entirely by the tests at the bottom of the
//! file.

/// Which chapter a [`TourStep`] belongs to.
///
/// The two chapters are shown back to back when the tour includes both: finishing
/// the last [`Chapter::Core`] step advances into the first [`Chapter::Wave2`] step.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Chapter {
    /// The core walkthrough: canvas, layers, measure, DRC, net highlight, minimap,
    /// and the agent/replay theater. Always shown.
    Core,
    /// The optional second chapter covering the Wave 2 tools.
    Wave2,
}

impl Chapter {
    /// A short human label for the chapter, shown in the overlay header.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Chapter::Core => "Getting started",
            Chapter::Wave2 => "Wave 2 tools",
        }
    }
}

/// The UI region a tour step points at.
///
/// This is a *named* target, not a pixel rectangle: the egui layer maps each
/// variant to the real panel or control it already draws and highlights that
/// region's rectangle. Keeping the target abstract means the tour never hard-codes
/// coordinates and stays robust as the layout is resized or rearranged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TourTarget {
    /// The central layout canvas (pan/zoom happen here).
    Canvas,
    /// The left-hand layer manager panel.
    LayerPanel,
    /// The toolbar row where tools (including Measure) are selected.
    Toolbar,
    /// The DRC panel in the right-hand column.
    DrcPanel,
    /// The net-highlight control (part of the right-hand column).
    NetHighlight,
    /// The minimap overview drawn inside the canvas.
    Minimap,
    /// The agent panel and replay-theater controls.
    AgentPanel,
    /// The drawing tools on the toolbar (rectangle, polygon, path).
    DrawTools,
    /// The boolean/transform operations panel.
    OpsPanel,
    /// The productivity panel (clipboard, array, move-by-delta, via stacks).
    ProductivityPanel,
    /// The snapping and guides panel.
    SnapPanel,
    /// The search / selection-depth panel.
    SearchPanel,
    /// The technology / layer editor panel.
    TechPanel,
    /// The view-and-export panel.
    ViewExportPanel,
}

/// One step of the tour: a stable id, a chapter, a highlighted target, and the
/// header/body text shown in the overlay card.
///
/// The `id` is a stable machine-readable key (handy for tests and for the egui
/// layer to key persistent widget state); the `title` and `body` are the
/// human-facing copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TourStep {
    /// A stable identifier for this step, unique across both chapters.
    pub id: &'static str,
    /// The chapter this step belongs to.
    pub chapter: Chapter,
    /// The UI region the overlay should highlight for this step.
    pub target: TourTarget,
    /// The step's short header line.
    pub title: &'static str,
    /// The step's body copy: what the panel does and what to try.
    pub body: &'static str,
}

/// The ordered core-chapter steps, always shown.
const CORE_STEPS: &[TourStep] = &[
    TourStep {
        id: "canvas",
        chapter: Chapter::Core,
        target: TourTarget::Canvas,
        title: "The canvas",
        body: "This is the layout canvas. Drag to pan and scroll to zoom toward the \
               cursor. Press Fit in the toolbar to frame the whole design.",
    },
    TourStep {
        id: "layers",
        chapter: Chapter::Core,
        target: TourTarget::LayerPanel,
        title: "Layers",
        body: "The layer manager on the left lists every layer. Toggle a checkbox to \
               show or hide a layer, and type in the filter to find one by name.",
    },
    TourStep {
        id: "measure",
        chapter: Chapter::Core,
        target: TourTarget::Toolbar,
        title: "Measure",
        body: "Pick the Measure tool from the toolbar, then click two points on the \
               canvas to read the distance in database units and microns.",
    },
    TourStep {
        id: "drc",
        chapter: Chapter::Core,
        target: TourTarget::DrcPanel,
        title: "Design-rule checking",
        body: "Run DRC from this panel to list rule violations. Click a violation to \
               zoom the canvas straight to it.",
    },
    TourStep {
        id: "net-highlight",
        chapter: Chapter::Core,
        target: TourTarget::NetHighlight,
        title: "Net highlight",
        body: "Highlight a net to light up every shape connected to it across the \
               design, so you can trace where a signal goes.",
    },
    TourStep {
        id: "minimap",
        chapter: Chapter::Core,
        target: TourTarget::Minimap,
        title: "Minimap",
        body: "The minimap overview shows the whole design with your current view \
               framed. Click inside it to jump the camera there.",
    },
    TourStep {
        id: "agent",
        chapter: Chapter::Core,
        target: TourTarget::AgentPanel,
        title: "Agent and replay",
        body: "The agent panel runs a scripted edit session, and the replay theater \
               plays a recorded run back step by step. That is the core tour.",
    },
];

/// The ordered Wave 2 chapter steps, shown only when the tour includes chapter two.
const WAVE2_STEPS: &[TourStep] = &[
    TourStep {
        id: "draw",
        chapter: Chapter::Wave2,
        target: TourTarget::DrawTools,
        title: "Drawing tools",
        body: "Draw rectangles, polygons, and paths from the toolbar, then switch to \
               the vertex-edit tool to drag individual corners.",
    },
    TourStep {
        id: "boolean",
        chapter: Chapter::Wave2,
        target: TourTarget::OpsPanel,
        title: "Boolean and transform",
        body: "The operations panel unions, intersects, and subtracts selected \
               shapes, and applies transforms. Every edit is undoable.",
    },
    TourStep {
        id: "productivity",
        chapter: Chapter::Wave2,
        target: TourTarget::ProductivityPanel,
        title: "Productivity",
        body: "Copy, duplicate, build arrays, move by an exact delta, and drop via \
               stacks from the productivity panel.",
    },
    TourStep {
        id: "snapping",
        chapter: Chapter::Wave2,
        target: TourTarget::SnapPanel,
        title: "Snapping and guides",
        body: "Snap the cursor to vertices, edges, midpoints, and centers, and pull \
               guide lines off the rulers to align geometry.",
    },
    TourStep {
        id: "layer-tech",
        chapter: Chapter::Wave2,
        target: TourTarget::TechPanel,
        title: "Layer and technology editing",
        body: "Reorder, recolor, and restyle layers, and edit the technology \
               definition, validated and round-tripped to the tech file.",
    },
    TourStep {
        id: "search",
        chapter: Chapter::Wave2,
        target: TourTarget::SearchPanel,
        title: "Search and selection",
        body: "Filter shapes with a query such as `layer:METAL1 width<400`, save \
               selection sets, and navigate the cell outline tree.",
    },
    TourStep {
        id: "view-export",
        chapter: Chapter::Wave2,
        target: TourTarget::ViewExportPanel,
        title: "View and export",
        body: "Switch the theme, save camera bookmarks, and export the current view \
               or selection to SVG and PNG. That completes the tour.",
    },
];

/// Whether the tour is dormant, running, or done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Not running: either never started or dismissed. Carries no step.
    Idle,
    /// Running: `index` points into the active step list.
    Running { index: usize },
    /// Finished (the user reached the end or skipped past the last step).
    Finished,
}

/// The tour state machine: the ordered steps, the current position, and whether the
/// optional second chapter is included.
///
/// Construct it with [`Tour::first_run`] on a fresh install (it starts running both
/// chapters), [`Tour::already_seen`] when a saved session records the tour as done
/// (it stays idle), or [`Tour::relaunch`] from the Help menu. The egui layer reads
/// [`Tour::current`] each frame to draw the overlay and calls [`Tour::next`],
/// [`Tour::skip`], or [`Tour::finish`] from the buttons.
#[derive(Clone, Debug)]
pub struct Tour {
    /// The active, ordered step list. This is `CORE_STEPS` alone, or `CORE_STEPS`
    /// followed by `WAVE2_STEPS` when the second chapter is included.
    steps: Vec<TourStep>,
    /// Where in `steps` the tour is, or whether it is idle or finished.
    phase: Phase,
    /// `true` if this run began automatically on first launch (as opposed to a
    /// user-initiated relaunch from the Help menu).
    first_run: bool,
}

impl Tour {
    /// The tour for a brand-new install: it starts running immediately and includes
    /// both chapters.
    ///
    /// This is what the app builds when no saved session is found, so a first-time
    /// user is walked through the editor without asking.
    #[must_use]
    pub fn first_run() -> Self {
        Self {
            steps: all_steps(),
            phase: Phase::Running { index: 0 },
            first_run: true,
        }
    }

    /// The tour for a returning user whose saved session records it as already seen:
    /// it stays idle until relaunched.
    #[must_use]
    pub fn already_seen() -> Self {
        Self {
            steps: all_steps(),
            phase: Phase::Idle,
            first_run: false,
        }
    }

    /// Rebuilds the tour from the persisted "seen" bit.
    ///
    /// `seen == false` means a fresh install, so the tour auto-starts
    /// ([`Tour::first_run`]); `seen == true` means it has run before, so it stays
    /// dormant ([`Tour::already_seen`]). This is the single entry point the app uses
    /// when constructing itself from a loaded session.
    #[must_use]
    pub fn from_seen(seen: bool) -> Self {
        if seen {
            Self::already_seen()
        } else {
            Self::first_run()
        }
    }

    /// Relaunches the tour from the beginning, choosing whether to include the
    /// optional Wave 2 chapter.
    ///
    /// This is what the Help menu calls. It always starts at the first core step and
    /// is marked as *not* a first run, so the caller can distinguish an automatic
    /// first-launch tour from a user-requested one.
    pub fn relaunch(&mut self, include_wave2: bool) {
        self.steps = if include_wave2 {
            all_steps()
        } else {
            CORE_STEPS.to_vec()
        };
        self.phase = Phase::Running { index: 0 };
        self.first_run = false;
    }

    /// The step currently shown, or `None` when the tour is idle or finished.
    #[must_use]
    pub fn current(&self) -> Option<&TourStep> {
        match self.phase {
            Phase::Running { index } => self.steps.get(index),
            Phase::Idle | Phase::Finished => None,
        }
    }

    /// Whether the overlay should be drawn this frame (the tour is running and has a
    /// current step).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.current().is_some()
    }

    /// Whether the tour has finished (reached or skipped past the last step).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self.phase, Phase::Finished)
    }

    /// Whether this run started automatically on first launch.
    ///
    /// The app persists the "seen" bit once this is a first run that reaches its
    /// end, so the automatic tour never shows twice.
    #[must_use]
    pub fn is_first_run(&self) -> bool {
        self.first_run
    }

    /// The chapter of the current step, or `None` when the tour is not running.
    #[must_use]
    pub fn current_chapter(&self) -> Option<Chapter> {
        self.current().map(|s| s.chapter)
    }

    /// The one-based position of the current step and the total step count, for a
    /// "Step 3 of 14" readout. `None` when the tour is not running.
    #[must_use]
    pub fn progress(&self) -> Option<(usize, usize)> {
        match self.phase {
            Phase::Running { index } => Some((index + 1, self.steps.len())),
            Phase::Idle | Phase::Finished => None,
        }
    }

    /// Advances to the next step, finishing the tour after the last one.
    ///
    /// Crossing the boundary from the last core step into the first Wave 2 step is
    /// just the ordinary next transition, because the two chapters are one flat
    /// ordered list. A no-op when the tour is idle or already finished.
    pub fn next(&mut self) {
        if let Phase::Running { index } = self.phase {
            let next = index + 1;
            if next < self.steps.len() {
                self.phase = Phase::Running { index: next };
            } else {
                self.phase = Phase::Finished;
            }
        }
    }

    /// Skips the rest of the current chapter.
    ///
    /// From a [`Chapter::Core`] step this jumps to the first [`Chapter::Wave2`] step
    /// if the second chapter is included, otherwise it finishes; this is the "decline
    /// the optional chapter" path. From a [`Chapter::Wave2`] step it finishes the
    /// tour. A no-op when the tour is idle or finished.
    pub fn skip(&mut self) {
        let Phase::Running { index } = self.phase else {
            return;
        };
        let Some(current) = self.steps.get(index).copied() else {
            self.phase = Phase::Finished;
            return;
        };
        match current.chapter {
            Chapter::Core => {
                // Jump to the first step of the next chapter, if there is one.
                match self.steps.iter().position(|s| s.chapter == Chapter::Wave2) {
                    Some(next) => self.phase = Phase::Running { index: next },
                    None => self.phase = Phase::Finished,
                }
            }
            Chapter::Wave2 => self.phase = Phase::Finished,
        }
    }

    /// Ends the tour immediately (the overlay's dismiss/close button).
    ///
    /// Unlike [`Tour::skip`], this finishes from any step regardless of chapter.
    pub fn finish(&mut self) {
        self.phase = Phase::Finished;
    }
}

/// The full ordered step list: the core chapter followed by the Wave 2 chapter.
fn all_steps() -> Vec<TourStep> {
    CORE_STEPS.iter().chain(WAVE2_STEPS).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_starts_on_the_first_core_step() {
        let tour = Tour::first_run();
        assert!(tour.is_active());
        assert!(tour.is_first_run());
        assert!(!tour.is_finished());
        let step = tour.current().expect("a first step");
        assert_eq!(step.id, "canvas");
        assert_eq!(step.chapter, Chapter::Core);
        assert_eq!(step.target, TourTarget::Canvas);
    }

    #[test]
    fn already_seen_stays_idle() {
        let tour = Tour::already_seen();
        assert!(!tour.is_active());
        assert!(!tour.is_finished());
        assert!(!tour.is_first_run());
        assert!(tour.current().is_none());
        assert!(tour.progress().is_none());
        assert!(tour.current_chapter().is_none());
    }

    #[test]
    fn from_seen_dispatches_first_run_versus_seen() {
        assert!(Tour::from_seen(false).is_active(), "unseen auto-starts");
        assert!(Tour::from_seen(false).is_first_run());
        assert!(!Tour::from_seen(true).is_active(), "seen stays idle");
        assert!(!Tour::from_seen(true).is_first_run());
    }

    #[test]
    fn next_walks_every_step_in_order_then_finishes() {
        let mut tour = Tour::first_run();
        let expected: Vec<&str> = CORE_STEPS.iter().chain(WAVE2_STEPS).map(|s| s.id).collect();
        let total = expected.len();

        for (i, id) in expected.iter().enumerate() {
            let step = tour.current().expect("still running");
            assert_eq!(step.id, *id, "step {i} out of order");
            assert_eq!(tour.progress(), Some((i + 1, total)));
            tour.next();
        }

        // One next past the last step finishes the tour.
        assert!(tour.is_finished());
        assert!(!tour.is_active());
        assert!(tour.current().is_none());
    }

    #[test]
    fn core_and_wave2_are_contiguous_and_ordered() {
        // Chapter 1 is exactly the core steps, chapter 2 exactly the Wave 2 steps,
        // and the whole list is core-then-wave2 with no interleaving.
        let all = all_steps();
        let split = CORE_STEPS.len();
        assert_eq!(all.len(), CORE_STEPS.len() + WAVE2_STEPS.len());
        assert!(all[..split].iter().all(|s| s.chapter == Chapter::Core));
        assert!(all[split..].iter().all(|s| s.chapter == Chapter::Wave2));
    }

    #[test]
    fn crossing_the_chapter_boundary_with_next_enters_wave2() {
        let mut tour = Tour::first_run();
        // Advance to the last core step.
        for _ in 0..CORE_STEPS.len() - 1 {
            tour.next();
        }
        let last_core = tour.current().expect("last core step");
        assert_eq!(last_core.chapter, Chapter::Core);
        assert_eq!(last_core.id, CORE_STEPS.last().unwrap().id);

        // The next step is the first Wave 2 step.
        tour.next();
        let first_wave2 = tour.current().expect("first wave2 step");
        assert_eq!(first_wave2.chapter, Chapter::Wave2);
        assert_eq!(first_wave2.id, WAVE2_STEPS[0].id);
    }

    #[test]
    fn skip_from_a_core_step_jumps_to_wave2_when_included() {
        let mut tour = Tour::first_run();
        assert_eq!(tour.current_chapter(), Some(Chapter::Core));
        tour.skip();
        // Declining the core chapter lands on the first Wave 2 step, not the end.
        assert!(tour.is_active());
        let step = tour.current().expect("moved into wave2");
        assert_eq!(step.chapter, Chapter::Wave2);
        assert_eq!(step.id, WAVE2_STEPS[0].id);
    }

    #[test]
    fn skip_from_a_core_step_finishes_when_wave2_is_absent() {
        // A core-only relaunch has no second chapter, so skip finishes.
        let mut tour = Tour::already_seen();
        tour.relaunch(false);
        assert_eq!(tour.current_chapter(), Some(Chapter::Core));
        tour.skip();
        assert!(tour.is_finished());
        assert!(!tour.is_active());
    }

    #[test]
    fn skip_from_a_wave2_step_finishes() {
        let mut tour = Tour::first_run();
        // Walk to the first Wave 2 step.
        for _ in 0..CORE_STEPS.len() {
            tour.next();
        }
        assert_eq!(tour.current_chapter(), Some(Chapter::Wave2));
        tour.skip();
        assert!(tour.is_finished());
    }

    #[test]
    fn finish_ends_from_any_step() {
        let mut tour = Tour::first_run();
        tour.next();
        tour.next();
        assert!(tour.is_active());
        tour.finish();
        assert!(tour.is_finished());
        assert!(!tour.is_active());
        assert!(tour.current().is_none());
    }

    #[test]
    fn relaunch_with_both_chapters_restarts_from_the_first_step() {
        let mut tour = Tour::first_run();
        tour.finish();
        assert!(tour.is_finished());

        tour.relaunch(true);
        assert!(tour.is_active());
        assert!(!tour.is_first_run(), "a relaunch is not a first run");
        assert_eq!(tour.current().unwrap().id, "canvas");
        assert_eq!(
            tour.progress(),
            Some((1, CORE_STEPS.len() + WAVE2_STEPS.len()))
        );
    }

    #[test]
    fn relaunch_core_only_excludes_wave2() {
        let mut tour = Tour::already_seen();
        tour.relaunch(false);
        assert_eq!(tour.progress(), Some((1, CORE_STEPS.len())));
        // Walking to the end never enters a Wave 2 step.
        while tour.is_active() {
            assert_eq!(tour.current_chapter(), Some(Chapter::Core));
            tour.next();
        }
        assert!(tour.is_finished());
    }

    #[test]
    fn first_run_shows_once_then_relaunch_is_manual() {
        // Model the app lifecycle: first run auto-starts, the user finishes it, the
        // "seen" bit is persisted, and the next launch stays idle until a relaunch.
        let mut first = Tour::first_run();
        assert!(first.is_active());
        for _ in 0..CORE_STEPS.len() + WAVE2_STEPS.len() {
            first.next();
        }
        assert!(first.is_finished());
        let seen_after_first_run = first.is_finished();
        assert!(
            seen_after_first_run,
            "the app would persist tour_seen = true"
        );

        // Next launch reconstructs from the persisted bit and stays dormant.
        let next_launch = Tour::from_seen(seen_after_first_run);
        assert!(!next_launch.is_active());

        // Only an explicit relaunch shows it again.
        let mut relaunched = next_launch;
        relaunched.relaunch(true);
        assert!(relaunched.is_active());
        assert_eq!(relaunched.current().unwrap().id, "canvas");
    }

    #[test]
    fn next_and_skip_are_no_ops_when_idle_or_finished() {
        let mut idle = Tour::already_seen();
        idle.next();
        idle.skip();
        assert!(!idle.is_active(), "still idle after next/skip");
        assert!(!idle.is_finished());

        let mut done = Tour::first_run();
        done.finish();
        done.next();
        done.skip();
        assert!(done.is_finished(), "still finished after next/skip");
    }

    #[test]
    fn every_step_has_a_unique_id() {
        let all = all_steps();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.id, b.id, "duplicate step id {}", a.id);
            }
            assert!(!a.title.is_empty(), "step {} has an empty title", a.id);
            assert!(!a.body.is_empty(), "step {} has an empty body", a.id);
        }
    }

    #[test]
    fn chapter_labels_are_stable() {
        assert_eq!(Chapter::Core.label(), "Getting started");
        assert_eq!(Chapter::Wave2.label(), "Wave 2 tools");
    }
}

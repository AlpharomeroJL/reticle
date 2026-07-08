//! First-contact onboarding state: once-only contextual hints, the onboarding
//! checklist, and the first-run GPU capability card (catalog 17, 19, 22).
//!
//! This module is deliberately **pure and egui-free**: it is the state machines
//! (which hints have fired, which checklist tasks are done, whether a card is
//! dismissed) and the copy shown in each, unit-tested here without a window. The
//! egui glue (drawing the hint bubble, the checklist card, and the GPU card, and
//! wiring their dismiss buttons) lives in [`crate::app`].
//!
//! ## Persistence
//!
//! Dismissal is sticky: a hint fires once and never again, and the checklist and
//! GPU card, once dismissed, stay dismissed. The sticky bits ride in
//! [`SessionState`](crate::session) so they survive a relaunch on native (there is
//! no session file on the web, so they reset per page load, matching the tour).

/// One of the three once-only contextual hints, each tied to the first time the
/// user reaches a particular surface.
///
/// A hint fires exactly once: the first time its trigger happens and the hint has
/// not been seen, the overlay shows a small bubble by the relevant control; any
/// interaction or an explicit dismiss marks it seen forever.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Hint {
    /// First time the Layers panel is opened: how visibility and filtering work.
    Layers,
    /// First time DRC is run: how to navigate the violations.
    Drc,
    /// First time a session is shared: what the link grants.
    Share,
}

impl Hint {
    /// Every hint, in a stable order (used for serialization and tests).
    pub const ALL: [Hint; 3] = [Hint::Layers, Hint::Drc, Hint::Share];

    /// The stable text tag used when persisting which hints have been seen.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Hint::Layers => "layers",
            Hint::Drc => "drc",
            Hint::Share => "share",
        }
    }

    /// Parses a persisted hint tag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Hint> {
        Hint::ALL.into_iter().find(|h| h.tag() == tag.trim())
    }

    /// The hint's short header line.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Hint::Layers => "Layer visibility",
            Hint::Drc => "Jump between violations",
            Hint::Share => "Anyone with the link",
        }
    }

    /// The hint's one-line body copy.
    #[must_use]
    pub fn body(self) -> &'static str {
        match self {
            Hint::Layers => {
                "Toggle a row to hide a layer, alt-click to solo it, and type in the \
                 filter to find one."
            }
            Hint::Drc => {
                "Click a violation to zoom straight to it, or step through them with \
                 the n and N keys."
            }
            Hint::Share => {
                "A share link opens this exact design in a browser, view-only, with \
                 your live cursor."
            }
        }
    }
}

/// Which once-only hints have already been shown.
///
/// Construct from the persisted set with [`Hints::from_tags`] and read it back with
/// [`Hints::seen_tags`]. [`Hints::fire`] is the single mutation the app calls when a
/// trigger happens; it returns the hint to show *only* the first time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hints {
    /// Seen flags, indexed to match [`Hint::ALL`].
    seen: [bool; 3],
}

impl Hints {
    /// The index of a hint within the `seen` array.
    fn index(hint: Hint) -> usize {
        match hint {
            Hint::Layers => 0,
            Hint::Drc => 1,
            Hint::Share => 2,
        }
    }

    /// Rebuilds the set from the persisted seen tags (unknown tags are ignored).
    #[must_use]
    pub fn from_tags(tags: &[&str]) -> Self {
        let mut hints = Self::default();
        for tag in tags {
            if let Some(hint) = Hint::from_tag(tag) {
                hints.seen[Self::index(hint)] = true;
            }
        }
        hints
    }

    /// Whether `hint` has already been shown.
    #[must_use]
    pub fn is_seen(&self, hint: Hint) -> bool {
        self.seen[Self::index(hint)]
    }

    /// Records `hint` as seen; returns `true` if this call changed the state (i.e.
    /// it was unseen before), so the caller can persist only on a real change.
    pub fn mark_seen(&mut self, hint: Hint) -> bool {
        let slot = &mut self.seen[Self::index(hint)];
        let changed = !*slot;
        *slot = true;
        changed
    }

    /// Fires `hint`: if it has not been seen, marks it seen and returns it (the app
    /// shows the bubble); otherwise returns `None` and does nothing.
    pub fn fire(&mut self, hint: Hint) -> Option<Hint> {
        if self.is_seen(hint) {
            None
        } else {
            self.seen[Self::index(hint)] = true;
            Some(hint)
        }
    }

    /// The seen hints as their stable tags, for persistence.
    #[must_use]
    pub fn seen_tags(&self) -> Vec<&'static str> {
        Hint::ALL
            .into_iter()
            .filter(|h| self.is_seen(*h))
            .map(Hint::tag)
            .collect()
    }
}

/// One task on the onboarding checklist (catalog 19).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Task {
    /// Open a real design (a file, a URL, or an example).
    OpenFile,
    /// Run design-rule checking at least once.
    RunDrc,
    /// Share the session by link.
    Share,
    /// Try the agent (run it or replay a transcript).
    TryAgent,
}

impl Task {
    /// Every task, in checklist order.
    pub const ALL: [Task; 4] = [Task::OpenFile, Task::RunDrc, Task::Share, Task::TryAgent];

    /// The stable text tag used when persisting completed tasks.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Task::OpenFile => "open",
            Task::RunDrc => "drc",
            Task::Share => "share",
            Task::TryAgent => "agent",
        }
    }

    /// Parses a persisted task tag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Task> {
        Task::ALL.into_iter().find(|t| t.tag() == tag.trim())
    }

    /// The task's one-line label on the checklist.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Task::OpenFile => "Open a design",
            Task::RunDrc => "Run a design-rule check",
            Task::Share => "Share the session by link",
            Task::TryAgent => "Try the agent",
        }
    }
}

/// The onboarding checklist: which tasks are done and whether the card is dismissed.
///
/// The card shows progress ("2 of 4") and a permanent dismiss; once dismissed it
/// never returns even with tasks outstanding. It also self-retires when every task
/// is complete, so a user who finishes the loop is not nagged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Checklist {
    /// Done flags, indexed to match [`Task::ALL`].
    done: [bool; 4],
    /// Whether the user permanently dismissed the card.
    dismissed: bool,
}

impl Checklist {
    /// The index of a task within the `done` array.
    fn index(task: Task) -> usize {
        match task {
            Task::OpenFile => 0,
            Task::RunDrc => 1,
            Task::Share => 2,
            Task::TryAgent => 3,
        }
    }

    /// Rebuilds the checklist from persisted completed-task tags and the dismissed
    /// bit (unknown tags are ignored).
    #[must_use]
    pub fn restore(done_tags: &[&str], dismissed: bool) -> Self {
        let mut checklist = Self {
            dismissed,
            ..Self::default()
        };
        for tag in done_tags {
            if let Some(task) = Task::from_tag(tag) {
                checklist.done[Self::index(task)] = true;
            }
        }
        checklist
    }

    /// Whether `task` is done.
    #[must_use]
    pub fn is_done(&self, task: Task) -> bool {
        self.done[Self::index(task)]
    }

    /// Marks `task` complete; returns `true` if this changed the state, so the
    /// caller can persist only on a real change.
    pub fn complete(&mut self, task: Task) -> bool {
        let slot = &mut self.done[Self::index(task)];
        let changed = !*slot;
        *slot = true;
        changed
    }

    /// Permanently dismisses the card.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Whether the user dismissed the card.
    #[must_use]
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// The number of completed tasks.
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.done.iter().filter(|d| **d).count()
    }

    /// Progress as `(done, total)` for the "2 of 4" readout.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (self.completed_count(), Task::ALL.len())
    }

    /// Whether every task is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.done.iter().all(|d| *d)
    }

    /// Whether the card should be drawn: not dismissed and not yet fully complete.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.dismissed && !self.is_complete()
    }

    /// The completed tasks as their stable tags, for persistence.
    #[must_use]
    pub fn done_tags(&self) -> Vec<&'static str> {
        Task::ALL
            .into_iter()
            .filter(|t| self.is_done(*t))
            .map(Task::tag)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_fires_exactly_once() {
        let mut hints = Hints::default();
        assert_eq!(hints.fire(Hint::Layers), Some(Hint::Layers));
        // Second time it is silent.
        assert_eq!(hints.fire(Hint::Layers), None);
        assert!(hints.is_seen(Hint::Layers));
        // Independent hints are unaffected.
        assert!(!hints.is_seen(Hint::Drc));
        assert_eq!(hints.fire(Hint::Drc), Some(Hint::Drc));
    }

    #[test]
    fn mark_seen_reports_change() {
        let mut hints = Hints::default();
        assert!(hints.mark_seen(Hint::Share), "first mark changes state");
        assert!(!hints.mark_seen(Hint::Share), "second mark is a no-op");
    }

    #[test]
    fn hints_round_trip_through_tags() {
        let mut hints = Hints::default();
        hints.mark_seen(Hint::Layers);
        hints.mark_seen(Hint::Share);
        let tags = hints.seen_tags();
        assert_eq!(tags, vec!["layers", "share"]);
        let restored = Hints::from_tags(&tags);
        assert_eq!(restored, hints);
        // Unknown tags are ignored, not fatal.
        let tolerant = Hints::from_tags(&["layers", "wombat"]);
        assert!(tolerant.is_seen(Hint::Layers));
        assert!(!tolerant.is_seen(Hint::Drc));
    }

    #[test]
    fn every_hint_has_copy_and_a_tag() {
        for hint in Hint::ALL {
            assert!(!hint.title().is_empty());
            assert!(!hint.body().is_empty());
            assert_eq!(Hint::from_tag(hint.tag()), Some(hint));
            // The style gate: no em dash in onboarding copy.
            assert!(!hint.title().contains('\u{2014}'));
            assert!(!hint.body().contains('\u{2014}'));
        }
    }

    #[test]
    fn checklist_tracks_progress_and_completion() {
        let mut list = Checklist::default();
        assert_eq!(list.progress(), (0, 4));
        assert!(list.is_visible());
        assert!(list.complete(Task::OpenFile));
        assert!(!list.complete(Task::OpenFile), "idempotent");
        assert_eq!(list.progress(), (1, 4));
        list.complete(Task::RunDrc);
        list.complete(Task::Share);
        assert!(!list.is_complete());
        list.complete(Task::TryAgent);
        assert!(list.is_complete());
        // A complete checklist retires itself.
        assert!(!list.is_visible());
    }

    #[test]
    fn checklist_dismiss_is_sticky() {
        let mut list = Checklist::default();
        list.complete(Task::OpenFile);
        list.dismiss();
        assert!(list.is_dismissed());
        assert!(!list.is_visible(), "a dismissed card never returns");
    }

    #[test]
    fn checklist_round_trips_through_tags() {
        let mut list = Checklist::default();
        list.complete(Task::OpenFile);
        list.complete(Task::Share);
        let tags = list.done_tags();
        assert_eq!(tags, vec!["open", "share"]);
        let restored = Checklist::restore(&tags, false);
        assert_eq!(restored, list);
        // The dismissed bit round-trips independently.
        let dismissed = Checklist::restore(&["drc"], true);
        assert!(dismissed.is_done(Task::RunDrc));
        assert!(dismissed.is_dismissed());
    }

    #[test]
    fn every_task_has_a_label_and_tag() {
        for task in Task::ALL {
            assert!(!task.label().is_empty());
            assert_eq!(Task::from_tag(task.tag()), Some(task));
            assert!(!task.label().contains('\u{2014}'));
        }
    }
}

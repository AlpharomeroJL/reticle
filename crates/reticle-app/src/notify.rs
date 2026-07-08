//! The app-wide notification (toast) queue: the one human-readable surface every
//! failure path reports through.
//!
//! Reticle has many places a thing can go wrong: an import can reject a file, an
//! opened file can carry non-fatal warnings, a session can fail to load, a
//! technology file can fail to parse. Before this module some of those paths were
//! silent or console-only. This module is the single sink they all route through:
//! [`App::report_error`](crate::app::App::report_error) and its siblings push a
//! [`Notification`] onto a [`Notifications`] queue, and a small egui toast area (in
//! [`crate::app`]) draws whatever is queued. No failure is silent, and no failure is
//! only in the console.
//!
//! # Why it lives here (pure and egui-free)
//!
//! Like [`crate::tour`] and [`crate::open`], this module is deliberately window-free:
//! it is the notification *model* (a bounded queue of severity-tagged messages, each
//! with a one-line summary and an optional longer detail, plus the age/dismiss
//! bookkeeping) with no `egui`, no GPU, and no filesystem. That is what lets the
//! whole surface be unit-tested in plain code: a caller can trigger an import error,
//! assert a notification was queued, and read its severity and text, all without a
//! window. The egui glue (drawing the toast stack, the dismiss button, fading an
//! expired toast) is a thin layer in [`crate::app`] that reads this queue each frame.
//!
//! It compiles unchanged on `wasm32` (no platform types cross its surface), so the
//! web bundle reports errors through exactly the same path as the desktop app.
//!
//! # Concurrency note (for sibling lanes)
//!
//! Several first-contact features report failures through this one sink: the Start
//! screen's open and example gallery, the browser-file open path, and the share
//! section. Each calls [`App::report_error`](crate::app::App::report_error) (or
//! [`App::notify`](crate::app::App::notify) for a non-error notice) rather than
//! printing or swallowing, so the surfaces converge on a single, consistent toast
//! area.

/// The structured explanation attached to a failure: what went wrong, what to do
/// next, and a copyable technical block for a bug report.
///
/// Catalog item 72 requires that *every* failure path show a cause, a next step,
/// and a copyable diagnostic block (never a console-only error). A [`Diagnostic`]
/// is the pure carrier of those three strings; the toast glue in [`crate::app`]
/// renders the cause and next step inline and offers a Copy-details action that
/// copies [`clipboard_text`](Diagnostic::clipboard_text). Kept `egui`-free so the
/// wording and the clipboard rendering are unit-tested in plain code.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Diagnostic {
    /// What went wrong, in plain language a non-expert can act on.
    pub cause: String,
    /// The next step the user can take (an alternative path, a fix, a workaround).
    pub next_step: String,
    /// A technical block (request, status, versions, stack) for a bug report, or
    /// empty when the cause and next step already say everything.
    pub details: String,
}

impl Diagnostic {
    /// A diagnostic with a `cause`, a `next_step`, and an optional technical
    /// `details` block.
    #[must_use]
    pub fn new(
        cause: impl Into<String>,
        next_step: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            cause: cause.into(),
            next_step: next_step.into(),
            details: details.into(),
        }
    }

    /// Whether a technical details block is present beyond the cause and next step.
    #[must_use]
    pub fn has_details(&self) -> bool {
        !self.details.is_empty()
    }

    /// The full copyable block a user pastes into a bug report: the cause, the next
    /// step, and the technical details, each labeled so the paste is self-describing.
    ///
    /// Labeled and newline-separated (never em-dash-joined, per the style gate) so a
    /// maintainer reading a pasted block sees exactly what the user saw.
    #[must_use]
    pub fn clipboard_text(&self) -> String {
        let mut out = format!("Cause: {}\nNext step: {}", self.cause, self.next_step);
        if self.has_details() {
            out.push_str("\n\nDetails:\n");
            out.push_str(&self.details);
        }
        out
    }
}

/// How serious a [`Notification`] is, which selects its toast color and icon.
///
/// The variants are ordered least-to-most severe so they derive a sensible
/// [`Ord`]; the app maps each to a color in its egui glue.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Severity {
    /// A neutral, informational notice (for example, "opened document (4 cells)").
    Info,
    /// A recoverable problem: the operation mostly succeeded but something was
    /// skipped, clamped, or defaulted (for example, an import warning).
    Warning,
    /// A hard failure: the operation could not complete (for example, a file that
    /// is not the claimed format, or a session that failed to load).
    Error,
}

impl Severity {
    /// A short, stable label for the severity, used as the toast's leading tag and
    /// handy in tests.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "Info",
            Severity::Warning => "Warning",
            Severity::Error => "Error",
        }
    }
}

/// An action button a [`Notification`] can carry (catalog item 71): the app maps
/// each to a [`theme::components::Button`](crate::theme::components::Button) and
/// interprets a click.
///
/// Kept a small closed enum (rather than a boxed callback) so the model stays
/// `egui`-free and unit-testable: the toast glue in [`crate::app`] renders a button
/// per action and, on click, runs the matching effect (re-run the operation, copy
/// the diagnostic block, undo the reversible action).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum NotificationAction {
    /// Re-run the failed operation (a failed open, a failed share mint).
    Retry,
    /// Copy the [`Diagnostic`] block to the clipboard for a bug report (item 72).
    CopyDetails,
    /// Undo a just-performed reversible action (do-then-Undo, item 78).
    Undo,
    /// Fit the freshly opened document to the view (the post-open summary toast,
    /// item 7).
    Fit,
}

impl NotificationAction {
    /// The button label shown for this action.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            NotificationAction::Retry => "Retry",
            NotificationAction::CopyDetails => "Copy details",
            NotificationAction::Undo => "Undo",
            NotificationAction::Fit => "Fit",
        }
    }
}

/// One entry in the notification queue: a severity, a one-line summary, and an
/// optional longer detail.
///
/// Plain owned data so it lives freely in the UI and crosses the wasm boundary. The
/// `summary` is always shown; the `detail` (when non-empty) is shown expanded or on
/// hover by the egui glue. `seen_secs` accumulates the wall-clock time the toast has
/// been on screen so the app can auto-expire an informational toast; an error stays
/// until the user dismisses it.
// `seen_secs` is an `f32`, so the struct is `PartialEq` but not `Eq`.
#[derive(Clone, PartialEq, Debug)]
pub struct Notification {
    /// How serious this notification is (selects color and dismiss behavior).
    pub severity: Severity,
    /// A short, human-readable one-liner naming what happened.
    pub summary: String,
    /// A longer explanation, or empty when the summary says it all.
    pub detail: String,
    /// Seconds this toast has been shown, accumulated by [`Notifications::advance`].
    /// Used to auto-expire non-error toasts; starts at zero.
    pub seen_secs: f32,
    /// The action buttons this toast offers (item 71), in display order. Empty for a
    /// plain notice.
    pub actions: Vec<NotificationAction>,
    /// The structured cause / next-step / copyable diagnostic (item 72), when this
    /// notification carries one (failures do; plain notices do not).
    pub diagnostic: Option<Diagnostic>,
}

impl Notification {
    /// A notification with a summary and a detail at the given severity, no actions
    /// and no diagnostic.
    #[must_use]
    pub fn new(severity: Severity, summary: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            severity,
            summary: summary.into(),
            detail: detail.into(),
            seen_secs: 0.0,
            actions: Vec::new(),
            diagnostic: None,
        }
    }

    /// Adds an action button to this notification (builder; keeps insertion order).
    #[must_use]
    pub fn with_action(mut self, action: NotificationAction) -> Self {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
        self
    }

    /// Attaches a [`Diagnostic`] (cause, next step, copyable block) to this
    /// notification (item 72).
    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostic = Some(diagnostic);
        self
    }

    /// Whether this notification has a longer detail beyond its summary.
    #[must_use]
    pub fn has_detail(&self) -> bool {
        !self.detail.is_empty()
    }

    /// Whether this notification offers the given action.
    #[must_use]
    pub fn has_action(&self, action: NotificationAction) -> bool {
        self.actions.contains(&action)
    }
}

/// The number of seconds an auto-expiring (non-error) toast stays on screen before
/// [`Notifications::advance`] drops it. Errors never auto-expire; the user dismisses
/// them so a failure is never missed.
const AUTO_DISMISS_SECS: f32 = 6.0;

/// The most notifications kept at once. Older ones are dropped when the cap is
/// exceeded, so a burst of failures cannot grow the queue without bound.
const MAX_QUEUED: usize = 6;

/// The most notifications retained in the notification-center history (item 69).
/// Once a toast leaves the live queue (dismissed, expired, cleared, or trimmed) it
/// is archived here, newest last, so the center can show recent activity; the ring
/// is bounded so a long session cannot grow it without limit.
const HISTORY_CAP: usize = 50;

/// A bounded, severity-tagged queue of notifications: the app's single error and
/// notice surface.
///
/// Push with [`push`](Notifications::push) (or the app's `report_error`/`notify`
/// wrappers), read the live list with [`iter`](Notifications::iter) to draw the
/// toasts, age them each frame with [`advance`](Notifications::advance) (which drops
/// expired non-error toasts), and remove one the user closed with
/// [`dismiss`](Notifications::dismiss). The newest notification is last.
#[derive(Clone, Debug, Default)]
pub struct Notifications {
    /// The live queue, oldest first. Capped at [`MAX_QUEUED`].
    items: Vec<Notification>,
    /// The notification-center history, newest last. Capped at [`HISTORY_CAP`]. A
    /// toast is archived here as it leaves the live queue (item 69).
    history: Vec<Notification>,
}

impl Notifications {
    /// An empty notification queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a notification, trimming the oldest if the cap is exceeded.
    ///
    /// Returns the index of the pushed notification in the live queue (after any
    /// trim), so a caller can reference it if needed.
    pub fn push(&mut self, note: Notification) {
        self.items.push(note);
        // Drop the oldest entries if we are over the cap, keeping the newest
        // `MAX_QUEUED`. A flurry of errors cannot grow the queue without bound.
        // Trimmed entries were still shown, so they are archived to the history.
        if self.items.len() > MAX_QUEUED {
            let overflow = self.items.len() - MAX_QUEUED;
            let trimmed: Vec<Notification> = self.items.drain(0..overflow).collect();
            for note in trimmed {
                self.archive(note);
            }
        }
    }

    /// Reports a rich failure (item 72): an [`Severity::Error`] notification carrying
    /// a [`Diagnostic`] (cause, next step, copyable block) and a
    /// [`Copy details`](NotificationAction::CopyDetails) action, so no failure is
    /// silent, console-only, or missing a next step.
    ///
    /// This is the single rich-failure entry point every open/URL/share/convert path
    /// routes through; the plain [`error`](Self::error) remains for a failure whose
    /// summary and detail already say everything.
    pub fn fail(&mut self, summary: impl Into<String>, diagnostic: Diagnostic) {
        let note = Notification::new(Severity::Error, summary, diagnostic.next_step.clone())
            .with_diagnostic(diagnostic)
            .with_action(NotificationAction::CopyDetails);
        self.push(note);
    }

    /// Reports a completed reversible action as a do-then-Undo notice (item 78): an
    /// informational toast carrying an [`Undo`](NotificationAction::Undo) action,
    /// replacing a blocking confirmation dialog. The action already happened; the
    /// user can undo it from the toast while it is on screen.
    pub fn undoable(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        let note = Notification::new(Severity::Info, summary, detail)
            .with_action(NotificationAction::Undo);
        self.push(note);
    }

    /// Archives a notification into the bounded history as it leaves the live queue.
    fn archive(&mut self, note: Notification) {
        self.history.push(note);
        if self.history.len() > HISTORY_CAP {
            let overflow = self.history.len() - HISTORY_CAP;
            self.history.drain(0..overflow);
        }
    }

    /// The notification-center history, oldest first, newest last (item 69).
    #[must_use]
    pub fn history(&self) -> &[Notification] {
        &self.history
    }

    /// Empties the notification-center history (a user "clear history").
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Reports a hard failure: pushes an [`Severity::Error`] notification.
    ///
    /// This is the workhorse the app's `report_error` wraps; every failure path
    /// converges here so no error is silent or console-only.
    pub fn error(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        self.push(Notification::new(Severity::Error, summary, detail));
    }

    /// Reports a recoverable problem: pushes an [`Severity::Warning`] notification.
    pub fn warning(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        self.push(Notification::new(Severity::Warning, summary, detail));
    }

    /// Reports a neutral notice: pushes an [`Severity::Info`] notification.
    pub fn info(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        self.push(Notification::new(Severity::Info, summary, detail));
    }

    /// The live notifications, oldest first, for the toast area to draw.
    pub fn iter(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter()
    }

    /// How many notifications are currently queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue is empty (nothing to draw).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The most severe severity currently queued, or `None` when empty.
    ///
    /// Lets the app color a summary badge (for example a header dot) by the worst
    /// pending notification without scanning the list itself.
    #[must_use]
    pub fn max_severity(&self) -> Option<Severity> {
        self.items.iter().map(|n| n.severity).max()
    }

    /// Ages every toast by `dt` seconds and drops the non-error ones that have been
    /// shown past the auto-dismiss window.
    ///
    /// Errors never expire here; they persist until [`dismiss`](Notifications::dismiss)
    /// or [`clear`](Notifications::clear) removes them, so a failure is never lost to
    /// a timeout. Call once per frame with the frame's delta time.
    pub fn advance(&mut self, dt: f32) {
        for n in &mut self.items {
            n.seen_secs += dt.max(0.0);
        }
        // Partition: expired non-error toasts leave the live queue and are archived.
        let mut expired = Vec::new();
        self.items.retain(|n| {
            let keep = n.severity == Severity::Error || n.seen_secs < AUTO_DISMISS_SECS;
            if !keep {
                expired.push(n.clone());
            }
            keep
        });
        for note in expired {
            self.archive(note);
        }
    }

    /// Removes the notification at `index` (a user dismissal), archiving it into the
    /// history. Out-of-range indices are ignored so a stale click cannot panic.
    pub fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            let note = self.items.remove(index);
            self.archive(note);
        }
    }

    /// Clears every live notification (dismiss all), archiving each into the history.
    pub fn clear(&mut self) {
        let cleared = std::mem::take(&mut self.items);
        for note in cleared {
            self.archive(note);
        }
    }
}

/// Whether the app currently has a live connection to the share relay.
///
/// Drives the offline badge and the reconnect toasts (item 74). The transport in
/// [`crate::livesync`] reports socket open/close, which
/// [`ConnectivityState`] turns into at-most-one toast per real transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Connectivity {
    /// Connected (or never went offline): no badge.
    #[default]
    Online,
    /// The connection dropped: show the offline badge and, on the edge, a toast.
    Offline,
}

/// Tracks online/offline state so the reconnect toasts fire once per real
/// transition, not on every socket heartbeat (item 74).
///
/// Pure: the wasm socket callbacks call [`set_offline`](ConnectivityState::set_offline)
/// / [`set_online`](ConnectivityState::set_online) and the app pushes any returned
/// [`Notification`], so the "warn once when we drop, reassure once when we return"
/// policy is unit-tested without a socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ConnectivityState {
    current: Connectivity,
}

impl ConnectivityState {
    /// A fresh state, assumed online.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current connectivity.
    #[must_use]
    pub fn current(self) -> Connectivity {
        self.current
    }

    /// Records that the connection dropped. Returns a warning [`Notification`] on the
    /// online -> offline edge, or `None` if already offline (no repeat toast).
    pub fn set_offline(&mut self) -> Option<Notification> {
        if self.current == Connectivity::Offline {
            return None;
        }
        self.current = Connectivity::Offline;
        Some(Notification::new(
            Severity::Warning,
            "You are offline",
            "Live sharing is paused. Reticle will reconnect automatically when the \
             connection returns.",
        ))
    }

    /// Records that the connection returned. Returns an informational
    /// [`Notification`] on the offline -> online edge, or `None` if already online.
    pub fn set_online(&mut self) -> Option<Notification> {
        if self.current == Connectivity::Online {
            return None;
        }
        self.current = Connectivity::Online;
        Some(Notification::new(
            Severity::Info,
            "Reconnected",
            "Live sharing has resumed.",
        ))
    }

    /// The offline-badge label to show in the status bar, or `None` while online.
    #[must_use]
    pub fn badge_label(self) -> Option<&'static str> {
        match self.current {
            Connectivity::Offline => Some("Offline"),
            Connectivity::Online => None,
        }
    }
}

/// Seconds a task must run before its status-bar spinner appears (item 73).
const SPINNER_AFTER_SECS: f32 = 0.3;
/// Seconds a task must run before it earns a cancelable progress toast (item 73).
const PROGRESS_AFTER_SECS: f32 = 2.0;

/// How prominently an in-flight long task is surfaced, by how long it has run
/// (item 73): nothing under 300 ms, a status-bar spinner past 300 ms, a cancelable
/// progress toast past 2 s. Encoding the thresholds here keeps the "don't flash a
/// spinner for an instant task, don't leave a long task silent" policy in one
/// unit-tested place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStage {
    /// Under the spinner threshold: show nothing (the task will likely finish first).
    Hidden,
    /// Past 300 ms: a quiet status-bar spinner so the user knows work is underway.
    Spinner,
    /// Past 2 s: a progress toast with a cancel affordance.
    Progress,
}

impl TaskStage {
    /// The stage for a task that has run `elapsed` seconds.
    #[must_use]
    pub fn for_elapsed(elapsed: f32) -> Self {
        if elapsed >= PROGRESS_AFTER_SECS {
            TaskStage::Progress
        } else if elapsed >= SPINNER_AFTER_SECS {
            TaskStage::Spinner
        } else {
            TaskStage::Hidden
        }
    }

    /// Whether this stage warrants a cancel affordance (only the progress toast does).
    #[must_use]
    pub fn shows_cancel(self) -> bool {
        matches!(self, TaskStage::Progress)
    }
}

/// A running long task tracked for the long-task surfacing pattern (item 73): a
/// label and the elapsed time, from which the [`TaskStage`] is derived.
///
/// The app advances one of these per frame while a cancelable operation (a staged
/// open, a convert, a share mint) runs, and reads [`stage`](LongTask::stage) to
/// decide whether to show nothing, a spinner, or a cancelable progress toast.
#[derive(Clone, Debug, PartialEq)]
pub struct LongTask {
    label: String,
    elapsed: f32,
}

impl LongTask {
    /// A task labeled `label` at zero elapsed time.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            elapsed: 0.0,
        }
    }

    /// Adds `dt` seconds (a frame's delta) to the elapsed time.
    pub fn advance(&mut self, dt: f32) {
        self.elapsed += dt.max(0.0);
    }

    /// The current surfacing stage for the elapsed time.
    #[must_use]
    pub fn stage(&self) -> TaskStage {
        TaskStage::for_elapsed(self.elapsed)
    }

    /// The task's label (for the spinner tooltip and the progress toast).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_clipboard_block_carries_cause_next_step_and_details() {
        // Item 72: every failure shows a cause, a next step, and a copyable
        // technical block. The clipboard rendering folds all three into one block a
        // user can paste into a bug report.
        let d = Diagnostic::new(
            "The server did not allow a cross-origin request (CORS).",
            "Host the file with permissive CORS headers, or open it in the desktop app.",
            "GET https://host/chip.gds\nstatus: (opaque)\norigin: https://reticle.example",
        );
        let block = d.clipboard_text();
        assert!(block.contains("CORS"), "cause is present: {block}");
        assert!(block.contains("Next step"), "labels the next step: {block}");
        assert!(
            block.contains("desktop app"),
            "next step text is present: {block}"
        );
        assert!(
            block.contains("status: (opaque)"),
            "the technical details are present: {block}"
        );
        // The block is em-dash free (the style gate) and non-empty.
        assert!(!block.contains('\u{2014}'));
        assert!(d.has_details());
    }

    #[test]
    fn diagnostic_without_details_still_renders_cause_and_next_step() {
        let d = Diagnostic::new("Cause line.", "Do this next.", "");
        assert!(!d.has_details());
        let block = d.clipboard_text();
        assert!(block.contains("Cause line."));
        assert!(block.contains("Do this next."));
    }

    #[test]
    fn actions_have_distinct_nonempty_labels() {
        // Item 71: a toast can carry action buttons (Retry, Copy details, Undo).
        let all = [
            NotificationAction::Retry,
            NotificationAction::CopyDetails,
            NotificationAction::Undo,
            NotificationAction::Fit,
        ];
        let mut seen = std::collections::HashSet::new();
        for a in all {
            let label = a.label();
            assert!(!label.is_empty());
            assert!(!label.contains('\u{2014}'), "no em-dash in {label}");
            assert!(seen.insert(label), "labels are distinct: {label}");
        }
    }

    #[test]
    fn fail_queues_an_error_with_diagnostic_and_a_copy_action() {
        // Item 72: every failure carries a cause, a next step, and a copyable
        // diagnostic. `fail` is the single rich-failure entry point: an Error
        // notification whose diagnostic is attached and whose Copy-details action is
        // present so the block can be lifted into a bug report.
        let mut n = Notifications::new();
        let diag = Diagnostic::new("It broke.", "Try again.", "trace: ...");
        n.fail("Could not open the file", diag.clone());
        let note = n.iter().next().expect("one notification");
        assert_eq!(note.severity, Severity::Error);
        assert_eq!(note.diagnostic.as_ref(), Some(&diag));
        assert!(
            note.actions.contains(&NotificationAction::CopyDetails),
            "a failure offers Copy details"
        );
    }

    #[test]
    fn undoable_queues_an_info_notice_with_an_undo_action() {
        // Item 78: a reversible action reports as a do-then-Undo toast rather than a
        // blocking confirmation. The notice is informational (the action already
        // happened) and carries an Undo action the app wires to its undo stack.
        let mut n = Notifications::new();
        n.undoable("Deleted 3 shapes", "");
        let note = n.iter().next().expect("one notification");
        assert_eq!(note.severity, Severity::Info);
        assert!(note.actions.contains(&NotificationAction::Undo));
    }

    #[test]
    fn dismissed_and_expired_toasts_are_retained_in_history() {
        // Item 69: a notification center keeps recent toasts after they leave the
        // live queue, so a toast that auto-expired is still reviewable.
        let mut n = Notifications::new();
        n.info("opened document", "");
        n.error("boom", "");
        assert!(n.history().is_empty(), "nothing archived while still live");
        // Expire the info toast: it leaves the live queue but is archived.
        n.advance(AUTO_DISMISS_SECS + 1.0);
        assert_eq!(n.len(), 1, "the error is still live");
        assert!(
            n.history().iter().any(|h| h.summary == "opened document"),
            "the expired info toast is in history"
        );
        // Dismissing the error also archives it.
        n.dismiss(0);
        assert!(n.history().iter().any(|h| h.summary == "boom"));
    }

    #[test]
    fn history_is_bounded() {
        let mut n = Notifications::new();
        for i in 0..(HISTORY_CAP + 10) {
            n.info(format!("note {i}"), "");
            n.advance(AUTO_DISMISS_SECS + 1.0); // expire it straight into history
        }
        assert!(
            n.history().len() <= HISTORY_CAP,
            "history never grows unbounded"
        );
        // The newest is retained; the oldest fell off.
        assert!(
            n.history()
                .iter()
                .any(|h| h.summary == format!("note {}", HISTORY_CAP + 9))
        );
    }

    #[test]
    fn connectivity_transitions_produce_reconnect_and_offline_toasts() {
        // Item 74: an offline badge plus reconnect toasts, wired to the live
        // reconnect states. Going offline warns; coming back reassures. A repeated
        // same-state update produces no toast (no spam on every socket heartbeat).
        let mut c = ConnectivityState::new();
        assert_eq!(c.current(), Connectivity::Online);
        assert!(c.set_offline().is_some(), "first drop warns");
        assert!(c.set_offline().is_none(), "still offline, no repeat toast");
        assert_eq!(c.current(), Connectivity::Offline);
        let back = c.set_online().expect("reconnect reassures");
        assert_eq!(back.severity, Severity::Info);
        assert!(c.set_online().is_none(), "already online, no repeat toast");
        // The badge label is present only while offline.
        c.set_offline();
        assert!(c.badge_label().is_some());
        c.set_online();
        assert!(c.badge_label().is_none());
    }

    #[test]
    fn long_task_stage_follows_the_300ms_and_2s_thresholds() {
        // Item 73: under 300 ms show nothing, past 300 ms a status-bar spinner, past
        // 2 s a progress toast with cancel.
        assert_eq!(TaskStage::for_elapsed(0.0), TaskStage::Hidden);
        assert_eq!(TaskStage::for_elapsed(0.29), TaskStage::Hidden);
        assert_eq!(TaskStage::for_elapsed(0.30), TaskStage::Spinner);
        assert_eq!(TaskStage::for_elapsed(1.9), TaskStage::Spinner);
        assert_eq!(TaskStage::for_elapsed(2.0), TaskStage::Progress);
        // Only the progress stage warrants a cancel affordance.
        assert!(!TaskStage::Hidden.shows_cancel());
        assert!(!TaskStage::Spinner.shows_cancel());
        assert!(TaskStage::Progress.shows_cancel());
    }

    #[test]
    fn long_task_accumulates_elapsed_time() {
        let mut t = LongTask::new("Opening chip.gds");
        assert_eq!(t.stage(), TaskStage::Hidden);
        t.advance(0.2);
        assert_eq!(t.stage(), TaskStage::Hidden);
        t.advance(0.2); // 0.4 total
        assert_eq!(t.stage(), TaskStage::Spinner);
        t.advance(2.0); // 2.4 total
        assert_eq!(t.stage(), TaskStage::Progress);
        assert_eq!(t.label(), "Opening chip.gds");
    }

    #[test]
    fn severity_orders_info_lt_warning_lt_error() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        // Labels are present, distinct, and em-dash-free (the style gate).
        for s in [Severity::Info, Severity::Warning, Severity::Error] {
            assert!(!s.label().is_empty());
            assert!(!s.label().contains('\u{2014}'));
        }
    }

    #[test]
    fn error_queues_a_readable_error_notification() {
        // The core promise: reporting an error queues one, and it reads back with
        // the summary, detail, and Error severity intact (no silent failure).
        let mut n = Notifications::new();
        assert!(n.is_empty());
        n.error("could not open GDSII file", "not the claimed format");
        assert_eq!(n.len(), 1);
        let note = n.iter().next().expect("one notification");
        assert_eq!(note.severity, Severity::Error);
        assert_eq!(note.summary, "could not open GDSII file");
        assert_eq!(note.detail, "not the claimed format");
        assert!(note.has_detail());
        assert_eq!(n.max_severity(), Some(Severity::Error));
    }

    #[test]
    fn info_and_warning_go_through_the_same_queue() {
        let mut n = Notifications::new();
        n.info("opened document", "");
        n.warning("skipped a degenerate shape", "2-vertex boundary dropped");
        assert_eq!(n.len(), 2);
        // The worst pending severity is the warning.
        assert_eq!(n.max_severity(), Some(Severity::Warning));
        // The info one carries no detail.
        let first = n.iter().next().unwrap();
        assert_eq!(first.severity, Severity::Info);
        assert!(!first.has_detail());
    }

    #[test]
    fn the_queue_is_bounded_and_keeps_the_newest() {
        let mut n = Notifications::new();
        for i in 0..(MAX_QUEUED + 3) {
            n.error(format!("error {i}"), "");
        }
        // Never grows past the cap.
        assert_eq!(n.len(), MAX_QUEUED);
        // The newest survivors are kept: the last one pushed is still present.
        let last = n.iter().last().unwrap();
        assert_eq!(last.summary, format!("error {}", MAX_QUEUED + 2));
        // And the very first (oldest) was trimmed.
        assert!(!n.iter().any(|x| x.summary == "error 0"));
    }

    #[test]
    fn advance_expires_non_errors_but_keeps_errors() {
        let mut n = Notifications::new();
        n.info("transient notice", "");
        n.error("persistent failure", "stays until dismissed");
        // Age past the auto-dismiss window.
        n.advance(AUTO_DISMISS_SECS + 1.0);
        // The info toast is gone; the error remains.
        assert_eq!(n.len(), 1);
        let remaining = n.iter().next().unwrap();
        assert_eq!(remaining.severity, Severity::Error);
        assert_eq!(remaining.summary, "persistent failure");
    }

    #[test]
    fn advance_below_the_window_keeps_everything() {
        let mut n = Notifications::new();
        n.info("a", "");
        n.warning("b", "");
        n.advance(AUTO_DISMISS_SECS / 2.0);
        assert_eq!(n.len(), 2, "nothing expires before the window");
        // A second small step still keeps them (accumulated, but under the window).
        n.advance(AUTO_DISMISS_SECS / 3.0);
        assert_eq!(n.len(), 2);
    }

    #[test]
    fn dismiss_removes_one_and_ignores_out_of_range() {
        let mut n = Notifications::new();
        n.error("first", "");
        n.error("second", "");
        n.dismiss(0);
        assert_eq!(n.len(), 1);
        assert_eq!(n.iter().next().unwrap().summary, "second");
        // An out-of-range dismissal is a no-op, not a panic.
        n.dismiss(99);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn clear_empties_the_queue() {
        let mut n = Notifications::new();
        n.error("boom", "");
        n.warning("careful", "");
        assert!(!n.is_empty());
        n.clear();
        assert!(n.is_empty());
        assert_eq!(n.max_severity(), None);
    }

    #[test]
    fn advance_ignores_negative_dt() {
        // A pathological negative dt must not un-age or panic.
        let mut n = Notifications::new();
        n.info("x", "");
        n.advance(-100.0);
        assert_eq!(n.len(), 1, "negative dt neither expires nor un-ages");
    }
}

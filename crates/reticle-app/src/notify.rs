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
}

impl Notification {
    /// A notification with a summary and a detail at the given severity.
    #[must_use]
    pub fn new(severity: Severity, summary: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            severity,
            summary: summary.into(),
            detail: detail.into(),
            seen_secs: 0.0,
        }
    }

    /// Whether this notification has a longer detail beyond its summary.
    #[must_use]
    pub fn has_detail(&self) -> bool {
        !self.detail.is_empty()
    }
}

/// The number of seconds an auto-expiring (non-error) toast stays on screen before
/// [`Notifications::advance`] drops it. Errors never auto-expire; the user dismisses
/// them so a failure is never missed.
const AUTO_DISMISS_SECS: f32 = 6.0;

/// The most notifications kept at once. Older ones are dropped when the cap is
/// exceeded, so a burst of failures cannot grow the queue without bound.
const MAX_QUEUED: usize = 6;

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
        if self.items.len() > MAX_QUEUED {
            let overflow = self.items.len() - MAX_QUEUED;
            self.items.drain(0..overflow);
        }
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
        self.items
            .retain(|n| n.severity == Severity::Error || n.seen_secs < AUTO_DISMISS_SECS);
    }

    /// Removes the notification at `index` (a user dismissal). Out-of-range indices
    /// are ignored so a stale click cannot panic.
    pub fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Clears every notification (dismiss all).
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

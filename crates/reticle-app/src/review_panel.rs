//! The design-review panel: per-thread review verdicts and the approve action.
//!
//! A lightweight design-review workflow on top of the shipped collaboration core.
//! The panel groups the layout's [`Comment`]s into threads, shows each thread's
//! derived [`ReviewState`] (open, approved, or changes requested), and offers
//! Approve / Request changes / Reopen. As in [`crate::drc_panel`] and
//! [`crate::comment_pins`], the interesting logic here, grouping comments into
//! threads, formatting a row, and producing a review-action comment, is plain
//! functions tested without an egui context; the [`ReviewPanel::show`] method is the
//! thin egui glue and is styled entirely from [`crate::theme`].
//!
//! # Riding the frozen wire
//!
//! A review verdict is NOT a new wire field. A recorded action is an ordinary
//! [`Comment`] (a reply in the thread) whose body is the marker `"[review:<tag>]"`
//! (see [`reticle_sync::Comment::review_action`]); the thread's verdict is derived
//! from those actions ([`CommentThread::review_state`]). So an approval persists in
//! the schema-V2 `comments` field and travels on the existing comment frame, and the
//! relay protocol, proto frames, and wire format stay frozen. The panel produces the
//! action comment; the app appends it exactly as it appends a typed comment, so it
//! rides whatever persistence and sync a comment already rides.

use std::collections::BTreeMap;

use eframe::egui;

use reticle_sync::{Comment, CommentThread, ReviewAction, ReviewState};

use crate::theme::components::{self, Ctx};

/// The review panel's state: whether its window is shown, the actor recorded as the
/// author of produced review actions, and a per-session id counter.
///
/// The panel is inert while [`open`](ReviewPanel::open) is false. Activation (a menu
/// item or the review command's shortcut) flips that flag; the flag is the seam the
/// separately-wired activation drives, so this module carries no command wiring.
#[derive(Clone, Debug)]
pub struct ReviewPanel {
    /// Whether the floating review window is shown.
    pub open: bool,
    /// The actor id recorded as the author of review actions this panel produces.
    author: String,
    /// A per-session counter making each produced review-action comment id unique.
    seq: u64,
}

impl Default for ReviewPanel {
    fn default() -> Self {
        Self {
            open: false,
            // Mirrors the comment panel's author for a locally-added annotation.
            author: "you".to_owned(),
            seq: 0,
        }
    }
}

impl ReviewPanel {
    /// Creates a closed review panel.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows or hides the panel window.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draws the review panel window (a no-op while closed) and returns a produced
    /// review-action comment when the reviewer records a verdict this frame.
    ///
    /// The returned [`Comment`] is an ordinary review-action reply; the caller
    /// appends it to the layout's comment set, so it persists and syncs exactly as a
    /// typed comment does (see the module docs). `now_ms` is the session clock
    /// (wasm-safe, from egui input) stamped on the produced action.
    pub fn show(
        &mut self,
        egui_ctx: &egui::Context,
        comp: Ctx,
        comments: &[Comment],
        now_ms: i64,
    ) -> Option<Comment> {
        if !self.open {
            return None;
        }
        let threads = review_threads(comments);
        let author = self.author.clone();
        let seq = self.seq;
        let mut produced: Option<(usize, ReviewAction)> = None;
        let mut open = true;
        egui::Window::new("Review")
            .id(egui::Id::new("review_panel_window"))
            .open(&mut open)
            .resizable(true)
            .default_width(320.0)
            .show(egui_ctx, |ui| {
                if threads.is_empty() {
                    components::EmptyState::new(
                        "No comment threads",
                        "Comment on a cell to open a thread, then approve or request changes here.",
                    )
                    .show(ui, comp);
                    return;
                }
                for (i, thread) in threads.iter().enumerate() {
                    if let Some(action) = draw_thread_row(ui, comp, thread) {
                        produced = Some((i, action));
                    }
                    ui.separator();
                }
            });
        self.open = open;

        let (index, action) = produced?;
        let id = review_action_id(seq, index, now_ms);
        let comment = review_action_comment(&threads[index], &author, action, id, now_ms)?;
        self.seq = self.seq.wrapping_add(1);
        Some(comment)
    }
}

/// Groups a flat comment slice into per-thread [`CommentThread`]s.
///
/// Each thread is ordered by [`CommentThread::from_comments`] (root first, then by
/// creation time), and the threads are returned sorted for display by their root's
/// creation time then thread id, so the list is stable across frames.
#[must_use]
pub fn review_threads(comments: &[Comment]) -> Vec<CommentThread> {
    let mut groups: BTreeMap<&str, Vec<Comment>> = BTreeMap::new();
    for c in comments {
        groups
            .entry(c.thread_id.as_str())
            .or_default()
            .push(c.clone());
    }
    let mut threads: Vec<CommentThread> = groups
        .into_values()
        .map(CommentThread::from_comments)
        .collect();
    threads.sort_by_key(|t| t.root().map(|r| (r.created_unix_ms, r.thread_id.clone())));
    threads
}

/// Builds a review-action comment recording `action` on `thread` by `author`, or
/// `None` if the thread has no root comment to reply to.
#[must_use]
pub fn review_action_comment(
    thread: &CommentThread,
    author: &str,
    action: ReviewAction,
    id: impl Into<String>,
    now_ms: i64,
) -> Option<Comment> {
    let root = thread.root()?;
    Some(Comment::review_action(root, id, author, action, now_ms))
}

/// A one-line title for a thread's row: its anchor and a single-line preview of the
/// root comment's body (newlines collapsed to spaces).
#[must_use]
pub fn thread_title(root: &Comment) -> String {
    let body = root.body.replace('\n', " ");
    format!("@{}  {}", root.anchor_ref, body)
}

/// A per-session-unique id for a produced review-action comment.
fn review_action_id(seq: u64, thread_index: usize, now_ms: i64) -> String {
    format!("review-{seq}-{thread_index}-{now_ms}")
}

/// The token color for a review verdict: success (approved), warning (changes
/// requested), or the weak text color (open, no verdict yet).
fn state_color(comp: Ctx, state: ReviewState) -> egui::Color32 {
    match state {
        ReviewState::Approved => comp.tokens.success,
        ReviewState::ChangesRequested => comp.tokens.warning,
        ReviewState::Open => comp.tokens.text_weak,
    }
}

/// Draws one thread's review row: the anchor and root preview, the current verdict
/// as a token-colored label with who set it, and the Approve / Request changes /
/// Reopen actions. Returns the action the reviewer clicked, if any.
fn draw_thread_row(ui: &mut egui::Ui, comp: Ctx, thread: &CommentThread) -> Option<ReviewAction> {
    let state = thread.review_state();
    let mut clicked = None;

    if let Some(root) = thread.root() {
        ui.label(egui::RichText::new(thread_title(root)).color(comp.tokens.text));
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Status:").color(comp.tokens.text_weak));
        ui.colored_label(state_color(comp, state), state.label());
        if let Some(last) = thread.last_review_action() {
            ui.label(
                egui::RichText::new(format!("by {}", last.author)).color(comp.tokens.text_weak),
            );
        }
    });
    ui.horizontal(|ui| {
        if components::Button::secondary("Approve")
            .show(ui, comp)
            .clicked()
        {
            clicked = Some(ReviewAction::Approve);
        }
        if components::Button::secondary("Request changes")
            .show(ui, comp)
            .clicked()
        {
            clicked = Some(ReviewAction::RequestChanges);
        }
        // Reopen is only meaningful once a verdict has been recorded.
        if state != ReviewState::Open
            && components::Button::ghost("Reopen").show(ui, comp).clicked()
        {
            clicked = Some(ReviewAction::Reopen);
        }
    });

    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root comment on `anchor` at time `ts`.
    fn root(id: &str, anchor: &str, ts: i64) -> Comment {
        Comment::root(id, anchor, "alice", "please review", ts)
    }

    #[test]
    fn review_threads_groups_and_orders_by_root_time() {
        // Two threads, fed interleaved and out of order; grouped by thread id and
        // ordered by root creation time (thread b's root is older).
        let root_a = root("a", "TOP", 20);
        let root_b = root("b", "SUB", 10);
        let reply_a = Comment::reply_to(&root_a, "a2", "bob", "a nit", 21);
        let comments = vec![reply_a, root_a.clone(), root_b.clone()];

        let threads = review_threads(&comments);
        assert_eq!(threads.len(), 2);
        // Thread b (root time 10) sorts before thread a (root time 20).
        assert_eq!(threads[0].root().unwrap().id, "b");
        assert_eq!(threads[1].root().unwrap().id, "a");
        // Thread a kept both of its comments.
        assert_eq!(threads[1].len(), 2);
    }

    #[test]
    fn produced_action_records_the_verdict_on_the_thread() {
        // Producing an Approve action and folding it back in flips the thread to
        // Approved, proving the panel's output drives the derived state.
        let r = root("t", "TOP", 1);
        let threads = review_threads(std::slice::from_ref(&r));
        let action = review_action_comment(&threads[0], "you", ReviewAction::Approve, "rv0", 5)
            .expect("thread has a root");
        assert_eq!(action.as_review_action(), Some(ReviewAction::Approve));

        let folded = review_threads(&[r, action]);
        assert_eq!(folded[0].review_state(), ReviewState::Approved);
    }

    #[test]
    fn review_action_comment_is_none_without_a_root() {
        // A thread of only replies (no root) cannot record a verdict.
        let orphan = Comment {
            id: "x".to_owned(),
            thread_id: "t".to_owned(),
            anchor_ref: "TOP".to_owned(),
            author: "bob".to_owned(),
            body: "reply".to_owned(),
            created_unix_ms: 2,
            in_reply_to: "missing".to_owned(),
        };
        let thread = CommentThread::from_comments(vec![orphan]);
        assert!(review_action_comment(&thread, "you", ReviewAction::Approve, "rv0", 3).is_none());
    }

    #[test]
    fn thread_title_carries_anchor_and_single_line_body() {
        let mut r = root("t", "TOP/shape-3", 1);
        r.body = "line one\nline two".to_owned();
        let title = thread_title(&r);
        assert!(title.contains("@TOP/shape-3"));
        assert!(title.contains("line one line two"));
        assert!(!title.contains('\n'));
    }

    #[test]
    fn review_action_ids_are_unique_per_thread_and_sequence() {
        assert_ne!(review_action_id(0, 0, 5), review_action_id(1, 0, 5));
        assert_ne!(review_action_id(0, 0, 5), review_action_id(0, 1, 5));
        assert_eq!(review_action_id(2, 3, 7), "review-2-3-7");
    }
}

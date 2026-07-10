//! Threaded comments anchored to a shape or cell.
//!
//! A [`Comment`] carries a stable `id`, the `thread_id` it belongs to, and an
//! `anchor_ref` pointing at the shape or cell it annotates. Replies set
//! `in_reply_to` to another comment's id, forming a thread. A [`CommentThread`]
//! groups a root comment with its replies for display.

use reticle_proto::v1;

/// A single threaded comment anchored to a document element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// Globally-unique comment id.
    pub id: String,
    /// Id shared by every comment in the same thread (usually the root's id).
    pub thread_id: String,
    /// Reference to the anchored shape or cell (for example a cell name or a
    /// `cell/shape-id` path).
    pub anchor_ref: String,
    /// Actor id of the comment's author.
    pub author: String,
    /// The comment text.
    pub body: String,
    /// Creation timestamp in Unix milliseconds.
    pub created_unix_ms: i64,
    /// Id of the comment this one replies to, or empty for a thread root.
    pub in_reply_to: String,
}

impl Comment {
    /// Creates a root comment (no parent) anchored to `anchor_ref`.
    ///
    /// The `thread_id` is set to the comment's own `id`, marking it as a thread
    /// root.
    #[must_use]
    pub fn root(
        id: impl Into<String>,
        anchor_ref: impl Into<String>,
        author: impl Into<String>,
        body: impl Into<String>,
        created_unix_ms: i64,
    ) -> Self {
        let id = id.into();
        Self {
            thread_id: id.clone(),
            id,
            anchor_ref: anchor_ref.into(),
            author: author.into(),
            body: body.into(),
            created_unix_ms,
            in_reply_to: String::new(),
        }
    }

    /// Creates a reply to `parent`, inheriting its thread and anchor.
    #[must_use]
    pub fn reply_to(
        parent: &Comment,
        id: impl Into<String>,
        author: impl Into<String>,
        body: impl Into<String>,
        created_unix_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            thread_id: parent.thread_id.clone(),
            anchor_ref: parent.anchor_ref.clone(),
            author: author.into(),
            body: body.into(),
            created_unix_ms,
            in_reply_to: parent.id.clone(),
        }
    }

    /// Returns `true` if this comment is the root of its thread.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.in_reply_to.is_empty()
    }

    /// Encodes this comment into its proto message form.
    #[must_use]
    pub fn to_proto(&self) -> v1::Comment {
        v1::Comment {
            id: self.id.clone(),
            thread_id: self.thread_id.clone(),
            anchor_ref: self.anchor_ref.clone(),
            author: self.author.clone(),
            body: self.body.clone(),
            created_unix_ms: self.created_unix_ms,
            in_reply_to: self.in_reply_to.clone(),
        }
    }

    /// Decodes a comment from its proto message form.
    #[must_use]
    pub fn from_proto(proto: &v1::Comment) -> Self {
        Self {
            id: proto.id.clone(),
            thread_id: proto.thread_id.clone(),
            anchor_ref: proto.anchor_ref.clone(),
            author: proto.author.clone(),
            body: proto.body.clone(),
            created_unix_ms: proto.created_unix_ms,
            in_reply_to: proto.in_reply_to.clone(),
        }
    }

    /// Wraps this comment in a [`v1::SyncMessage`] envelope ready to be sent on a
    /// live collaboration session.
    #[must_use]
    pub fn to_message(&self) -> v1::SyncMessage {
        v1::SyncMessage {
            payload: Some(v1::sync_message::Payload::Comment(self.to_proto())),
        }
    }
}

/// Encodes a slice of comments into their proto form, for persistence in a
/// schema-V2 `reticle_proto::v1::Document`'s `comments` field.
///
/// This is the inverse of [`from_proto_comments`]; together they carry the app's
/// live comment set into the versioned document and back without loss.
#[must_use]
pub fn to_proto_comments(comments: &[Comment]) -> Vec<v1::Comment> {
    comments.iter().map(Comment::to_proto).collect()
}

/// Decodes a slice of proto comments (as read from a schema-V2 document's
/// `comments` field) back into [`Comment`]s. The inverse of
/// [`to_proto_comments`].
#[must_use]
pub fn from_proto_comments(protos: &[v1::Comment]) -> Vec<Comment> {
    protos.iter().map(Comment::from_proto).collect()
}

/// A collection of comments grouped and ordered into a single display thread: the
/// root followed by its replies in creation order.
#[derive(Clone, Debug, Default)]
pub struct CommentThread {
    /// Every comment in the thread, root first, then replies by timestamp.
    pub comments: Vec<Comment>,
}

impl CommentThread {
    /// Builds an ordered thread from an unordered set of comments sharing a
    /// `thread_id`. The root (if present) is placed first; the remainder are
    /// sorted by `created_unix_ms` and then `id` for a stable order.
    #[must_use]
    pub fn from_comments(mut comments: Vec<Comment>) -> Self {
        comments.sort_by(|a, b| {
            // Root always leads; otherwise order by creation time then id.
            b.is_root()
                .cmp(&a.is_root())
                .then(a.created_unix_ms.cmp(&b.created_unix_ms))
                .then_with(|| a.id.cmp(&b.id))
        });
        Self { comments }
    }

    /// The thread's root comment, if the set contained one.
    #[must_use]
    pub fn root(&self) -> Option<&Comment> {
        self.comments.first().filter(|c| c.is_root())
    }

    /// The number of comments in the thread.
    #[must_use]
    pub fn len(&self) -> usize {
        self.comments.len()
    }

    /// Returns `true` if the thread has no comments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.comments.is_empty()
    }
}

// -----------------------------------------------------------------------------
// Design-review states (additive; the wire format stays frozen)
// -----------------------------------------------------------------------------
//
// A lightweight design-review workflow rides the SHIPPED comment core: a thread
// carries a review verdict (open -> approved / changes-requested) that a reviewer
// sets. The verdict is NOT a new wire field. It is DERIVED from ordinary
// review-action comments (see [`Comment::review_action`]): a review action is a
// reply in the thread whose body is the marker `"[review:<tag>]"`, so it persists
// in the schema-V2 `Document.comments` field and travels on the existing comment
// frame (`v1::SyncMessage`, first byte `0x1A`). No proto message, field number, or
// relay code changes; the frozen wire (ADR 0080) is untouched.

/// The design-review verdict recorded on a [`CommentThread`].
///
/// A thread starts [`Open`](ReviewState::Open) (under discussion) and a reviewer
/// moves it to [`Approved`](ReviewState::Approved) or
/// [`ChangesRequested`](ReviewState::ChangesRequested). The verdict is derived from
/// the thread's review-action comments (see [`CommentThread::review_state`]), so it
/// syncs with the comments themselves and needs no new wire field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewState {
    /// Under discussion; no verdict recorded yet. The default.
    #[default]
    Open,
    /// A reviewer approved the change the thread annotates.
    Approved,
    /// A reviewer asked for changes before the thread can be approved.
    ChangesRequested,
}

impl ReviewState {
    /// The state reached by applying `action` to this state.
    ///
    /// A verdict is last-write-wins: an [`Approve`](ReviewAction::Approve) yields
    /// [`Approved`](ReviewState::Approved), a
    /// [`RequestChanges`](ReviewAction::RequestChanges) yields
    /// [`ChangesRequested`](ReviewState::ChangesRequested), and a
    /// [`Reopen`](ReviewAction::Reopen) returns to [`Open`](ReviewState::Open),
    /// whatever the prior state. Re-applying the same verdict is therefore
    /// idempotent (approving an approved thread leaves it approved).
    #[must_use]
    pub fn after(self, action: ReviewAction) -> ReviewState {
        // Matched as a full transition table (keyed on both the current state and
        // the action) so the shape stays honest and extensible even though every
        // current verdict is last-write-wins.
        match (self, action) {
            (_, ReviewAction::Approve) => ReviewState::Approved,
            (_, ReviewAction::RequestChanges) => ReviewState::ChangesRequested,
            (_, ReviewAction::Reopen) => ReviewState::Open,
        }
    }

    /// A short, stable, human-readable label for the verdict (panel and status text).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ReviewState::Open => "Open",
            ReviewState::Approved => "Approved",
            ReviewState::ChangesRequested => "Changes requested",
        }
    }

    /// Whether the thread has been approved.
    #[must_use]
    pub fn is_approved(self) -> bool {
        matches!(self, ReviewState::Approved)
    }
}

/// A reviewer action that drives a thread's [`ReviewState`].
///
/// Each action is recorded as a review-action comment (see
/// [`Comment::review_action`]) and decoded back with
/// [`Comment::as_review_action`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewAction {
    /// Approve the change the thread annotates.
    Approve,
    /// Ask for changes before the thread can be approved.
    RequestChanges,
    /// Return a decided thread to [`Open`](ReviewState::Open) for more discussion.
    Reopen,
}

impl ReviewAction {
    /// The stable tag naming this action (the payload of the body marker).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            ReviewAction::Approve => "approve",
            ReviewAction::RequestChanges => "request-changes",
            ReviewAction::Reopen => "reopen",
        }
    }

    /// The action for a [`tag`](ReviewAction::tag), or `None` if it names none.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<ReviewAction> {
        match tag {
            "approve" => Some(ReviewAction::Approve),
            "request-changes" => Some(ReviewAction::RequestChanges),
            "reopen" => Some(ReviewAction::Reopen),
            _ => None,
        }
    }
}

/// Encodes a review action as a comment body: the marker `"[review:<tag>]"`.
///
/// The whole body is the marker (never a fragment of human prose), so a comment is
/// a review action exactly when its body decodes here; a human who typed the marker
/// verbatim would harmlessly register the same verdict.
fn encode_review_action(action: ReviewAction) -> String {
    format!("[review:{}]", action.tag())
}

/// Decodes a review action from a comment body, or `None` if the body is not a
/// review-action marker (i.e. it is human prose). The inverse of
/// [`encode_review_action`].
fn decode_review_action(body: &str) -> Option<ReviewAction> {
    let tag = body.strip_prefix("[review:")?.strip_suffix(']')?;
    ReviewAction::from_tag(tag)
}

impl Comment {
    /// Creates a review-action comment: a reply in `parent`'s thread whose body
    /// encodes `action` (the design-review workflow).
    ///
    /// The result is an ordinary [`Comment`]. It inherits `parent`'s `thread_id` and
    /// `anchor_ref` and sets `in_reply_to` to `parent.id`, so it persists in the
    /// schema-V2 `comments` field and travels on the live comment frame with no proto
    /// or relay change. [`CommentThread::review_state`] folds these into a thread's
    /// [`ReviewState`]; [`as_review_action`](Comment::as_review_action) decodes one.
    #[must_use]
    pub fn review_action(
        parent: &Comment,
        id: impl Into<String>,
        author: impl Into<String>,
        action: ReviewAction,
        created_unix_ms: i64,
    ) -> Self {
        Comment::reply_to(
            parent,
            id,
            author,
            encode_review_action(action),
            created_unix_ms,
        )
    }

    /// The review action this comment carries, or `None` if it is human prose.
    #[must_use]
    pub fn as_review_action(&self) -> Option<ReviewAction> {
        decode_review_action(&self.body)
    }

    /// Whether this comment encodes a review action rather than human prose.
    ///
    /// A comment display that wants to show only discussion can filter these out
    /// (see [`CommentThread::discussion`]); their verdict surfaces through
    /// [`CommentThread::review_state`] instead.
    #[must_use]
    pub fn is_review_action(&self) -> bool {
        self.as_review_action().is_some()
    }
}

impl CommentThread {
    /// The thread's derived [`ReviewState`] (the design-review verdict).
    ///
    /// Folds the thread's review-action comments over [`ReviewState::after`] in
    /// creation order, starting from [`ReviewState::Open`]. Because a verdict is
    /// last-write-wins the result is exactly the newest review action's verdict
    /// (idempotent under repeats), and a thread with no review action is
    /// [`Open`](ReviewState::Open). Human comments are ignored. The fold walks the
    /// stored order, which [`CommentThread::from_comments`] normalizes to root-first
    /// then by creation time, so a thread built through that constructor folds
    /// chronologically regardless of the input order.
    #[must_use]
    pub fn review_state(&self) -> ReviewState {
        self.comments
            .iter()
            .filter_map(Comment::as_review_action)
            .fold(ReviewState::Open, ReviewState::after)
    }

    /// The most recent review-action comment in the thread, if any: it names who
    /// recorded the current verdict and when. `None` for a thread with no verdict.
    #[must_use]
    pub fn last_review_action(&self) -> Option<&Comment> {
        self.comments.iter().rev().find(|c| c.is_review_action())
    }

    /// The thread's human comments, excluding the machine-authored review actions,
    /// so a display can show the discussion apart from the recorded verdict.
    pub fn discussion(&self) -> impl Iterator<Item = &Comment> {
        self.comments.iter().filter(|c| !c.is_review_action())
    }
}

#[cfg(test)]
mod review_tests {
    use super::{Comment, CommentThread, ReviewAction, ReviewState};

    /// A root comment and one review action replying to it, folded into a thread.
    fn thread_with(actions: &[(ReviewAction, i64)]) -> CommentThread {
        let root = Comment::root("root", "TOP", "alice", "please review", 1);
        let mut comments = vec![root.clone()];
        for (i, (action, ts)) in actions.iter().enumerate() {
            comments.push(Comment::review_action(
                &root,
                format!("rv{i}"),
                "bob",
                *action,
                *ts,
            ));
        }
        CommentThread::from_comments(comments)
    }

    #[test]
    fn every_transition_is_last_write_wins() {
        // `after` ignores the prior state: each action fully determines the verdict.
        for start in [
            ReviewState::Open,
            ReviewState::Approved,
            ReviewState::ChangesRequested,
        ] {
            assert_eq!(start.after(ReviewAction::Approve), ReviewState::Approved);
            assert_eq!(
                start.after(ReviewAction::RequestChanges),
                ReviewState::ChangesRequested
            );
            assert_eq!(start.after(ReviewAction::Reopen), ReviewState::Open);
        }
    }

    #[test]
    fn a_fresh_thread_is_open() {
        // No comments, and a thread of only human prose, are both Open.
        assert_eq!(CommentThread::default().review_state(), ReviewState::Open);
        let human = CommentThread::from_comments(vec![
            Comment::root("r", "TOP", "alice", "note", 1),
            Comment::reply_to(
                &Comment::root("r", "TOP", "alice", "note", 1),
                "r2",
                "bob",
                "looks fine",
                2,
            ),
        ]);
        assert_eq!(human.review_state(), ReviewState::Open);
    }

    #[test]
    fn open_to_approved_and_changes_requested() {
        assert_eq!(
            thread_with(&[(ReviewAction::Approve, 2)]).review_state(),
            ReviewState::Approved
        );
        assert_eq!(
            thread_with(&[(ReviewAction::RequestChanges, 2)]).review_state(),
            ReviewState::ChangesRequested
        );
    }

    #[test]
    fn approving_twice_is_idempotent() {
        let once = thread_with(&[(ReviewAction::Approve, 2)]).review_state();
        let twice =
            thread_with(&[(ReviewAction::Approve, 2), (ReviewAction::Approve, 3)]).review_state();
        assert_eq!(once, ReviewState::Approved);
        assert_eq!(twice, ReviewState::Approved);
    }

    #[test]
    fn the_newest_verdict_wins() {
        // Approve, then request changes, then reopen: the latest action decides.
        assert_eq!(
            thread_with(&[
                (ReviewAction::Approve, 2),
                (ReviewAction::RequestChanges, 3),
            ])
            .review_state(),
            ReviewState::ChangesRequested
        );
        assert_eq!(
            thread_with(&[
                (ReviewAction::Approve, 2),
                (ReviewAction::RequestChanges, 3),
                (ReviewAction::Reopen, 4),
            ])
            .review_state(),
            ReviewState::Open
        );
    }

    #[test]
    fn from_comments_normalizes_order_before_the_fold() {
        // Feed the actions out of order; from_comments sorts by timestamp, so the
        // fold still lands on the newest verdict (request-changes at ts 4).
        let root = Comment::root("root", "TOP", "alice", "review", 1);
        let out_of_order = vec![
            Comment::review_action(&root, "b", "bob", ReviewAction::RequestChanges, 4),
            root.clone(),
            Comment::review_action(&root, "a", "bob", ReviewAction::Approve, 2),
        ];
        let thread = CommentThread::from_comments(out_of_order);
        assert_eq!(thread.review_state(), ReviewState::ChangesRequested);
    }

    #[test]
    fn review_action_round_trips_and_rides_the_thread() {
        let root = Comment::root("root", "TOP/shape-3", "alice", "check this", 1);
        let action = Comment::review_action(&root, "rv0", "bob", ReviewAction::Approve, 5);
        // It decodes back to the same action, and reads as a review action.
        assert_eq!(action.as_review_action(), Some(ReviewAction::Approve));
        assert!(action.is_review_action());
        // It is an ordinary reply: same thread and anchor, not a root.
        assert_eq!(action.thread_id, root.thread_id);
        assert_eq!(action.anchor_ref, "TOP/shape-3");
        assert!(!action.is_root());
        // A human comment is not a review action.
        assert!(!root.is_review_action());
        assert_eq!(root.as_review_action(), None);
    }

    #[test]
    fn last_review_action_and_discussion_split_verdict_from_prose() {
        let root = Comment::root("root", "TOP", "alice", "please review", 1);
        let reply = Comment::reply_to(&root, "human", "carol", "one nit", 2);
        let approve = Comment::review_action(&root, "rv0", "bob", ReviewAction::Approve, 3);
        let thread =
            CommentThread::from_comments(vec![root.clone(), reply.clone(), approve.clone()]);
        // The latest review action is the approval by bob.
        let last = thread.last_review_action().expect("a verdict was recorded");
        assert_eq!(last.author, "bob");
        assert_eq!(last.as_review_action(), Some(ReviewAction::Approve));
        // Discussion is the two human comments only, in thread order.
        let discussion: Vec<&str> = thread.discussion().map(|c| c.id.as_str()).collect();
        assert_eq!(discussion, ["root", "human"]);
    }

    #[test]
    fn labels_and_is_approved_read_cleanly() {
        assert_eq!(ReviewState::Open.label(), "Open");
        assert_eq!(ReviewState::Approved.label(), "Approved");
        assert_eq!(ReviewState::ChangesRequested.label(), "Changes requested");
        assert!(ReviewState::Approved.is_approved());
        assert!(!ReviewState::Open.is_approved());
        assert!(!ReviewState::ChangesRequested.is_approved());
    }

    #[test]
    fn action_tags_round_trip() {
        for action in [
            ReviewAction::Approve,
            ReviewAction::RequestChanges,
            ReviewAction::Reopen,
        ] {
            assert_eq!(ReviewAction::from_tag(action.tag()), Some(action));
        }
        assert_eq!(ReviewAction::from_tag("nope"), None);
    }
}

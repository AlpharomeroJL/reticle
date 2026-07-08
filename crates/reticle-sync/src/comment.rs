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

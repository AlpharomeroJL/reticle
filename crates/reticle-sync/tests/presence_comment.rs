//! Round-trip tests for presence and threaded comments, plus the awareness map.

use reticle_geometry::{Point, Rect};
use reticle_sync::{Awareness, Comment, CommentThread, Presence};

#[test]
fn presence_proto_round_trip() {
    let mut p = Presence::new("alice");
    p.display_name = "Alice A.".to_owned();
    p.color_rgba = 0xFF_00_88_FF;
    p.cursor = Point::new(-123, 456);
    p.selection = vec!["top/shape-1".to_owned(), "sub".to_owned()];
    p.viewport = Rect::new(Point::new(0, 0), Point::new(1920, 1080));

    let decoded = Presence::from_proto(&p.to_proto());
    assert_eq!(p, decoded, "presence did not survive proto round-trip");
}

#[test]
fn presence_defaults_when_fields_absent() {
    // A proto with no cursor and no viewport should decode to origin / empty.
    let proto = reticle_proto::v1::Presence {
        actor: "bob".to_owned(),
        display_name: String::new(),
        color_rgba: 0,
        cursor: None,
        selection: Vec::new(),
        viewport: None,
    };
    let p = Presence::from_proto(&proto);
    assert_eq!(p.actor, "bob");
    assert_eq!(p.cursor, Point::ORIGIN);
    assert_eq!(p.viewport, Rect::default());
}

#[test]
fn presence_wraps_in_sync_message() {
    let p = Presence::new("alice");
    let msg = p.to_message();
    match msg.payload {
        Some(reticle_proto::v1::sync_message::Payload::Presence(inner)) => {
            assert_eq!(inner.actor, "alice");
        }
        _ => panic!("expected a presence payload"),
    }
}

#[test]
fn awareness_tracks_latest_per_actor() {
    let mut awareness = Awareness::new();
    assert!(awareness.is_empty());

    let mut a1 = Presence::new("alice");
    a1.cursor = Point::new(1, 1);
    assert!(awareness.set(a1).is_none());

    // A newer presence for the same actor replaces the old one.
    let mut a2 = Presence::new("alice");
    a2.cursor = Point::new(2, 2);
    let previous = awareness.set(a2).expect("previous alice presence");
    assert_eq!(previous.cursor, Point::new(1, 1));

    awareness.set(Presence::new("bob"));
    assert_eq!(awareness.len(), 2);
    assert_eq!(awareness.get("alice").unwrap().cursor, Point::new(2, 2));

    let removed = awareness.remove("alice").expect("alice removed");
    assert_eq!(removed.cursor, Point::new(2, 2));
    assert_eq!(awareness.len(), 1);
    assert!(awareness.get("alice").is_none());
}

#[test]
fn comment_proto_round_trip() {
    let root = Comment::root("c1", "top/shape-7", "alice", "Should this be wider?", 1000);
    assert!(root.is_root());
    assert_eq!(root.thread_id, "c1");

    let decoded = Comment::from_proto(&root.to_proto());
    assert_eq!(root, decoded, "comment did not survive proto round-trip");
}

#[test]
fn comment_reply_inherits_thread_and_anchor() {
    let root = Comment::root("c1", "cellA", "alice", "Please review", 1000);
    let reply = Comment::reply_to(&root, "c2", "bob", "Looks good", 2000);

    assert!(!reply.is_root());
    assert_eq!(reply.thread_id, root.thread_id);
    assert_eq!(reply.anchor_ref, root.anchor_ref);
    assert_eq!(reply.in_reply_to, "c1");

    // Round-trip the reply too.
    assert_eq!(reply, Comment::from_proto(&reply.to_proto()));
}

#[test]
fn comment_thread_orders_root_first_then_by_time() {
    let root = Comment::root("c1", "cellA", "alice", "Q", 1000);
    let reply_late = Comment::reply_to(&root, "c3", "carol", "later", 3000);
    let reply_early = Comment::reply_to(&root, "c2", "bob", "earlier", 2000);

    // Feed them out of order; the thread must sort root-first then by timestamp.
    let thread = CommentThread::from_comments(vec![reply_late, root.clone(), reply_early]);
    assert_eq!(thread.len(), 3);
    assert_eq!(thread.root().map(|c| c.id.as_str()), Some("c1"));
    let ids: Vec<&str> = thread.comments.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["c1", "c2", "c3"]);
}

#[test]
fn comment_wraps_in_sync_message() {
    let c = Comment::root("c1", "cellA", "alice", "hi", 42);
    match c.to_message().payload {
        Some(reticle_proto::v1::sync_message::Payload::Comment(inner)) => {
            assert_eq!(inner.id, "c1");
            assert_eq!(inner.created_unix_ms, 42);
        }
        _ => panic!("expected a comment payload"),
    }
}

//! Share-this-session room links.
//!
//! The collaboration relay (`reticle-server`) joins every peer of one document
//! into a named room over `GET /ws/{room}`; sharing a session therefore means
//! handing a collaborator the WebSocket URL that names the same relay host and
//! room. This module owns the pure link logic, sanitizing a free-form room
//! name into a URL path segment and composing the join URL from whatever the
//! user typed as the relay host, so the app module only draws the fields and a
//! copy button. Portable: it builds for the web target too.
//!
//! # Read-only viewer links (ADR 0038)
//!
//! Sharing a session read-only hands a viewer a **page** URL rather than a raw
//! WebSocket URL: the deployed web bundle opens it, reads the query, and joins the
//! named room read-only. The web entry already selects a start view from `?view=`
//! (ADR 0026); [`viewer_link`] extends that with `?view=viewer`, plus `room=` and
//! `relay=` so the viewer page knows which room on which relay to join. A viewer
//! joins the relay with the read-only flag (`?mode=view`, see [`viewer_ws_link`]
//! and `reticle-server`'s `JoinMode`) and never publishes, so the sharer's session
//! stays authoritative. [`parse_viewer_query`] is the inverse, used by the web
//! entry to recover the room and relay from the page URL.

/// The relay address `reticle-server` binds when `RETICLE_SERVER_ADDR` is
/// unset, used as the share panel's starting value.
pub const DEFAULT_SERVER: &str = "127.0.0.1:3030";

/// The room name used when sanitizing leaves nothing (an all-junk name).
const FALLBACK_ROOM: &str = "layout";

/// Sanitizes a free-form name into a relay room id, safe as a URL path
/// segment.
///
/// Lowercases ASCII, keeps `a-z`, `0-9`, `-`, and `_`, and collapses every run
/// of anything else into a single `-`, trimming stray dashes from the ends. A
/// name with nothing usable in it becomes `FALLBACK_ROOM` (`"layout"`) so the
/// composed link always has a room. Idempotent: sanitizing a sanitized id is a
/// no-op.
#[must_use]
pub fn room_id(name: &str) -> String {
    let mut id = String::with_capacity(name.len());
    let mut pending_dash = false;
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            if pending_dash && !id.is_empty() {
                id.push('-');
            }
            pending_dash = false;
            id.push(c);
        } else {
            pending_dash = true;
        }
    }
    let id = id.trim_matches('-');
    if id.is_empty() {
        FALLBACK_ROOM.to_owned()
    } else {
        id.to_owned()
    }
}

/// Composes the WebSocket join link for `room` on `server`.
///
/// The server spec is whatever the user typed: a bare `host:port` gets the
/// `ws://` scheme, `http`/`https` map to their WebSocket counterparts
/// (`ws`/`wss`), and an explicit `ws://` or `wss://` is kept. Trailing slashes
/// are dropped before the `/ws/{room}` route (the relay's only route) is
/// appended; the room goes through [`room_id`]. An empty server spec falls
/// back to [`DEFAULT_SERVER`].
#[must_use]
pub fn room_link(server: &str, room: &str) -> String {
    let trimmed = server.trim();
    let spec = if trimmed.is_empty() {
        DEFAULT_SERVER
    } else {
        trimmed
    };
    let with_scheme = if spec.starts_with("ws://") || spec.starts_with("wss://") {
        spec.to_owned()
    } else if let Some(rest) = spec.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = spec.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{spec}")
    };
    let base = with_scheme.trim_end_matches('/');
    format!("{base}/ws/{}", room_id(room))
}

/// The `?view=` query value that opens the web bundle as a read-only viewer.
pub const VIEWER_VIEW: &str = "viewer";

/// Composes the read-only **viewer page** link for `room` on `relay`, hosted at
/// the page origin `page`.
///
/// The result is a URL the deployed web bundle understands: `page` is the site the
/// bundle is served from (for example `https://reticle.example`), and the query
/// carries `view=viewer` (so the entry opens read-only, ADR 0026/0038), the
/// sanitized `room`, and the `relay` host the viewer connects to. An empty `page`
/// yields a relative `?...` query so the link resolves against wherever the bundle
/// is already loaded. An empty `relay` falls back to [`DEFAULT_SERVER`].
///
/// This is intentionally a *page* URL, not the raw WebSocket URL: a human opens it
/// in a browser and the bundle does the read-only join. Use [`viewer_ws_link`] for
/// the WebSocket URL the viewer page then dials.
#[must_use]
pub fn viewer_link(page: &str, relay: &str, room: &str) -> String {
    let room = room_id(room);
    let relay_spec = {
        let trimmed = relay.trim();
        if trimmed.is_empty() {
            DEFAULT_SERVER
        } else {
            trimmed
        }
    };
    let query = format!(
        "view={VIEWER_VIEW}&room={room}&relay={}",
        encode_query_component(relay_spec)
    );
    let base = page.trim().trim_end_matches('/');
    if base.is_empty() {
        format!("?{query}")
    } else {
        format!("{base}/?{query}")
    }
}

/// Composes the **read-only WebSocket** link a viewer page dials for `room` on
/// `relay`: [`room_link`] with the read-only `?mode=view` flag appended.
///
/// The relay enforces read-only on this flag (`reticle-server`'s `JoinMode`), so a
/// viewer that dials this URL receives the sharer's frames but cannot publish. The
/// app-side viewer also never sends edits; the flag is the server-side backstop.
#[must_use]
pub fn viewer_ws_link(relay: &str, room: &str) -> String {
    let base = room_link(relay, room);
    // `room_link` never emits a query, so the separator is always `?`.
    format!("{base}?mode=view")
}

/// The room and relay recovered from a viewer page's query string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ViewerTarget {
    /// The sanitized relay room the viewer joins read-only.
    pub room: String,
    /// The relay host the viewer connects to (as the sharer specified it).
    pub relay: String,
}

/// Parses a viewer page's query string (the part after `?`) into a
/// [`ViewerTarget`], or `None` if it is not a viewer link.
///
/// Returns `Some` only when `view=viewer` is present and a non-empty `room` is
/// given; a missing or empty `relay` falls back to [`DEFAULT_SERVER`] so a link
/// that omits it still resolves. This is the inverse of [`viewer_link`] and is
/// meant to be fed the browser's `location.search` (a leading `?` is tolerated).
#[must_use]
pub fn parse_viewer_query(query: &str) -> Option<ViewerTarget> {
    let query = query.trim_start_matches('?');
    let mut view = None;
    let mut room = None;
    let mut relay = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode_query_component(value);
        match key {
            "view" => view = Some(value),
            "room" => room = Some(value),
            "relay" => relay = Some(value),
            _ => {}
        }
    }
    if view.as_deref() != Some(VIEWER_VIEW) {
        return None;
    }
    // A `room` key must be present; its value is sanitized through `room_id`, which
    // always yields a valid segment (an empty or all-junk value becomes the
    // `room_id` fallback). A viewer link with no `room` key at all is not a target.
    let room = room_id(&room?);
    let relay = relay
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVER.to_owned());
    Some(ViewerTarget { room, relay })
}

/// Percent-encodes the characters that would break a query component: `&`, `=`,
/// `%`, `#`, `?`, and whitespace. A relay spec is host/port/scheme text, so this
/// small set is sufficient; it keeps the link readable rather than over-encoding.
fn encode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            ' ' => out.push_str("%20"),
            _ => out.push(c),
        }
    }
    out
}

/// Reverses [`encode_query_component`], decoding `%XX` byte escapes. Invalid or
/// truncated escapes are left verbatim so a hand-typed link never panics.
fn decode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex_val(*b)),
                bytes.get(i + 2).and_then(|b| hex_val(*b)),
            )
        {
            out.push(char::from(h * 16 + l));
            i += 3;
            continue;
        }
        out.push(char::from(bytes[i]));
        i += 1;
    }
    out
}

/// The value of a single ASCII hex digit, or `None`.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_and_demo_room_compose() {
        assert_eq!(room_link("", "CHIP_TOP"), "ws://127.0.0.1:3030/ws/chip_top");
        assert_eq!(
            room_link("   ", "CHIP_TOP"),
            "ws://127.0.0.1:3030/ws/chip_top"
        );
    }

    #[test]
    fn bare_host_gets_the_ws_scheme() {
        assert_eq!(
            room_link("relay.lab:9000", "top"),
            "ws://relay.lab:9000/ws/top"
        );
    }

    #[test]
    fn http_schemes_map_to_websocket_schemes() {
        assert_eq!(
            room_link("http://relay.lab", "top"),
            "ws://relay.lab/ws/top"
        );
        assert_eq!(
            room_link("https://relay.lab", "top"),
            "wss://relay.lab/ws/top"
        );
        // Explicit WebSocket schemes are kept as-is.
        assert_eq!(room_link("ws://r:1", "top"), "ws://r:1/ws/top");
        assert_eq!(room_link("wss://r:1", "top"), "wss://r:1/ws/top");
    }

    #[test]
    fn trailing_slashes_are_dropped() {
        assert_eq!(room_link("ws://relay.lab/", "top"), "ws://relay.lab/ws/top");
        assert_eq!(room_link("relay.lab//", "top"), "ws://relay.lab/ws/top");
    }

    #[test]
    fn room_ids_sanitize_to_path_segments() {
        assert_eq!(room_id("My Fancy Layout!"), "my-fancy-layout");
        assert_eq!(room_id("CHIP_TOP"), "chip_top");
        assert_eq!(room_id("a  b\tc"), "a-b-c");
        assert_eq!(room_id("sram-64x32"), "sram-64x32");
        assert_eq!(room_id("  spaced  "), "spaced");
        assert_eq!(room_id("!!!"), "layout", "all-junk falls back");
        assert_eq!(room_id(""), "layout");
        // Runs of junk collapse to one dash, and edge dashes are trimmed.
        assert_eq!(room_id("--a//b--"), "a-b");
    }

    #[test]
    fn room_id_is_idempotent() {
        for name in ["My Fancy Layout!", "CHIP_TOP", "!!!", "a b", "x"] {
            let once = room_id(name);
            assert_eq!(room_id(&once), once, "sanitizing {name:?} twice");
        }
    }

    #[test]
    fn viewer_link_carries_view_room_and_relay() {
        // A colon in the relay host is legal in a query value and left as-is; the
        // room is sanitized and `view=viewer` marks the read-only page.
        let link = viewer_link("https://reticle.example", "relay.lab:9000", "CHIP_TOP");
        assert_eq!(
            link,
            "https://reticle.example/?view=viewer&room=chip_top&relay=relay.lab:9000"
        );
    }

    #[test]
    fn viewer_link_with_empty_page_is_a_relative_query() {
        // No page origin: a relative query so it resolves against the loaded bundle.
        let link = viewer_link("", "", "top");
        assert_eq!(link, "?view=viewer&room=top&relay=127.0.0.1:3030");
    }

    #[test]
    fn viewer_link_encodes_reserved_query_characters() {
        // A relay spec carrying reserved query characters is percent-encoded so the
        // link stays parseable, and decodes back exactly.
        let link = viewer_link("", "ws://h?a=b&c=d", "top");
        assert!(
            link.contains("relay=ws://h%3Fa%3Db%26c%3Dd"),
            "reserved characters must be encoded: {link}"
        );
        let query = link.trim_start_matches('?');
        let target = parse_viewer_query(query).expect("viewer query");
        assert_eq!(target.relay, "ws://h?a=b&c=d", "relay decodes back exactly");
    }

    #[test]
    fn viewer_ws_link_appends_the_read_only_flag() {
        assert_eq!(
            viewer_ws_link("relay.lab:9000", "top"),
            "ws://relay.lab:9000/ws/top?mode=view"
        );
        // Scheme mapping from room_link is preserved before the flag.
        assert_eq!(
            viewer_ws_link("https://relay.lab", "CHIP_TOP"),
            "wss://relay.lab/ws/chip_top?mode=view"
        );
    }

    #[test]
    fn parse_viewer_query_round_trips_viewer_link() {
        // The query produced by viewer_link parses back to the same room and relay.
        let link = viewer_link("https://reticle.example", "relay.lab:9000", "CHIP_TOP");
        let query = link.split_once('?').expect("link has a query").1;
        let target = parse_viewer_query(query).expect("a viewer query");
        assert_eq!(target.room, "chip_top");
        assert_eq!(target.relay, "relay.lab:9000");
    }

    #[test]
    fn parse_viewer_query_tolerates_leading_question_mark_and_defaults_relay() {
        // A leading '?' (as in location.search) is fine; a missing relay defaults.
        let target = parse_viewer_query("?view=viewer&room=top").expect("viewer query");
        assert_eq!(target.room, "top");
        assert_eq!(target.relay, DEFAULT_SERVER);
    }

    #[test]
    fn parse_viewer_query_rejects_non_viewer_links() {
        // Not a viewer view: the editor/replay start views are not viewer targets.
        assert!(parse_viewer_query("view=editor&room=top").is_none());
        assert!(parse_viewer_query("view=replay").is_none());
        // Viewer view but no `room` key at all: not a target.
        assert!(parse_viewer_query("view=viewer").is_none());
        assert!(parse_viewer_query("view=viewer&relay=r:1").is_none());
        // No view at all.
        assert!(parse_viewer_query("room=top&relay=r:1").is_none());
    }

    #[test]
    fn parse_viewer_query_maps_empty_or_junk_room_to_the_fallback() {
        // A present `room` key with an empty or all-junk value sanitizes to the same
        // fallback `room_id` uses, so the viewer still lands in a valid room.
        for q in ["view=viewer&room=", "view=viewer&room=!!!"] {
            let target = parse_viewer_query(q).expect("viewer query");
            assert_eq!(target.room, "layout", "for {q:?}");
            assert_eq!(target.room, room_id(""));
        }
    }
}

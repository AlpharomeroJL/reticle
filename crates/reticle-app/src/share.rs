//! Share-this-session room links.
//!
//! The collaboration relay (`reticle-server`) joins every peer of one document
//! into a named room over `GET /ws/{room}`; sharing a session therefore means
//! handing a collaborator the WebSocket URL that names the same relay host and
//! room. This module owns the pure link logic, sanitizing a free-form room
//! name into a URL path segment and composing the join URL from whatever the
//! user typed as the relay host, so the app module only draws the fields and a
//! copy button. Portable: it builds for the web target too.

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
}

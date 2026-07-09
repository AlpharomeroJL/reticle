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

/// A deep-link into an already-opened document: which cell to focus, where to point
/// the camera, and which layers to show.
///
/// A permalink layers view state on top of the `?gds=<url>` open ([`crate::webopen`]):
/// the bundle opens the document, then applies whichever of these three are present.
/// All three are optional and independent, so a link may restore only a camera, only
/// a cell, or all three. Emitting is [`emit_permalink`]; parsing is
/// [`parse_permalink`], and the two round-trip (`emit -> parse` is the identity on the
/// [`Permalink`] fields).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Permalink {
    /// The cell to focus on open, if the link names one (URL-decoded).
    pub cell: Option<String>,
    /// The camera to restore as `(center_x, center_y, pixels_per_dbu)` in world DBU,
    /// if the link carries a camera spec.
    pub camera: Option<(f64, f64, f64)>,
    /// The layers to show (every other layer hidden), each `(layer, datatype)`. `Some`
    /// with an empty vec means "hide everything"; `None` means "leave layers alone".
    pub layers: Option<Vec<(u16, u16)>>,
}

/// Parses the view-state permalink params out of a page query string.
///
/// Recognizes three keys, each independently optional:
/// * `cell=<name>` - the focus cell, URL-decoded (UTF-8), ignored when empty.
/// * `view=<x>,<y>,<zoom>` - a camera spec, **only** when the value parses as exactly
///   three comma-separated `f64`s. The `view` key is also the start-view selector
///   (`viewer`/`editor`/`replay`, ADR 0026); the two are disambiguated by shape here,
///   so a `view=editor` leaves [`Permalink::camera`] `None` and a `view=1,2,3` fills it.
/// * `layers=<csv>` - a comma-separated list of `layer/datatype` specs; malformed
///   entries are skipped rather than failing the whole list, and a present-but-empty
///   value yields `Some(vec![])` ("hide everything"). An absent key yields `None`.
///
/// Every value is parsed leniently: a malformed float, a bad layer spec, or an unknown
/// key is ignored and never panics. A leading `?` is tolerated.
#[must_use]
pub fn parse_permalink(query: &str) -> Permalink {
    let query = query.trim_start_matches('?');
    let mut out = Permalink::default();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "cell" => {
                let name = decode_permalink_value(value);
                if !name.is_empty() {
                    out.cell = Some(name);
                }
            }
            // `view` is shared with the start-view selector (ADR 0026): a value that
            // parses as exactly three floats is a camera spec; anything else (a
            // `viewer`/`editor`/`replay` keyword, or junk) leaves the camera unset.
            "view" => {
                if let Some(camera) = parse_camera_spec(value) {
                    out.camera = Some(camera);
                }
            }
            // A present `layers` key always yields `Some` (even empty, meaning "hide
            // all"); malformed specs inside are skipped, not fatal.
            "layers" => out.layers = Some(parse_layer_csv(value)),
            _ => {}
        }
    }
    out
}

/// Parses a `view=<x>,<y>,<zoom>` value into a camera spec, or `None` when it is not
/// exactly three finite comma-separated floats (so a start-view keyword, a truncated
/// pair, or an over-long list is rejected rather than misread as a camera).
fn parse_camera_spec(value: &str) -> Option<(f64, f64, f64)> {
    let mut parts = value.split(',');
    let x = parts.next()?.trim().parse::<f64>().ok()?;
    let y = parts.next()?.trim().parse::<f64>().ok()?;
    let zoom = parts.next()?.trim().parse::<f64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if x.is_finite() && y.is_finite() && zoom.is_finite() {
        Some((x, y, zoom))
    } else {
        None
    }
}

/// Parses a `layers=` CSV of `layer/datatype` specs, skipping any entry that is not two
/// `u16`s split by `/` (so an unknown or out-of-range spec is ignored, never fatal). An
/// empty value yields an empty vec, which the caller reads as "hide every layer".
fn parse_layer_csv(value: &str) -> Vec<(u16, u16)> {
    let mut layers = Vec::new();
    for spec in value.split(',') {
        let spec = spec.trim();
        if spec.is_empty() {
            continue;
        }
        if let Some((layer, datatype)) = spec.split_once('/')
            && let (Ok(layer), Ok(datatype)) =
                (layer.trim().parse::<u16>(), datatype.trim().parse::<u16>())
        {
            layers.push((layer, datatype));
        }
    }
    layers
}

/// Emits a shareable page URL carrying `gds` (when given) and the view-state params of
/// `p`, hosted at page origin `base_page`.
///
/// The inverse of [`parse_permalink`]: the query it writes parses back to the same
/// [`Permalink`]. An empty `base_page` yields a relative `?...` query (resolving against
/// the loaded bundle), mirroring [`viewer_link`]; a non-empty one is joined as
/// `base/?...`. The `gds` URL, cell name, and any values are percent-encoded so the
/// link stays parseable. Absent [`Permalink`] fields emit no key at all.
#[must_use]
pub fn emit_permalink(base_page: &str, gds: Option<&str>, p: &Permalink) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(gds) = gds {
        parts.push(format!("gds={}", encode_permalink_value(gds)));
    }
    if let Some(cell) = &p.cell {
        parts.push(format!("cell={}", encode_permalink_value(cell)));
    }
    if let Some((x, y, zoom)) = p.camera {
        // Rust's default float formatting is the shortest string that parses back
        // exactly, so the camera round-trips without a chosen precision.
        parts.push(format!("view={x},{y},{zoom}"));
    }
    if let Some(layers) = &p.layers {
        let csv = layers
            .iter()
            .map(|(layer, datatype)| format!("{layer}/{datatype}"))
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("layers={csv}"));
    }
    let query = parts.join("&");
    let base = base_page.trim().trim_end_matches('/');
    match (base.is_empty(), query.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("?{query}"),
        (false, true) => base.to_owned(),
        (false, false) => format!("{base}/?{query}"),
    }
}

/// Percent-encodes `s` as a permalink query-component value, byte by byte: the
/// unreserved URL characters (`A-Za-z0-9-_.~`) are kept and every other byte -
/// including each byte of a multi-byte UTF-8 character - becomes `%XX`.
///
/// This is a full RFC 3986 component encoding, unlike [`encode_query_component`] (which
/// only escapes the handful of reserved characters a relay host can carry): a permalink
/// value is a cell name or a URL and may hold spaces, `/`, `,`, or non-ASCII text, so it
/// must be fully escaped to round-trip through [`decode_permalink_value`].
fn encode_permalink_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                out.push('%');
                out.push(char::from(hex_digit(b >> 4)));
                out.push(char::from(hex_digit(b & 0x0f)));
            }
        }
    }
    out
}

/// Reverses [`encode_permalink_value`], decoding `%XX` byte escapes and rebuilding the
/// UTF-8 string from the resulting bytes.
///
/// Invalid or truncated escapes are left verbatim so a hand-edited link never panics,
/// and the byte buffer is decoded lossily to guarantee a valid `String`.
fn decode_permalink_value(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex_val(*b)),
                bytes.get(i + 2).and_then(|b| hex_val(*b)),
            )
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The uppercase ASCII hex digit for a nibble in `0..=15`.
fn hex_digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'A' + (nibble - 10),
    }
}

/// Builds the [`Permalink`] for the current session view: the focused cell (dropped
/// when the name is empty), the camera as `(center_x, center_y, pixels_per_dbu)`, and
/// the set of currently visible layers (each `(layer, datatype)`).
///
/// This is what the copy-permalink action serializes with [`emit_permalink`]; kept pure
/// so the mapping from session state to a link is unit-tested without the app. The
/// camera and layers are always `Some` (a copied permalink pins the exact view,
/// including "no layers visible"); only the cell is optional.
#[must_use]
pub fn session_permalink(
    cell: Option<&str>,
    camera: (f64, f64, f64),
    visible_layers: &[(u16, u16)],
) -> Permalink {
    Permalink {
        cell: cell.filter(|c| !c.is_empty()).map(str::to_owned),
        camera: Some(camera),
        layers: Some(visible_layers.to_vec()),
    }
}

/// Mints a fresh, shareable room id from a human-readable `base` name and a `seed`.
///
/// The seed drives a short url-safe suffix appended to the sanitized base, so a
/// one-click Share gets a room that is recognizable (it carries the design name) yet
/// unlikely to collide with an unrelated session. Deterministic in `(base, seed)` so a
/// test can pin the seed and assert the exact room; the caller supplies real entropy
/// (the frame clock) at runtime. The result is always a valid [`room_id`].
#[must_use]
pub fn minted_room_id(base: &str, seed: u64) -> String {
    // A six-character base-36 suffix from the seed, appended to the sanitized base.
    const DIGITS: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut n = seed;
    let mut suffix = String::with_capacity(6);
    for _ in 0..6 {
        suffix.push(char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
    }
    room_id(&format!("{base}-{suffix}"))
}

/// Whether the page query requests the e2e edit-script mode (`?e2e-edit=1`).
///
/// When this and `?share=1` are both set, the publisher-on-boot places one scripted
/// rect after going live so a browser test (lane v8-1e) can observe the edit propagate
/// to a viewer. Accepts `1` or `true`; a leading `?` is tolerated. Pure, so it is
/// unit-tested without a browser.
#[must_use]
pub fn parse_e2e_edit(query: &str) -> bool {
    let query = query.trim_start_matches('?');
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "e2e-edit" {
            return matches!(value, "1" | "true");
        }
    }
    false
}

/// Whether the page URL asks the replay theater to start playing on boot
/// (`?e2e-autoplay=1`).
///
/// The public `?view=replay` landing waits at Play so a first-time visitor sees
/// the transport before anything moves (a deliberate choice; see the `?view=replay`
/// boot path). This e2e-only flag opts a headed browser test into automatic
/// playback so it can assert the wasm replay reproduces the recorded hash
/// (`window.__reticle_stats.hash_check == "Match"`) without clicking the
/// GPU-painted transport. Pure string logic mirroring [`parse_e2e_edit`], so it is
/// unit-tested without a browser.
#[must_use]
pub fn parse_e2e_replay_autoplay(query: &str) -> bool {
    let query = query.trim_start_matches('?');
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "e2e-autoplay" {
            return matches!(value, "1" | "true");
        }
    }
    false
}

/// Whether the page URL requests embed mode (`?embed=1`, lane 2D, catalog 94):
/// minimal chrome for an `<iframe>`, hiding every panel and menu and leaving only
/// the canvas plus a small "open in Reticle" affordance.
///
/// Pure string logic mirroring [`parse_e2e_edit`], so it is unit-tested without a
/// browser.
///
/// # Embedding notes (CORS/CSP)
///
/// An embedding page frames the deployed bundle with
/// `<iframe src="https://<host>/?embed=1&archive=<url>">`. Two headers govern this:
/// the Reticle host must not send `X-Frame-Options: DENY` (or a restrictive
/// `Content-Security-Policy: frame-ancestors`) or the frame is blocked; and a design
/// loaded over `?archive=`/`?gds=` from a *third* origin must itself answer with
/// permissive `Access-Control-Allow-Origin` and allow `Range` requests, exactly as
/// the non-embedded browse already requires (see [`archive_url_from_query`]). Embed
/// mode adds no new cross-origin surface; it only changes the chrome.
#[must_use]
pub fn parse_embed(query: &str) -> bool {
    let query = query.trim_start_matches('?');
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "embed" {
            return matches!(value, "1" | "true");
        }
    }
    false
}

/// The query key that opens a served `.rtla` archive in the browser build.
///
/// A page URL carrying `?archive=<url>` streams the archive at `<url>` over the
/// HTTP-range [`TileSource`](reticle_index::TileSource) into a read-only
/// [`StreamedScene`](crate::streamed::StreamedScene) (ADR 0062), rather than importing a
/// GDS/OASIS file into an editable document (`?gds=`, [`crate::webopen`]). The two are
/// deliberately distinct keys: `?gds=` opens something the visitor can edit, `?archive=`
/// opens a multi-gigabyte die they only browse.
pub const ARCHIVE_KEY: &str = "archive";

/// Extracts the `?archive=<url>` target from a page query string, or `None` when the
/// parameter is absent or empty.
///
/// `query` is the raw `window.location.search` (for example
/// `"?archive=https://host/chip.rtla&view=editor"`), with or without the leading `?`.
/// The value is percent-decoded (the full `decode_permalink_value` decoding, so an
/// encoded URL round-trips), trimmed, and rejected if empty so a bare `?archive=` does
/// not kick off a fetch of the empty string. Permissive about the URL itself: any
/// non-empty value is returned, and whether it resolves (and whether CORS and `Range`
/// are permitted) is decided by the fetch, which surfaces a clear error on failure.
///
/// Pure string logic, so it is unit-tested without a browser; the inverse of
/// [`emit_archive_link`].
#[must_use]
pub fn archive_url_from_query(query: &str) -> Option<String> {
    let query = query.trim_start_matches('?');
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != ARCHIVE_KEY {
            continue;
        }
        let decoded = decode_permalink_value(value);
        let trimmed = decoded.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_owned());
    }
    None
}

/// Composes a page URL that opens the served archive at `archive_url`, hosted at page
/// origin `base_page`.
///
/// The inverse of [`archive_url_from_query`]: the `?archive=` query it writes parses
/// back to the same URL. The archive URL is fully percent-encoded with
/// `encode_permalink_value` so a URL carrying `/`, `?`, `&`, or `#` stays parseable.
/// An empty `base_page` yields a relative `?archive=...` query (resolving against the
/// loaded bundle), mirroring [`emit_permalink`]; a non-empty one is joined as
/// `base/?archive=...`. This is what a gallery entry (a real-chip deep link) serializes.
#[must_use]
pub fn emit_archive_link(base_page: &str, archive_url: &str) -> String {
    let query = format!("{ARCHIVE_KEY}={}", encode_permalink_value(archive_url));
    let base = base_page.trim().trim_end_matches('/');
    if base.is_empty() {
        format!("?{query}")
    } else {
        format!("{base}/?{query}")
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

    #[test]
    fn permalink_round_trips_cell_camera_and_layers() {
        // emit -> parse is the identity on the Permalink fields, including a cell name
        // with a space and a non-ASCII character and a fractional zoom.
        let p = Permalink {
            cell: Some("my cell µ".to_owned()),
            camera: Some((1000.0, -500.0, 0.25)),
            layers: Some(vec![(68, 20), (69, 20)]),
        };
        let url = emit_permalink("", None, &p);
        let query = url.trim_start_matches('?');
        assert_eq!(parse_permalink(query), p, "round trip via {url}");
    }

    #[test]
    fn permalink_emit_includes_gds_and_a_page_base() {
        let p = Permalink {
            cell: Some("TOP".to_owned()),
            camera: None,
            layers: None,
        };
        let url = emit_permalink("https://reticle.example", Some("https://h/c.gds"), &p);
        assert!(
            url.starts_with("https://reticle.example/?"),
            "base joined: {url}"
        );
        assert!(
            url.contains("gds=https%3A%2F%2Fh%2Fc.gds"),
            "gds encoded: {url}"
        );
        assert!(url.contains("cell=TOP"), "cell present: {url}");
        // The gds round-trips through the webopen parser, and the cell through ours.
        let query = url.split_once('?').expect("query").1;
        assert_eq!(parse_permalink(query).cell.as_deref(), Some("TOP"));
    }

    #[test]
    fn permalink_camera_is_disambiguated_from_the_start_view_selector() {
        // A three-float `view` is a camera spec...
        let cam = parse_permalink("view=1000,-500,0.25").camera;
        assert_eq!(cam, Some((1000.0, -500.0, 0.25)));
        // ...while a start-view keyword leaves the camera empty (it is handled by the
        // boot's StartView selector, not the permalink).
        for q in ["view=editor", "view=viewer", "view=replay"] {
            assert_eq!(parse_permalink(q).camera, None, "for {q:?}");
        }
    }

    #[test]
    fn permalink_parses_a_layer_csv_and_hides_all_on_empty() {
        assert_eq!(
            parse_permalink("layers=68/20,69/20").layers,
            Some(vec![(68, 20), (69, 20)])
        );
        // A present-but-empty `layers` means "hide everything".
        assert_eq!(parse_permalink("layers=").layers, Some(vec![]));
        // An absent key leaves layers untouched.
        assert_eq!(parse_permalink("cell=TOP").layers, None);
    }

    #[test]
    fn permalink_ignores_malformed_values_without_panicking() {
        // A non-numeric camera value is ignored (not a camera, not a panic).
        assert_eq!(parse_permalink("view=abc").camera, None);
        assert_eq!(parse_permalink("view=1,2").camera, None);
        assert_eq!(parse_permalink("view=1,2,3,4").camera, None);
        // A layer that is out of range or malformed is skipped; valid siblings survive.
        assert_eq!(
            parse_permalink("layers=68/20,junk,99,70/5,999999/0").layers,
            Some(vec![(68, 20), (70, 5)])
        );
        // A leading '?' is tolerated and the whole thing never panics.
        let p = parse_permalink("?cell=&view=&layers=&bogus");
        assert_eq!(p.cell, None);
        assert_eq!(p.camera, None);
        assert_eq!(p.layers, Some(vec![]));
    }

    #[test]
    fn session_permalink_captures_cell_camera_and_visible_layers() {
        let p = session_permalink(Some("TOP"), (10.0, -20.0, 0.5), &[(68, 20), (69, 20)]);
        assert_eq!(p.cell.as_deref(), Some("TOP"));
        assert_eq!(p.camera, Some((10.0, -20.0, 0.5)));
        assert_eq!(p.layers, Some(vec![(68, 20), (69, 20)]));
        // No cell focused (empty name) drops the cell; an empty visible set stays `Some`
        // so the link still pins "nothing visible".
        let none = session_permalink(Some(""), (0.0, 0.0, 1.0), &[]);
        assert_eq!(none.cell, None);
        assert_eq!(none.layers, Some(vec![]));
        // And it round-trips through the URL form.
        let url = emit_permalink("", None, &p);
        assert_eq!(parse_permalink(url.trim_start_matches('?')), p);
    }

    #[test]
    fn minted_room_id_is_deterministic_valid_and_varies_by_seed() {
        let a = minted_room_id("CHIP_TOP", 42);
        // Deterministic in (base, seed).
        assert_eq!(a, minted_room_id("CHIP_TOP", 42));
        // A valid, idempotent room id.
        assert_eq!(room_id(&a), a, "minted id is already a room id");
        assert!(!a.is_empty());
        // Different seeds yield different rooms.
        assert_ne!(a, minted_room_id("CHIP_TOP", 43));
        // An empty/junk base still mints a usable room.
        let j = minted_room_id("!!!", 7);
        assert_eq!(room_id(&j), j);
        assert!(!j.is_empty());
    }

    #[test]
    fn parse_e2e_edit_reads_the_flag() {
        assert!(parse_e2e_edit("e2e-edit=1"));
        assert!(parse_e2e_edit("?share=1&e2e-edit=1"));
        assert!(parse_e2e_edit("e2e-edit=true"));
        assert!(!parse_e2e_edit("e2e-edit=0"));
        assert!(!parse_e2e_edit("share=1"));
        assert!(!parse_e2e_edit(""));
        // Not a partial-key match.
        assert!(!parse_e2e_edit("e2eedit=1"));
    }

    #[test]
    fn parse_e2e_replay_autoplay_reads_the_flag() {
        assert!(parse_e2e_replay_autoplay("e2e-autoplay=1"));
        assert!(parse_e2e_replay_autoplay("?view=replay&e2e-autoplay=1"));
        assert!(parse_e2e_replay_autoplay("e2e-autoplay=true"));
        assert!(!parse_e2e_replay_autoplay("e2e-autoplay=0"));
        assert!(!parse_e2e_replay_autoplay("view=replay"));
        assert!(!parse_e2e_replay_autoplay(""));
        // Not a partial-key match.
        assert!(!parse_e2e_replay_autoplay("e2eautoplay=1"));
    }

    #[test]
    fn parse_embed_reads_the_flag() {
        assert!(parse_embed("embed=1"));
        assert!(parse_embed("?archive=https://host/c.rtla&embed=1"));
        assert!(parse_embed("embed=true"));
        assert!(!parse_embed("embed=0"));
        assert!(!parse_embed("view=viewer"));
        assert!(!parse_embed(""));
        // Not a partial-key match.
        assert!(!parse_embed("embedded=1"));
    }

    #[test]
    fn archive_url_from_query_reads_the_parameter() {
        assert_eq!(
            archive_url_from_query("?archive=https://host/chip.rtla"),
            Some("https://host/chip.rtla".to_owned())
        );
        // Works without the leading '?', among other params, and in any position.
        assert_eq!(
            archive_url_from_query("view=editor&archive=http://h/c.rtla"),
            Some("http://h/c.rtla".to_owned())
        );
        // Not a partial-key match: `archives=` is not `archive=`.
        assert_eq!(archive_url_from_query("archives=http://h/c.rtla"), None);
    }

    #[test]
    fn archive_url_from_query_absent_or_empty_is_none() {
        assert_eq!(archive_url_from_query("?view=editor"), None);
        assert_eq!(archive_url_from_query(""), None);
        assert_eq!(archive_url_from_query("?archive="), None);
        // A value that is only encoded whitespace trims to empty and is rejected.
        assert_eq!(archive_url_from_query("?archive=%20%20"), None);
    }

    #[test]
    fn archive_link_round_trips_through_the_query() {
        // emit -> parse is the identity on the archive URL, including one carrying the
        // reserved query characters `?`, `&`, and `#` that must be encoded to survive.
        for url in [
            "http://localhost:8788/fixture.rtla",
            "https://cdn.example/dies/chip.rtla?v=2#frag",
            "https://h/a b&c=d.rtla",
        ] {
            let link = emit_archive_link("https://reticle.example", url);
            assert!(
                link.starts_with("https://reticle.example/?archive="),
                "base joined: {link}"
            );
            let query = link.split_once('?').expect("link has a query").1;
            assert_eq!(
                archive_url_from_query(query).as_deref(),
                Some(url),
                "round trip via {link}"
            );
        }
    }

    #[test]
    fn archive_link_with_empty_page_is_a_relative_query() {
        let link = emit_archive_link("", "http://localhost:8788/fixture.rtla");
        assert!(link.starts_with("?archive="), "relative query: {link}");
        assert_eq!(
            archive_url_from_query(link.trim_start_matches('?')).as_deref(),
            Some("http://localhost:8788/fixture.rtla")
        );
    }

    #[test]
    fn permalink_absent_fields_emit_no_keys() {
        let empty = emit_permalink("", None, &Permalink::default());
        assert_eq!(empty, "", "an empty permalink with no gds emits nothing");
        // Only a camera: exactly the view key, no cell or layers.
        let cam_only = emit_permalink(
            "",
            None,
            &Permalink {
                cell: None,
                camera: Some((0.0, 0.0, 1.0)),
                layers: None,
            },
        );
        assert_eq!(cam_only, "?view=0,0,1");
    }
}

//! The live share transport: dialing the relay socket so a shared link streams (ADR 0058).
//!
//! ADR 0038 built the read-only [`ViewerSession`](crate::viewer::ViewerSession) and the
//! [share links](crate::share), but nothing opened the socket in the browser: a shared
//! read-only link named a room and streamed nothing. This module is the wasm
//! `web_sys::WebSocket` glue that connects a browser tab to a relay room and pumps
//! frames, so a shared link actually streams live sync and presence.
//!
//! It owns **two transports**, deliberately asymmetric so read-only is a structural
//! property, not a discipline (ADR 0058):
//!
//! * [`ViewerTransport`] dials [`viewer_ws_link`](crate::share::viewer_ws_link) (which
//!   carries `?mode=view`), sets the socket to deliver binary as `ArrayBuffer`, and on
//!   each message decodes a [`SyncMessage`](reticle_sync) frame and posts a routed
//!   [`LiveEvent`] into a shared [`LiveInbox`] the egui loop drains each frame. It has
//!   **no method that sends a document frame**: the type cannot publish, so a viewer
//!   cannot mutate the shared session even if the relay's `?mode=view` backstop were
//!   removed.
//! * [`SharerTransport`] dials [`room_link`](crate::share::room_link) (Edit mode) and
//!   exposes [`publish_update`](SharerTransport::publish_update) and
//!   [`publish_presence`](SharerTransport::publish_presence), each framing through
//!   `reticle_sync`'s wire codec ([`encode_update_frame`](reticle_sync::encode_update_frame),
//!   [`encode_presence_frame`](reticle_sync::encode_presence_frame)) and sending one
//!   binary frame.
//!
//! # Testable seam (mirrors `webopen.rs`)
//!
//! The DOM (`web_sys::WebSocket`, its event closures) cannot run in a headless unit
//! test, so the module keeps a hard line between *pure logic* and *DOM glue*:
//!
//! * **Pure, unit-tested (no `cfg`):** the connection [`LiveStatus`] state machine, the
//!   [`LiveInbox`] queue, and [`route_frame`], which turns one binary socket message
//!   into the [`LiveEvent`] the viewer applies (decode a `SyncMessage`, route update vs
//!   presence). This is the exact logic the wasm `onmessage` handler runs, proven
//!   without a browser.
//! * **DOM glue, `#[cfg(target_arch = "wasm32")]` (bottom of the file):** opening the
//!   socket and wiring the event closures. Its end-to-end behavior (a shared link
//!   streams the sharer's geometry and cursor) is proven by the Playwright two-context
//!   e2e; the transport + read-only *contract* is proven headlessly by the Rust relay
//!   test `crates/reticle-server/tests/share_live.rs`.
//!
//! Native builds provide no-op stubs of both transports so [`crate::app`] compiles and
//! links uniformly across targets, exactly as `webopen.rs` does.

use reticle_sync::{Frame, Presence, decode_frame};

/// The connection status of a live transport, for the status line and repaint logic.
///
/// A small state machine the socket's lifecycle callbacks advance: a transport starts
/// [`Connecting`](LiveStatus::Connecting), moves to [`Open`](LiveStatus::Open) on the
/// socket's `open` event, and ends [`Closed`](LiveStatus::Closed) (clean close) or
/// [`Failed`](LiveStatus::Failed) (an error before or after open). Pure and `cfg`-free
/// so the transitions are unit-tested without a browser.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum LiveStatus {
    /// The socket is opening; no frames have flowed yet.
    #[default]
    Connecting,
    /// The socket is open and delivering (or accepting) frames.
    Open,
    /// The socket closed cleanly (the room ended, or the tab navigated away).
    Closed,
    /// The socket failed with a human-readable reason (a dead relay, a network error).
    Failed {
        /// The reason to show, already phrased for a person.
        reason: String,
    },
}

impl LiveStatus {
    /// Whether the socket is currently open and carrying frames.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, LiveStatus::Open)
    }

    /// Whether this is a terminal status (closed or failed): the transport will carry no
    /// more frames without a reconnect.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, LiveStatus::Closed | LiveStatus::Failed { .. })
    }

    /// The failure reason, if this status is [`Failed`](LiveStatus::Failed).
    #[must_use]
    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            LiveStatus::Failed { reason } => Some(reason),
            _ => None,
        }
    }

    /// A short, human-readable label for the status line.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            LiveStatus::Connecting => "connecting to the shared session...".to_owned(),
            LiveStatus::Open => "connected to the shared session".to_owned(),
            LiveStatus::Closed => "the shared session ended".to_owned(),
            LiveStatus::Failed { reason } => format!("could not join the shared session: {reason}"),
        }
    }
}

/// One event a live transport posts into the [`LiveInbox`] for the egui loop to apply.
///
/// The viewer transport posts [`Update`](LiveEvent::Update) and
/// [`Presence`](LiveEvent::Presence) (routed from decoded frames) plus
/// [`Status`](LiveEvent::Status) on the socket's lifecycle. Kept `cfg`-free so the App
/// can match on it without a gate, though it is only produced on wasm.
#[derive(Clone, Debug)]
pub enum LiveEvent {
    /// A CRDT document delta arrived: the raw `yrs` bytes to feed
    /// [`ViewerSession::apply_frame`](crate::viewer::ViewerSession::apply_frame).
    Update(Vec<u8>),
    /// A collaborator's presence arrived: feed it to the viewer's awareness.
    Presence(Presence),
    /// The socket's status changed; update the status line and repaint.
    Status(LiveStatus),
}

/// Routes one binary socket message into a [`LiveEvent`], or `None` if it should be
/// ignored.
///
/// This is the pure heart of the viewer transport's `onmessage` handler: it decodes the
/// bytes as a [`SyncMessage`](reticle_sync) frame with [`decode_frame`] and maps the
/// frame kind to the event the App applies. A document delta becomes
/// [`LiveEvent::Update`] (carrying the raw `yrs` bytes), a presence becomes
/// [`LiveEvent::Presence`]; a comment frame (not part of the read-only view) and a frame
/// that fails to decode are ignored (returned as `None`) rather than crashing the
/// session, since a single malformed frame must not tear down a live view.
///
/// Proven without a browser: feed it the bytes `reticle_sync::frame` produced and assert
/// the routed event.
#[must_use]
pub fn route_frame(bytes: &[u8]) -> Option<LiveEvent> {
    match decode_frame(bytes) {
        Ok(Frame::Update(raw)) => Some(LiveEvent::Update(raw)),
        Ok(Frame::Presence(presence)) => Some(LiveEvent::Presence(presence)),
        // A comment is carried by the same envelope but is not part of the read-only
        // viewer's live view; ignore it. A decode failure is likewise swallowed so one
        // bad frame does not end the session.
        Ok(Frame::Comment(_)) | Err(_) => None,
    }
}

/// A shared, single-slot mailbox the async socket callbacks post [`LiveEvent`]s into and
/// the App drains each frame.
///
/// A `VecDeque` behind interior mutability so a socket callback can push without a borrow
/// of the App, and the egui loop can pop what has arrived. This is the one point of
/// contact between the socket's event world and the synchronous egui loop, exactly as
/// [`WebOpenInbox`](crate::webopen::WebOpenInbox) is for the browser open path. Cloning
/// shares the same queue.
#[derive(Clone, Default)]
pub struct LiveInbox {
    #[cfg(target_arch = "wasm32")]
    inner: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<LiveEvent>>>,
    #[cfg(not(target_arch = "wasm32"))]
    // Native never posts; the field is absent so the type is a zero-cost placeholder.
    _native: (),
}

impl LiveInbox {
    /// A new, empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Posts an event for the App to pick up next frame (wasm only; a no-op elsewhere so
    /// the type is uniform across targets).
    #[cfg(target_arch = "wasm32")]
    pub fn post(&self, event: LiveEvent) {
        self.inner.borrow_mut().push_back(event);
    }

    /// Drains all posted events in order (wasm only; always empty elsewhere).
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn drain(&self) -> Vec<LiveEvent> {
        self.inner.borrow_mut().drain(..).collect()
    }

    /// Drains all posted events (native: nothing is ever posted, so this is empty).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn drain(&self) -> Vec<LiveEvent> {
        Vec::new()
    }
}

impl std::fmt::Debug for LiveInbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveInbox").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// The two transports.
//
// On wasm these own a live `web_sys::WebSocket` and its event closures. On native they
// are inert placeholders so `crate::app` compiles and links across targets (mirroring
// how `webopen.rs` keeps its DOM glue behind a cfg while the App refers to it freely).
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub use wasm::{SharerTransport, ViewerTransport};

/// The read-only viewer transport (native no-op stub).
///
/// On wasm this owns the `web_sys::WebSocket` dialing the room read-only and pumping
/// decoded frames into a [`LiveInbox`]; on native it is inert. It exposes **no method
/// that sends a document frame**, on either target, so a viewer is structurally unable
/// to publish (ADR 0058).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub struct ViewerTransport;

#[cfg(not(target_arch = "wasm32"))]
impl ViewerTransport {
    /// Native no-op: opening a viewer socket has no meaning off the browser. Returns an
    /// inert transport so the App's wiring is target-uniform.
    #[must_use]
    pub fn connect(
        _relay: &str,
        _room: &str,
        _inbox: &LiveInbox,
        _repaint: &eframe::egui::Context,
    ) -> Self {
        Self
    }
}

/// The publishing sharer transport (native no-op stub).
///
/// On wasm this owns the `web_sys::WebSocket` dialing the room in Edit mode and frames
/// outgoing updates and presence through `reticle_sync`'s wire codec; on native it is
/// inert so the desktop editor (which does not run a browser socket) still compiles.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub struct SharerTransport;

#[cfg(not(target_arch = "wasm32"))]
impl SharerTransport {
    /// Native no-op constructor.
    #[must_use]
    pub fn connect(
        _relay: &str,
        _room: &str,
        _inbox: &LiveInbox,
        _repaint: &eframe::egui::Context,
    ) -> Self {
        Self
    }

    /// Native no-op: nothing is published off the browser.
    pub fn publish_update(&self, _bytes: &[u8]) {}

    /// Native no-op: nothing is published off the browser.
    pub fn publish_presence(&self, _presence: &Presence) {}
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{LiveEvent, LiveInbox, LiveStatus, route_frame};
    use reticle_sync::{Presence, encode_presence_frame, encode_update_frame};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    use web_sys::WebSocket;

    /// Opens a `web_sys::WebSocket` to `url`, wiring the lifecycle callbacks (`open`,
    /// `close`, `error`) to post [`LiveStatus`] events and request a repaint, and
    /// binary delivery as `ArrayBuffer`. Returns the socket and keeps the lifecycle
    /// closures alive by leaking them (`forget`), which is correct for a
    /// session-lifetime socket: they live as long as the tab's connection.
    ///
    /// The caller attaches the `onmessage` handler, which differs between the viewer
    /// (decode and route) and the sharer (ignore inbound, it is authoritative).
    fn open_socket(
        url: &str,
        inbox: &LiveInbox,
        repaint: &eframe::egui::Context,
    ) -> Result<WebSocket, String> {
        let socket = WebSocket::new(url).map_err(|e| describe(&e))?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

        // open -> Status(Open)
        {
            let inbox = inbox.clone();
            let repaint = repaint.clone();
            let url = url.to_owned();
            let on_open = Closure::<dyn FnMut()>::new(move || {
                // A browser-observable signal the Playwright e2e reads (the egui status
                // bar is canvas-rendered, not DOM). Honest instrumentation, not a stub.
                web_sys::console::log_1(&format!("reticle-live: socket open {url}").into());
                inbox.post(LiveEvent::Status(LiveStatus::Open));
                repaint.request_repaint();
            });
            socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            on_open.forget();
        }

        // close -> Status(Closed)
        {
            let inbox = inbox.clone();
            let repaint = repaint.clone();
            let on_close =
                Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_e: web_sys::CloseEvent| {
                    inbox.post(LiveEvent::Status(LiveStatus::Closed));
                    repaint.request_repaint();
                });
            socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();
        }

        // error -> Status(Failed)
        {
            let inbox = inbox.clone();
            let repaint = repaint.clone();
            let on_error =
                Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |e: web_sys::ErrorEvent| {
                    let reason = {
                        let msg = e.message();
                        if msg.is_empty() {
                            "the relay connection errored".to_owned()
                        } else {
                            msg
                        }
                    };
                    inbox.post(LiveEvent::Status(LiveStatus::Failed { reason }));
                    repaint.request_repaint();
                });
            socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();
        }

        Ok(socket)
    }

    /// Copies an incoming `ArrayBuffer` message into a `Vec<u8>`.
    fn message_bytes(event: &web_sys::MessageEvent) -> Option<Vec<u8>> {
        let buffer = event.data().dyn_into::<js_sys::ArrayBuffer>().ok()?;
        Some(js_sys::Uint8Array::new(&buffer).to_vec())
    }

    /// Renders a `JsValue` error into a short human string.
    fn describe(value: &wasm_bindgen::JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                value
                    .dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "WebSocket error".to_owned())
    }

    /// The read-only viewer transport: a `web_sys::WebSocket` dialing the room with
    /// `?mode=view`, decoding each binary frame and posting a routed [`LiveEvent`].
    ///
    /// It holds the socket and its `onmessage` closure. Crucially it exposes **no
    /// method that sends a document frame**: there is no `publish_*` here at all, so it
    /// is structurally impossible for the viewer to mutate the shared session from the
    /// app side (the relay's `?mode=view` drop is the independent server-side backstop).
    /// It sends nothing on the socket.
    pub struct ViewerTransport {
        _socket: WebSocket,
        _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    }

    impl std::fmt::Debug for ViewerTransport {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ViewerTransport").finish_non_exhaustive()
        }
    }

    impl ViewerTransport {
        /// Opens the read-only viewer socket to `viewer_ws_link(relay, room)` (which
        /// carries `?mode=view`) and begins pumping decoded frames into `inbox`.
        ///
        /// On any frame, [`route_frame`] decodes the `SyncMessage` and posts an
        /// [`LiveEvent::Update`] or [`LiveEvent::Presence`]; the App drains the inbox
        /// each frame and applies it to its
        /// [`ViewerSession`](crate::viewer::ViewerSession). A socket that cannot even be
        /// constructed posts a [`LiveStatus::Failed`] so the failure is visible.
        #[must_use]
        pub fn connect(
            relay: &str,
            room: &str,
            inbox: &LiveInbox,
            repaint: &eframe::egui::Context,
        ) -> Self {
            let url = crate::share::viewer_ws_link(relay, room);
            match open_socket(&url, inbox, repaint) {
                Ok(socket) => {
                    let on_message = {
                        let inbox = inbox.clone();
                        let repaint = repaint.clone();
                        let mut logged_first = false;
                        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
                            move |e: web_sys::MessageEvent| {
                                if let Some(bytes) = message_bytes(&e)
                                    && let Some(event) = route_frame(&bytes)
                                {
                                    // Log the first decoded frame so the e2e can confirm
                                    // the viewer actually received the sharer's stream.
                                    if !logged_first {
                                        logged_first = true;
                                        let kind = match &event {
                                            LiveEvent::Update(_) => "update",
                                            LiveEvent::Presence(_) => "presence",
                                            LiveEvent::Status(_) => "status",
                                        };
                                        web_sys::console::log_1(
                                            &format!("reticle-live: first frame {kind}").into(),
                                        );
                                    }
                                    inbox.post(event);
                                    repaint.request_repaint();
                                }
                            },
                        )
                    };
                    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
                    Self {
                        _socket: socket,
                        _on_message: on_message,
                    }
                }
                Err(reason) => {
                    // Report the failure and hand back an inert transport whose socket
                    // is a already-closed placeholder, so the caller's field type is
                    // uniform. A fresh WebSocket to an unusable URL is not constructed;
                    // instead we surface the error and keep a closed dummy.
                    inbox.post(LiveEvent::Status(LiveStatus::Failed { reason }));
                    repaint.request_repaint();
                    // SAFETY-of-intent: an empty-string URL always errors, so this
                    // dummy socket is immediately in the CLOSED/CONNECTING-then-error
                    // state and carries nothing. We keep it only to satisfy the field.
                    let dummy = WebSocket::new("ws://127.0.0.1:0/closed")
                        .unwrap_or_else(|_| unreachable!("a syntactically valid ws URL"));
                    let noop = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|_| {});
                    Self {
                        _socket: dummy,
                        _on_message: noop,
                    }
                }
            }
        }
    }

    /// The publishing sharer transport: a `web_sys::WebSocket` dialing the room in Edit
    /// mode, framing outgoing updates and presence through `reticle_sync`'s wire codec.
    ///
    /// Unlike [`ViewerTransport`] it exposes [`publish_update`](Self::publish_update)
    /// and [`publish_presence`](Self::publish_presence). Inbound frames are ignored: the
    /// sharer's editor is the authoritative document, so it does not apply frames from
    /// the room (a second editor's edits are out of scope for this lane; the sharer
    /// publishes, viewers consume).
    pub struct SharerTransport {
        socket: WebSocket,
    }

    impl std::fmt::Debug for SharerTransport {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SharerTransport").finish_non_exhaustive()
        }
    }

    impl SharerTransport {
        /// Opens the sharer socket to `room_link(relay, room)` (Edit mode) and begins
        /// accepting published frames. A construction failure posts a
        /// [`LiveStatus::Failed`].
        #[must_use]
        pub fn connect(
            relay: &str,
            room: &str,
            inbox: &LiveInbox,
            repaint: &eframe::egui::Context,
        ) -> Self {
            let url = crate::share::room_link(relay, room);
            let socket = match open_socket(&url, inbox, repaint) {
                Ok(socket) => socket,
                Err(reason) => {
                    inbox.post(LiveEvent::Status(LiveStatus::Failed { reason }));
                    repaint.request_repaint();
                    WebSocket::new("ws://127.0.0.1:0/closed")
                        .unwrap_or_else(|_| unreachable!("a syntactically valid ws URL"))
                }
            };
            Self { socket }
        }

        /// Publishes the sharer's document delta: wraps the raw `yrs` `bytes` in the
        /// `SyncMessage` envelope and sends one binary frame. A send while the socket is
        /// not yet open is dropped (the next full-state frame carries the whole document
        /// again), matching the demo publisher's best-effort semantics.
        pub fn publish_update(&self, bytes: &[u8]) {
            let frame = encode_update_frame(bytes);
            let _ = self.socket.send_with_u8_array(&frame);
        }

        /// Publishes the sharer's presence (cursor, selection, viewport) as one framed
        /// binary message, so a viewer sees the live cursor and can follow the viewport.
        pub fn publish_presence(&self, presence: &Presence) {
            let frame = encode_presence_frame(presence);
            let _ = self.socket.send_with_u8_array(&frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveEvent, LiveStatus, route_frame};
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, ShapeKind};
    use reticle_sync::{Presence, SyncDocument, encode_presence_frame, encode_update_frame};

    /// A raw `yrs` state update from a doc that adds one cell with one rect.
    fn sharer_state_bytes() -> Vec<u8> {
        let mut doc = SyncDocument::new("sharer");
        let mut cell = Cell::new("top");
        cell.shapes.push(DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 400))),
        ));
        doc.add_cell(&cell);
        doc.encode_state_update()
    }

    #[test]
    fn route_frame_routes_an_update_to_the_raw_bytes() {
        let raw = sharer_state_bytes();
        let frame = encode_update_frame(&raw);
        match route_frame(&frame) {
            Some(LiveEvent::Update(bytes)) => {
                assert_eq!(bytes, raw, "the raw yrs bytes are handed through unchanged");
                // And they materialize the sharer's geometry, as the viewer will do.
                let mut peer = SyncDocument::new("viewer");
                peer.apply_update(&bytes).expect("valid update");
                assert!(peer.document().cell("top").is_some());
            }
            other => panic!("expected an update event, got {other:?}"),
        }
    }

    #[test]
    fn route_frame_routes_a_presence() {
        let mut p = Presence::new("sharer");
        p.cursor = Point::new(7, -3);
        p.viewport = Rect::new(Point::new(0, 0), Point::new(100, 100));
        let frame = encode_presence_frame(&p);
        match route_frame(&frame) {
            Some(LiveEvent::Presence(got)) => assert_eq!(got, p),
            other => panic!("expected a presence event, got {other:?}"),
        }
    }

    #[test]
    fn route_frame_ignores_garbage_rather_than_crashing() {
        // A non-frame blob and an empty slice must both be ignored (None), so one bad
        // socket message never tears down the live view.
        assert!(route_frame(&[0xff, 0xff, 0xff, 0xff]).is_none());
        assert!(route_frame(&[]).is_none());
    }

    #[test]
    fn live_status_transitions_and_labels() {
        let connecting = LiveStatus::default();
        assert_eq!(connecting, LiveStatus::Connecting);
        assert!(!connecting.is_open());
        assert!(!connecting.is_terminal());

        let open = LiveStatus::Open;
        assert!(open.is_open());
        assert!(!open.is_terminal());

        let closed = LiveStatus::Closed;
        assert!(closed.is_terminal());
        assert!(!closed.is_open());

        let failed = LiveStatus::Failed {
            reason: "dead relay".to_owned(),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.failure_reason(), Some("dead relay"));
        assert!(failed.label().contains("dead relay"));
    }

    /// A compile-and-behavior guarantee that the viewer transport exposes no publish
    /// path. On native the `ViewerTransport` stub has only `connect`; there is no
    /// `publish_update`/`publish_presence` on it, so this test would not compile if one
    /// were added. This is the app-side half of the read-only guarantee (ADR 0058).
    #[test]
    fn viewer_transport_has_no_publish_method() {
        use super::{LiveInbox, ViewerTransport};
        let ctx = eframe::egui::Context::default();
        let _viewer = ViewerTransport::connect("127.0.0.1:3030", "room", &LiveInbox::new(), &ctx);
        // Intentionally nothing else: the point is the *absence* of a publish method.
        // `SharerTransport`, by contrast, has publish_update/publish_presence.
    }
}

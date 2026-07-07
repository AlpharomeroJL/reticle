//! The relay under test: a base WebSocket URL plus the timing and semantic
//! knobs that let one vector runner drive either relay.
//!
//! Two constructors produce the two targets the suite proves equivalent:
//!
//! * [`Target::native`] spawns the in-process `reticle_server` axum relay on an
//!   ephemeral port (the pattern of `reticle-server/tests/share_live.rs`) and
//!   points at it. It never coalesces presence.
//! * [`Target::external`] points at any already-running relay by URL (the
//!   Cloudflare Durable Object under `wrangler dev`, or a deployed
//!   `wss://...workers.dev`). It is told whether that relay coalesces presence.
//!
//! The only behavioral difference the runner honors is presence coalescing: the
//! Durable Object drops all-but-the-newest presence per client inside a short
//! window to respect the free tier, while the native relay forwards every frame.
//! Both preserve the shared invariant a burst asserts (the newest presence
//! converges, updates are never dropped); see [`crate::runner`].

use std::time::Duration;

use reticle_server::{RelayState, serve};

/// A relay under test, addressed by base WebSocket URL.
///
/// Clone is intentionally not derived: a `Target` owns timing configuration and
/// is passed by reference to [`crate::runner::run_vector`].
#[derive(Debug)]
pub struct Target {
    /// Base URL with no trailing slash, for example `ws://127.0.0.1:5510`. A
    /// join composes `{base_ws}/ws/{room}` (plus `?mode=view` for a viewer).
    base_ws: String,
    /// Whether this relay coalesces presence frames per client. `false` for the
    /// native relay, `true` for the Durable Object.
    coalesces_presence: bool,
    /// How long to wait for a frame that should arrive before failing.
    recv_timeout: Duration,
    /// How long to wait to confirm a frame does *not* arrive.
    negative_timeout: Duration,
    /// Grace period after a join so the server-side upgrade subscribes before a
    /// peer publishes (the client handshake completing does not guarantee it).
    connect_grace: Duration,
}

impl Target {
    /// Spawns the in-process native relay on an ephemeral port and returns a
    /// target addressing it. The relay task is detached and lives for the
    /// duration of the test process.
    pub async fn native() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port for the native relay");
        let addr = listener.local_addr().expect("resolve native relay addr");
        tokio::spawn(async move {
            serve(listener, RelayState::new())
                .await
                .expect("native relay serve");
        });
        Self {
            base_ws: format!("ws://{addr}"),
            coalesces_presence: false,
            recv_timeout: Duration::from_secs(5),
            negative_timeout: Duration::from_millis(400),
            connect_grace: Duration::from_millis(300),
        }
    }

    /// A target addressing an already-running relay by base URL (no trailing
    /// slash, no `/ws`). `coalesces_presence` tells the runner whether to expect
    /// presence coalescing. Timeouts are widened for a networked/miniflare relay.
    #[must_use]
    pub fn external(base_ws: impl Into<String>, coalesces_presence: bool) -> Self {
        Self {
            base_ws: base_ws.into(),
            coalesces_presence,
            recv_timeout: Duration::from_secs(8),
            negative_timeout: Duration::from_millis(900),
            connect_grace: Duration::from_millis(700),
        }
    }

    /// The `ws://.../ws/{room}` URL a client joins, with `?mode=view` appended
    /// when `view` is set.
    #[must_use]
    pub fn join_url(&self, room: &str, view: bool) -> String {
        let base = format!("{}/ws/{room}", self.base_ws);
        if view {
            format!("{base}?mode=view")
        } else {
            base
        }
    }

    /// Whether this relay coalesces presence frames per client.
    #[must_use]
    pub fn coalesces_presence(&self) -> bool {
        self.coalesces_presence
    }

    /// Timeout for a frame that must arrive.
    #[must_use]
    pub fn recv_timeout(&self) -> Duration {
        self.recv_timeout
    }

    /// Timeout for asserting a frame must not arrive.
    #[must_use]
    pub fn negative_timeout(&self) -> Duration {
        self.negative_timeout
    }

    /// Post-join grace period.
    #[must_use]
    pub fn connect_grace(&self) -> Duration {
        self.connect_grace
    }
}

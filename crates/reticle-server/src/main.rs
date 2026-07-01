//! The Reticle collaboration relay binary.
//!
//! This is a thin entry point: it resolves the bind address, constructs a
//! default [`RelayState`], and serves the WebSocket relay. All relay logic lives
//! in the [`reticle_server`] library crate so it can be unit- and
//! integration-tested without a live socket.

use reticle_server::{RelayState, bind_address, serve};

/// Binds the relay's listener and serves it on a multi-threaded runtime.
///
/// The address comes from the `RETICLE_SERVER_ADDR` environment variable and
/// defaults to `127.0.0.1:3030`.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = bind_address();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("reticle-server: collaboration relay listening on ws://{addr}/ws/{{room}}");
    serve(listener, RelayState::new()).await
}

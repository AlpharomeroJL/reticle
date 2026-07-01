//! The Reticle collaboration relay.
//!
//! Wave 3 implements an `axum` + `tokio` WebSocket relay: rooms, awareness,
//! update broadcast, initial-state sync, and a persistence hook. It carries no
//! business logic beyond relaying CRDT and presence messages.

fn main() {
    // Wave 3: bind the axum router and serve the WebSocket relay.
    println!("reticle-server: collaboration relay (Wave 3 stub)");
}

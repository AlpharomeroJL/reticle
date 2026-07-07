//! The Durable Object half of the conformance gate: the identical vector table
//! against the Cloudflare relay under `wrangler dev --local` (miniflare; no
//! Cloudflare auth). Gated by `RETICLE_CONFORMANCE_DO=1` so a plain
//! `cargo nextest run` (and `just ci`) stays Node-free; `just conformance` sets
//! the flag after ensuring `worker/node_modules` exists.
//!
//! The test spawns `npx wrangler dev` inside `worker/`, polls until the room
//! endpoint accepts a WebSocket, runs every shared vector plus the negative
//! vector, then kills the process tree. It proves the two relays return
//! identical verdicts: the same table that passes in-process against the native
//! relay passes over a real socket against the Durable Object, and the
//! deliberately-broken vector fails against it too.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reticle_relay_conformance::vectors::broken_expects_view_frame_forwarded;
use reticle_relay_conformance::{Target, run_vector, vectors};

/// The `worker/` directory, resolved from this crate's manifest dir.
fn worker_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("worker")
}

/// Grabs a free TCP port by binding to `:0` and releasing it.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// A `wrangler dev` child process and its whole tree, killed on drop.
struct WranglerDev {
    child: Child,
}

impl WranglerDev {
    /// Spawns `npx wrangler dev --local --port {port}` in `worker/`.
    fn spawn(port: u16) -> Self {
        // `cmd /C` so Windows resolves `npx.cmd`; killing the cmd tree later
        // reaps node/workerd. Output is discarded; readiness is polled over TCP.
        let child = Command::new("cmd")
            .args([
                "/C",
                "npx",
                "wrangler",
                "dev",
                "--local",
                "--port",
                &port.to_string(),
            ])
            .current_dir(worker_dir())
            // Isolate the worker's wasm build from the test's native target dir so
            // the two concurrent cargo invocations never contend on a lock.
            .env("CARGO_TARGET_DIR", worker_dir().join("target"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `npx wrangler dev` (is worker/node_modules installed?)");
        Self { child }
    }

    /// Kills the whole process tree (wrangler spawns workerd/miniflare children).
    fn kill(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &self.child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for WranglerDev {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Polls the room endpoint until a WebSocket connects, or times out. The first
/// spawn compiles the worker, so the budget is generous.
async fn wait_ready(base: &str) -> bool {
    let url = format!("{base}/ws/readycheck");
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if let Ok((mut ws, _)) = tokio_tungstenite::connect_async(&url).await {
            let _ = ws.close(None).await;
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Every shared vector passes against the Durable Object, and the negative
/// vector fails against it: the two relays return identical verdicts.
#[tokio::test]
async fn every_vector_passes_against_the_durable_object() {
    if std::env::var("RETICLE_CONFORMANCE_DO").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping Durable Object conformance: set RETICLE_CONFORMANCE_DO=1 \
             (or run `just conformance`, which manages worker/node_modules)"
        );
        return;
    }

    let port = free_port();
    let base = format!("ws://127.0.0.1:{port}");
    let mut dev = WranglerDev::spawn(port);

    if !wait_ready(&base).await {
        dev.kill();
        panic!("wrangler dev did not become ready within the timeout");
    }

    let target = Target::external(&base, true);
    let mut failures = Vec::new();
    for vector in vectors() {
        if let Err(failure) = run_vector(&target, &vector).await {
            failures.push(failure.to_string());
        }
    }
    let negative = run_vector(&target, &broken_expects_view_frame_forwarded()).await;

    dev.kill();

    assert!(
        failures.is_empty(),
        "Durable Object conformance failures:\n{}",
        failures.join("\n")
    );
    assert!(
        negative.is_err(),
        "the negative vector must fail against the Durable Object too (harness has teeth)"
    );
}

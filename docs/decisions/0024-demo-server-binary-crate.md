# 0024, The demo server binary lives in its own composition crate

## Context

Lane 2H ships `just demo-up`: a runnable demo server that exposes the
rate-limited `reticle-demo` service, drives the real `reticle-agent`
propose-verify-correct loop when an API key is present, and streams the agent's
edits into a `reticle-server` collaboration room a spectator can watch.

That composition needs, in one process: `reticle-demo` (the `LimitConfig` and the
axum service), `reticle-agent` (the model client, the loop, and the
`AgentCollaborator` bridge from ADR 0022), `reticle-server` (the WebSocket relay),
`reticle-proto` (the `CrdtUpdate` wire envelope), a WebSocket client to push
frames to the relay, and `tokio` plus `axum`.

Two constraints pull against just adding a `src/bin/` to an existing crate:

1. The `reticle-demo` library is deliberately dependency-light (serde, axum,
   tokio, tower). Its whole point is that `DemoServer` cannot be constructed
   without a `LimitConfig`, and it must stay small and safe to audit. Pulling
   `reticle-agent` (and thus `ureq`, the DRC engine, the sync CRDT) into its
   normal dependencies would defeat that.

2. `reticle-agent` is a library plus a single-task runner binary. Adding the
   demo server there would drag `reticle-demo`, `reticle-server`, and a WebSocket
   client into `reticle-agent`'s dependency set, coupling the agent library to the
   demo hosting concern.

## Decision

Add a small new workspace member, `reticle-demo-server`, that is a binary only
(`src/main.rs`, no library surface). It depends on `reticle-demo`,
`reticle-agent`, `reticle-server`, `reticle-proto`, `reticle-agent-api`,
`reticle-geometry`, axum, tokio, and `tokio-tungstenite` (the WebSocket client
used to publish CRDT frames to the relay). It composes them:

- builds a `DemoServer` from a non-permissive `LimitConfig` (a demo-tuned default,
  overridable by env) and serves it on `HOST:PORT` (default `127.0.0.1:3040`);
- brings up the `reticle-server` relay in the same process on its own port
  (default `127.0.0.1:3041`, `RETICLE_RELAY_ADDR`) so a spectator can watch the
  room the service hands back, with a documented way to point at an external relay
  instead;
- selects the harness at runtime: the real `reticle-agent`-backed harness when
  `ANTHROPIC_API_KEY` is set, otherwise the offline `MockHarness`.

`reticle-demo` and `reticle-agent` are untouched; this crate only wires them.

## Consequences

- The `reticle-demo` library stays lean and independently auditable; the
  heavyweight composition lives in one clearly-named binary crate.
- `just ci` gains one more small crate to build, but it has no new library
  dependents, so nothing downstream slows down.
- The binary is the single place that decides limits, ports, relay wiring, and
  harness selection, which is exactly the surface a deployment doc and Dockerfile
  target.
- Because the crate is binary-only, its clippy target is `--all-targets` over the
  binary and its smoke test; there is no public API to document beyond the binary.

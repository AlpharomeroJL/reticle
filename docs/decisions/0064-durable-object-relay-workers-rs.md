# 0064, the Durable Object relay: workers-rs over a TypeScript fallback

## Context

Reticle's collaboration relay was a single native binary (`crates/reticle-server`:
axum + tokio, one broadcast channel and an in-memory log per room). We wanted a
second relay that runs on Cloudflare's free plan, so a share session can be hosted
with no server to operate: a Worker routes `?room`/`/ws/{room}` to a per-room
Durable Object (DO) that holds the WebSockets. The wire protocol, not the relay, is
the asset (ADR 0058, 0061): both relays must be observably interchangeable.

The DO must use the runtime's hibernation API, not the classic `addEventListener`
pattern, so an idle room does not pin an isolate: `state.acceptWebSocket(server)`
plus `webSocketMessage`/`webSocketClose` handler methods, per-connection state in a
socket attachment that survives eviction, and the Alarms API (never `setTimeout`,
which blocks hibernation) for any timer. The brief mandated one honest timeboxed
attempt at workers-rs (Rust to wasm) before falling back to a TypeScript DO, with
the exact gap recorded here.

## Decision

Ship the DO in **workers-rs** (`worker` 0.8.5), Rust compiled to wasm. The
hibernation surface is fully present and the code is small: `#[durable_object]` on
`ReticleRoom`, `State::accept_web_socket`, `get_websockets`, `serialize_attachment`
/`deserialize_attachment` for per-connection `{id, view}`, `Storage` for the frame
log and alarms. The relay mirrors the native semantics exactly (join with
`?mode=view`, full ordered log replay before live traffic, view-mode frames
dropped, echo suppression by connection id, binary-only payloads) and adds the
free-tier engineering the native relay does not need: presence frames (first byte
`0x12`) are coalesced per client to about 10 Hz, driven by the DO **alarm** (a
`setTimeout` would block hibernation); the newest presence always converges so
follow-mode still lands on the sharer's latest viewport, while update frames
(`0x0A`) are never coalesced or dropped. The frame log is persisted in fixed 32-
frame storage chunks and replayed in order. Room expiry rides the same alarm: when
it fires with no open sockets, the room's storage is deleted.

The one real gap was in the build toolchain, not the API: `worker` 0.8.5 requires
`wasm-bindgen ^0.2.125`, and the first `worker-build` we installed (the stale
`0.1.14` line) bundled `wasm-bindgen-cli 0.2.105`, whose bindgen schema must match
exactly. Installing the version-matched `worker-build 0.8.5` (which fetches
`wasm-bindgen 0.2.126` and `wasm-opt`) resolved it; the DO then built to wasm and
served under `wrangler dev --local`. Because the resolution was clean, the
TypeScript fallback was not needed. Had it been, the conformance suite is
implementation-agnostic, so it would have cost a rewrite of one file, not the lane.

## Consequences

The relay is one small, well-typed Rust file that shares the project's language and
the frozen first-byte wire invariant (ADR 0061) with the rest of the codebase. The
conformance suite (ADR 0065) proves it returns identical verdicts to the native
relay over a real socket. Two honest limits: presence coalescing means the DO
delivers strictly fewer presence frames than the native relay for a burst (by
design, convergence preserved), which the conformance vectors encode as a
target-aware branch rather than byte-equality; and hibernation eviction is not
provable under miniflare locally, so the hibernation-safe design (attachments +
alarms, no `setTimeout`) is asserted by construction and by the passing vectors,
not by observing a real eviction. Reproducibility rests on a pinned `wrangler`
devDependency and the version-matched `worker-build`; the latter is a local build
tool, not a committed dependency, and is documented here so a future run installs
`worker-build` at the `worker` crate's version, not the newest `0.1.x`.

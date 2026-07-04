# 0039, Share rooms on the demo server: rate-limited creation and a TTL

## Context

A read-only viewer (ADR 0038) joins a relay room the sharer created. On a public
demo deployment, room creation has to be an abuse-controlled resource, for the same
reason agent sessions are (ADR 0024): without a cap a script could mint an unbounded
number of rooms, and without expiry a demo host would retain shared sessions forever.
The demo service already enforced per-IP rate limits and per-IP/global concurrency
caps on the `/submit` path with a sliding-window `RateLimiter` and RAII slots, and
`LimitConfig` is a frozen contract whose fields are pinned by full struct literals
across the code and a round-trip test. Share rooms are a different resource from
agent sessions, though: they carry no prompt, no token or command budget, and no
agent loop, only a relay room id and a lifetime.

## Decision

Add a dedicated `/share` endpoint and a `ShareRooms` registry to `reticle-demo`,
governed by a **separate** `ShareLimits` (per-IP creation rate, room TTL, and a live-room
ceiling) rather than widening the frozen `LimitConfig`. `POST /share` mints a room id
(`share-XXXXXXXX`) and returns it with its TTL in a `ShareResponse`; creation is
rate-limited per source IP with the same sliding-window `RateLimiter` the submit path
uses (a flood is `429`, mirroring a submit flood), and the live-room ceiling maps to
`503` (mirroring the global session cap), so share creation and session submission
report capacity refusals the same way. Each room is stamped with a creation instant
plus the TTL; `create_at` sweeps expired rooms before the capacity check (so aged-out
rooms free capacity) and `is_live_at` reports whether a room is still within its TTL.
The registry is time-injectable, exactly like the `RateLimiter` it reuses, so the
rate-limit and expiry behavior is tested without sleeping. There are no accounts: a
room is anonymous, bounded, and self-expiring.

## Consequences

A public demo can offer share links without becoming an open room factory or a
permanent store: creation floods are rejected per IP, the number of live rooms is
capped, and every room expires on its own, all proven by demo tests in the same
style as the existing abuse suite (a creation flood gets `429`, the live-room ceiling
gets `503`, a room reports live up to its TTL boundary and gone after it, and an
expired room frees capacity for a new one). Keeping `ShareLimits` separate from
`LimitConfig` means the frozen submit-path contract and its round-trip test are
untouched, at the cost of a second small config type the deployment wires with its
own defaults (a handful of rooms per minute per IP, a half-hour TTL, a 256-room
ceiling). The registry tracks only room ids and expiries, not who may join or how
many viewers a room has; enforcing read-only for the joiners themselves is the relay's
job (ADR 0038), and richer per-room policy (max viewers, revocation) is left for when
a real deployment needs it.

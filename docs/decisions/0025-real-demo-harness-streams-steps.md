# 0025, The real demo harness streams atomic steps to the relay

## Context

The demo server drives sessions through the `reticle_demo::Harness` trait. The
built-in `MockHarness` walks a session Queued to Running to Done and honours
cancel and budgets, but draws nothing a spectator can see. Deliverable 1 of Lane
2H is a `Harness` backed by the real `reticle-agent` loop that: runs the
propose-verify-correct loop for the submitted prompt, updates the session status
per iteration, respects cancel and the token/command budget, and streams its
edits into the collaboration room a spectator watches.

`reticle-agent` already has the two pieces this needs, but they are not composed
for a server:

- the propose loop lives in `run::run_agent_task`, which owns its own `Session`
  and writes four on-disk artifacts. Its `context_hook` fires *before* each
  proposal, so it offers no seam to publish an atomic step *after* it applies. It
  is built for a batch CLI run, not a live, cancellable, streaming session.
- the `AgentCollaborator` bridge (ADR 0022) mirrors a batch of `AgentCommand`s
  onto a `SyncDocument` as one atomic `yrs` transaction and publishes presence and
  an `AgentStatus`. It is exactly the right per-step primitive, but nothing drives
  it from a server, and it does not itself put bytes on a socket.

The relay (`reticle-server`) fans opaque binary frames out to every peer in a
room and replays the room log to late joiners. A watcher decodes the frames as
`reticle_proto::v1::SyncMessage` envelopes.

## Decision

Implement `AgentHarness` in `reticle-demo-server`, driving the loop directly over
the public seams rather than through `run_agent_task`, so each step can stream:

1. Build a `ModelClient`: the real `AnthropicModel` (key from env) in production,
   or a deterministic scripted client for tests. The trait, `propose(task, prompt,
   context)`, is the same for both.
2. Per iteration, on a `spawn_blocking` worker (the model client is blocking):
   ask the model for a command batch, charge it against the token and command
   budget, apply it to a private `Session` (the authoritative document and the
   transcript), and count DRC violations for the status line and the correcting
   feedback.
3. Feed the same batch to an `AgentCollaborator`, which applies it as one atomic
   `SyncDocument::step` and publishes presence plus an `AgentStatus`. Then encode
   the collaborator's document as a `yrs` update, wrap it in a `SyncMessage {
   CrdtUpdate { doc_id, actor = AGENT_ACTOR, update } }`, and publish it to the
   relay room over a WebSocket as a binary frame. Any peer in the room receives it
   (or the replayed log on join) and materializes the agent's document, exactly as
   any other `reticle-sync` peer would.
4. Update the `SessionHandle` status (iteration, violation count, message) after
   each step, check the cancel token before and after each step, and settle the
   session into Done, Cancelled, or Error.

Because the whole batch commits as one CRDT transaction and is shipped as one
frame, a spectator never observes a half-drawn step (the ADR 0022 guarantee is
preserved end to end onto the wire).

## Consequences

- The real path is genuinely wired: real model, real DRC verification, real
  budget and cancel enforcement, and real CRDT frames on a real relay socket. It
  is not a mock dressed up as streaming.
- The harness reuses the agent's own command vocabulary and the ADR 0022 bridge,
  so there is no second, drifting translation of edits.
- If the relay connection cannot be established, streaming degrades to a logged
  warning and the loop still runs (status, limits, cancel, and the authoritative
  document are unaffected); the session does not fail because a spectator transport
  is down. This is the honest reduced behaviour, not a silent fake.
- The demo harness does not write the four artifacts the CLI does; it is a live
  session, not a benchmark run. The authoritative document lives in the session and
  is mirrored to the room.
- The default remains `MockHarness` when no key is set, so `just demo-up` runs
  offline with no network and no key.

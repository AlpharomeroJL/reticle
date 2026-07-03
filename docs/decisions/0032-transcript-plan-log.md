# 0032, Transcript gains an additive per-iteration plan log

## Context

v6 adds agent planning transparency: before each propose-verify-correct iteration,
the harness derives a structured plan step (goal, intended tools, expected checks)
and the agent panel renders it, so a viewer sees the agent's stated intent next to
what it did and a later pass can mine stated-plan-versus-outcome divergence.

`Transcript` (in `reticle-agent-api`) is the frozen Wave-0 audit-and-replay record,
and its transcripts are already committed as benchmark fixtures and replay-hash
tested. The plan log has to be stored somewhere durable and viewable, but it must
not disturb the replay contract or invalidate any transcript written before it
existed.

A second constraint comes from an existing invariant: the written transcript carries
only a task's public identifiers, never the free-text prompt, because a prompt can
embed a secret (this is the structural property that keeps an API key out of the
transcript, asserted by `transcript_is_jsonl_and_carries_no_injected_secret`).

## Decision

Add `plan: Vec<PlanStep>` to `Transcript` as an additive field with
`#[serde(default)]`, alongside a new `PlanStep { goal, intended_tools,
expected_checks }` type in `reticle-agent-api` next to the transcript types. Replay
(`replay` / `verify_replay`) reads only `records` and `final_hash`, so the plan is
replay-neutral; a transcript written before the field existed deserializes with an
empty plan. In the JSONL artifact the plan rides on the existing trailer object as a
`plan` array, so the per-command lines are untouched and a reader that scans only
command lines (or reads `final_hash`) is unaffected.

The harness derives one `PlanStep` per iteration in `reticle-agent`'s `drive(...)`:
the goal is `task {id} (checker {checker})` (the task's public identifiers, never the
prompt), the intended tools are the `op` names of the proposed commands, and the
expected checks are `drc` plus the task's checker. A plan step is narration for the
viewer and material for failure mining, not a binding contract on what the iteration
did.

## Consequences

The plan is auditable and viewable without a new artifact or a schema break: old
transcripts, benchmark fixtures, and the replay-hash tests keep passing, and the
prompt-free-transcript invariant is preserved (the goal is built from the task id and
checker, both already public in the run's artifacts). `PlanStep` is a public type on
the frozen command-surface crate; because it is purely additive it is amended here
per [ADR 0028](0028-v6-subagent-worktree-orchestration.md). The `plan` field being
`#[serde(default)]` means a producer that omits it and a consumer that ignores it
both remain correct, so the plan log can be adopted incrementally.

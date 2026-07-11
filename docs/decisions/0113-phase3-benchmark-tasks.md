# 0113, Phase-3 benchmark tasks: query the read-only API surface directly, port PCell geometry to Rust rather than add a checker dependency, ledger SPICE export

## Context

The v8.2 campaign's bench-2 lane extends the frozen agent benchmark suite (`reticle-bench`,
frozen at v0.6.0 by bench-freeze) with a second wave of tasks exercising Phase 2/3 depth:
PCell params, net-trace queries, SPICE/netlist export, and multi-step edits. A survey of the
current workspace state (as opposed to the Phase 0 grounding in `scratch/campaign/premises.md`,
which predates Phase 2) found:

- `reticle_extract::query` (net-at-point, net-extent, shorts/opens; ADR 0103, F3) and
  `reticle_gen::PCellDef` (schema defaulting, content hashing, range validation; ADR 0107,
  building on the F2 produce-metadata contract of ADR 0102) both exist and are usable, but
  neither is reachable through `AgentCommand`: the 31-variant command vocabulary has no
  PCell-produce or point-query op, and none of the 39 `reticle-mcp` tools cover them either.
- Producing a `PCellDef`'s actual geometry requires `reticle_script::pcell::produce`, the
  sandboxed rhai interpreter. `reticle-bench` depends on `reticle-gen` (already used by the
  `generator` checker) but not on `reticle-script`, and the brief for this lane says new tasks
  must add no new checker dependency.
- No SPICE or netlist writer exists anywhere in the workspace. `docs/src/spice-export.md` is
  an empty stub ("filled by the `netlist` lane") and `commands.rs` carries only a reserved,
  non-dispatched command id. There is nothing yet for a checker to exercise.

## Decision

**Net-trace and PCell tasks are graded by checkers that call the read-only APIs directly,
bypassing `AgentCommand`.** A task's *solution* is still ordinary `add_rect`/`create_cell`
commands (an agent has no other way to build geometry); the *checker* calls
`reticle_extract::net_at_point`/`net_extent` (`crates/reticle-bench/src/net_checkers.rs`) or
`reticle_gen::PCellDef` methods (`crates/reticle-bench/src/pcell_checkers.rs`) against the
resulting document, the way `crate::checkers::IntentCheck` already calls
`reticle_extract::check_intent` directly rather than going through `AgentCommand::CheckIntent`.
This is additive: it does not ask the agent-api or MCP lanes to add new command variants
before the harness can use their read-only surface, and it matches how every other geometric
checker in the crate works (compute, don't dispatch).

**The `pcell_box` checker ports its fixed PCell's geometry to Rust instead of adding a
`reticle-script` dependency.** `PcellBoxPad` builds a real `PCellDef` (`bench.box_pad`) and
resolves parameters through its real methods (`effective_params`, `effective_param_hash`,
`validate_params`), so the schema-defaulting and identity-hashing half of the Phase 2 API is
genuinely exercised. The *geometry* that PCell's script would draw (two concentric squares) is
computed by a Rust closure instead of being produced by `reticle_script::pcell::produce`, and
the module doc says so plainly rather than implying a script ran. This keeps `reticle-bench`'s
dependency set exactly as the brief requires; a future lane that wants a checker proving a real
sandboxed produce is unblocked (add `reticle-script` as a dependency and swap the reference
geometry for a real `produce()` call) without needing to revisit this decision.

**SPICE/netlist export is ledgered, not half-built.** No writer exists to call, so no checker
can exercise one honestly. Building a checker against the committed exchange-format fixture
(`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.spice`) without a
producer to validate would be a checker that can never be proven to accept a real correct
answer, exactly the "checker that always returns true" failure mode the crate's two-way-testing
discipline exists to prevent. The `netlist` lane (per the fixture's own provenance) owns
closing this gap.

## Consequences

Three new checker families ship (`net_checkers`, `pcell_checkers`, plus two multi-step tasks on
existing checkers), each two-way tested and proven solvable through the real
propose-verify-correct loop (`crates/reticle-bench/tests/wave4_tasks.rs`). The suite version
moves 0.6.0 to 0.7.0, adding 7 tasks for 95 total.

If a later lane adds PCell-produce or point-query `AgentCommand` variants, these checkers do
not need to change: they already validate the document state a real produce/query would also
observe, so the *scoring* is unaffected by which path (direct engine call vs. dispatched
command) built it. The Rust-ported PCell geometry is a known, documented gap (not a silent
approximation): if `bench.box_pad`'s script text and the checker's Rust formula ever drift, only
the module doc and this record catch it, since nothing here round-trips them against each other.
No SPICE task exists yet; ledgered above and in `RESULT.md`, not silently dropped.

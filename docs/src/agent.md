# Agent API and harness

Reticle exposes its whole editing engine as a small, serializable command surface,
so a program (or a language model) can build and check layouts through the same
operations a human uses. On top of that surface sits a propose-verify-correct
harness that drives a model against objective checks, writes a replayable
transcript, and can mirror its edits onto the live collaboration document.

## The command surface (`reticle-agent-api`)

[`AgentCommand`](https://docs.rs/reticle-agent-api) is a tagged, serde-serializable
enum of 30 operations over the engine: create a cell, add a rectangle, polygon, or
path, add a label or pin, transform or delete shapes, set the technology, run DRC,
check a connectivity intent, extract nets, compare against a netlist, export GDS or
OASIS, and render a region to PNG. A [`Session`] owns an editable document and a
stable element-id allocator, so a command that adds geometry returns an
[`ElementId`] that later commands and the transcript can refer to even across
deletions (ADR 0018).

Five of those operations lift the in-app editor's productivity actions to the
command surface (ADR 0031): `BooleanCombine` (union, intersection, difference, or xor
over a set of shapes, onto a target layer), `AlignShapes`, `DistributeShapes`,
`OffsetShapes` (grow or shrink), and `BuildViaStack` (a cut plus enclosures sized
from the technology's enclosure rules). They run on the same `reticle-geometry`
primitives as the editor, so agent-built and hand-built geometry match exactly.

Every command applied to a session is recorded as a [`CommandRecord`] in a
[`Transcript`], and the model's document has a [`document_hash`]. Replaying a
transcript reproduces that hash exactly, so a run is deterministic and auditable:
the `verify_replay` path recomputes the hash and rejects a tampered transcript.

## The propose-verify-correct loop (`reticle-agent`)

```mermaid
flowchart LR
    M[Model] -->|AgentCommands| A[Apply to Session]
    A --> V{Verify: DRC + intent}
    V -->|clean| D[Done, write artifacts]
    V -->|violations| F[Feed violations back]
    F --> M
```

The harness asks a [`ModelClient`] for a batch of commands, applies them to a private
session, and verifies the result with the SKY130 DRC subset plus, where a task
carries an intent spec, the connectivity checker. Violations become correcting
context for the next proposal, up to an iteration bound. Success is defined by the
checker, not by the model's say-so, and a failure is recorded as a failure, never
retro-edited to a pass. Each run writes four artifacts: the transcript, the final
GDS, a rendered PNG, and a result record.

The model is either the real `AnthropicModel` (an Anthropic-compatible endpoint; the
API key is read from the environment only and never printed, serialized, or written
to an artifact) or the deterministic `MockModel` used offline and in tests.

## Iterative refinement (mid-session constraints)

A session does not have to be one fixed prompt. A user watching the loop can add a
new constraint *between* iterations ("make the wire wider", "keep it on met1", "shrink
the guard ring") and the loop folds it into the very next proposal without tearing the
run down and starting over. This is the refinement protocol.

```mermaid
flowchart LR
    U[User constraint] -->|between iterations| C[Fold into context]
    V[Checker feedback] --> C
    C --> M[Model]
    M -->|AgentCommands| A[Apply]
    A --> K{Verify}
    K -->|clean and constraint met| D[Done]
    K -->|violations| V
    K -.->|user adds more| U
```

The mechanism is deliberately small. `run_agent_task_refined` takes a
`RefinementSource`: a trait whose one method, `drain(iteration)`, returns the
constraint strings that have arrived since the loop last asked. The loop drains it once
per iteration, just before it asks the model, and accumulates the constraints. Each
iteration it builds the model context from the accumulated user constraints followed by
the checker's own failure reasons, so the model sees both in one place. Because the
constraints accumulate rather than replace, a constraint added once keeps conditioning
every later proposal; because the checker still runs every iteration, success is still
defined by objective verification, not by the model's agreement that it complied.

Three properties make this safe and testable:

- **No restart.** Folding a constraint into the running context is one continuous run:
  the session, its transcript, and its element ids are never rebuilt. The
  `refinement_is_folded_in_and_loop_converges_without_restart` test starts a task, injects
  a widen constraint before the second iteration, and asserts the final layout carries the
  wider wire while the run stays a single loop.
- **Bounded on conflict.** A constraint that cannot be satisfied (it conflicts with the
  task, or asks for something no proposal can reach) does not spin forever: the same
  `max_iterations` cap applies, and the run is recorded as an honest failure. The
  `conflicting_refinement_is_bounded_by_max_iterations_not_infinite` test proves the loop
  stops at the cap rather than looping.
- **Frozen context type.** The refinements ride in the existing
  `Context::feedback` channel, so `reticle_bench`'s `Context` type is unchanged. A
  caller with no mid-session constraints uses `run_agent_task`, which is
  `run_agent_task_refined` with a `NoRefinements` source and behaves exactly as before.

The source is whatever a caller needs: a channel receiver behind a live UI or HTTP
endpoint, or a scripted `RefinementFn` closure in a deterministic test. The loop owns no
channel of its own, so the same code path serves an interactive session and a reproducible
convergence test on the mock model.

## Live collaboration (`reticle-agent::collab`)

An [`AgentCollaborator`] mirrors each agent step onto the `reticle-sync` CRDT under a
distinct actor id, as one atomic transaction per step, so a human peer watching the
room never sees a half-drawn step and can edit alongside the agent (ADR 0022). The
same transcript the harness writes is what the in-app replay theater plays back
through a live session.

## Scoped sessions and context packs (`reticle-agent::context_pack`)

A session can be opened on a *region* of the document rather than the whole thing. This
is what the DRC error browser's "ask the agent to fix this" button does: it hands the
harness a rectangle and the specific violation, and the agent works only that corner.

For a scoped run the harness does not condition the model on the whole-document
snapshot. Instead a [`ContextPack`] assembles a minimal, region-local context string:

1. the scoped region;
2. the shapes whose bounding box overlaps that region, each as kind, layer, and
   bounding box, capped so a dense region still yields a bounded prompt;
3. the violated rule, stated structurally (kind, layers, measured value, required
   value);
4. only the technology rules whose layers appear in those shapes or in the violation.

The overlap test is the same inclusive touch-or-overlap test the `query_shapes` command
uses, so a pack and a `query_shapes` on the same rectangle agree on which shapes are in
scope. The pack is a pure function of the document and the region, so it plugs into the
same context hook the whole-document path uses: a scoped run sets the model's document
context to the pack instead of the summary.

**Why it saves tokens.** The compact per-cell summary the unscoped hook sends is already
small, but it is lossy: it lists how many shapes a cell holds, not where they are. To
reason about a local geometric fix a model needs the coordinates of the nearby shapes,
so the honest whole-document baseline is the full per-shape listing (`whole_document_context`),
not the summary. A pack replaces that full listing with a region-scoped slice.

On a synthetic document of one cell with 200 shapes, a met1 width rule and a met2
spacing rule, and a repair region overlapping exactly one shape, the measured estimates
(characters over four) are:

| Context | Tokens | Characters |
| --- | --- | --- |
| Whole-document, full per-shape fidelity | 1878 | 7509 |
| Scoped pack (one shape + violation + one relevant rule) | 62 | 245 |

That is roughly a 30x reduction, about 97% fewer tokens, and the pack's size tracks the
*region* rather than the design, so the saving grows with the design. The measurement is
pinned by a test in the `context_pack` module.

## Where it runs

- `reticle-mcp` exposes this command surface to a model over the Model Context
  Protocol; see [the MCP chapter](mcp.md).
- `reticle-demo-server` runs the loop behind a rate-limited public endpoint and
  streams each step to a watchable room; see [Deployment](deployment.md).
- The [benchmark suite](benchmarks.md) scores the loop across 63 graded tasks.

See ADRs [0018](https://github.com/AlpharomeroJL/reticle/blob/main/docs/decisions/0018-agent-api-layering-and-element-ids.md),
[0021](https://github.com/AlpharomeroJL/reticle/blob/main/docs/decisions/0021-intent-types-in-extract.md),
and [0022](https://github.com/AlpharomeroJL/reticle/blob/main/docs/decisions/0022-agent-crdt-collaborator-bridge.md).

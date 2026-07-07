# Agent live-collaboration demo

This directory holds the committed artifact for the "agent as a live collaborator"
demo (ADR 0062).

## `agent_drc_fix.transcript.jsonl`

A replay-theater transcript of the Reticle agent fixing a seeded design-rule violation
while sharing a live relay room with humans. The run:

1. installs a small technology whose met1 layer (SKY130 layer 68, datatype 20) carries
   the `m1.2` minimum-spacing rule at its cited SKY130-subset value of **140 DBU**;
2. seeds two met1 rectangles **100 DBU apart** (closer than the rule allows);
3. runs DRC, which **flags** the spacing violation;
4. `transform_shapes` the second rectangle right by 100 DBU to a legal **200 DBU** gap;
5. runs DRC again, which is **clean**.

The file is one `CommandRecord` per line, then a trailer line carrying the `final_hash`
a faithful replay reproduces (plus the plan narration and this provenance note). It loads
in the in-app replay theater's parser and replays to its pinned hash; a determinism test
(`reticle-agent/tests/agent_drc_demo.rs`) pins that hash.

## Provenance and honest labeling

**This is a deterministic scripted run, not a live model.** No LLM, no API key, and no
network beyond the in-process relay socket are involved anywhere in producing it. The
commands are the fixed script in `reticle_agent::live::scripted_drc_fix_steps`; the
"agent" here is the harness driving those commands. The transcript's wall-clock timestamps
are zeroed so the artifact is byte-deterministic (replay reads only the commands and the
final hash, never the timestamps).

Regenerate it with:

```text
cargo run -p reticle-agent --example agent_live_room -- \
    --emit examples/collab/agent_drc_fix.transcript.jsonl
```

To watch the same script run live in a room on a running relay:

```text
cargo run -p reticle-agent --example agent_live_room -- --relay ws://127.0.0.1:8080 --room demo
```

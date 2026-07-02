# 0021, Intent types live in reticle-extract; serde on geometry value types

## Context

The connectivity intent spec (`IntentSpec`) and its structural report
(`IntentReport`) were first frozen in `reticle-agent-api`. But the intent checker
belongs in `reticle-extract`, next to the connectivity extraction it builds on, and
`reticle-agent-api` already depends on `reticle-extract` (for its `run_extract` and
`check_intent` commands). Putting the types in `reticle-agent-api` and the checker in
`reticle-extract` would force `reticle-extract` to depend on `reticle-agent-api`, a
dependency cycle.

## Decision

Move the intent types (`IntentSpec`, `IntentNet`, `Terminal`, `ForbiddenPair`,
`IntentReport`, `Open`, `Short`) into `reticle-extract`, where the checker lives, and
re-export them from `reticle-agent-api` for callers of the command surface. To let the
intent types use real geometry rather than duplicate coordinate structs, derive serde on
the three geometry value types `Point`, `Rect`, and `LayerId`. This narrows the
"geometry stays serde-free" stance of ADR 0018: the small value types are now
serializable; the heavier shape and boolean machinery is not touched.

The command surface still carries an intent as a serialized string in `check_intent`, so
the command enum itself is unchanged, and the argument types (`PointArg`, `RectArg`,
`LayerArg`) stay as the stable JSON wire shape for commands.

## Consequences

- No dependency cycle. `reticle-extract` owns the intent types and the checker, so the
  intent-engine lane works entirely within one crate.
- Three geometry value types gained serde derives (additive, no behavior change);
  everything downstream still builds and its tests pass.
- There is now a small overlap between the command argument types and the serde-enabled
  geometry types. The command args are kept because they are the frozen, tested wire
  contract; the intent types use geometry directly because they are new. A future
  consolidation could drop the arg types, but not mid-run.

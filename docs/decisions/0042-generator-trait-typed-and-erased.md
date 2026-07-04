# 0042, Generator framework: a typed trait plus a type-erased registry path

## Context

Wave 2 adds parameterized layout generators (guard rings, via farms, and, as lanes
2B and 2C follow, more): pure functions from typed parameters plus a technology to
geometry. Two very different callers need to drive them. Lane 2D's Generate panel and
the agent tool surface need to enumerate every generator and its parameters
*generically*, without naming a concrete parameter type at each call site, and to move
parameters as JSON (a filled-in form, or a model's tool-call arguments). A typed caller
(a test, or another crate that knows the concrete generator) instead wants full
compile-time checking on the parameter struct.

A single object-safe trait cannot serve both: an associated `Params` type gives typed
callers their checking but is not object-safe, so it cannot live behind the `dyn` the
registry needs; erasing everything to JSON gives the registry its uniformity but throws
away all typing for direct callers. The parameter schema (field names, types, ranges,
defaults, docs) that 2D turns into a form and a tool schema also has to come from
somewhere stable and model-friendly.

## Decision

Split the contract into two layers in `reticle-gen`. `Generator` is the typed trait a
concrete generator implements: an associated `Params: GenParams` type, `id`/`title`/
`description`, and `generate(&params, &tech, &mut cell) -> Result<GenOutput, GenError>`
that appends shapes to a caller-provided `Cell`. `GenParams` bounds the parameter struct
with `Serialize + DeserializeOwned + Default + Clone + Debug` and adds `schema()` (the
machine-readable `ParamSchema`) and `validate()` (the authoritative range and cross-field
check).

`ErasedGenerator` is the object-safe face the `Registry` stores: the same capabilities
with parameters moved as `serde_json::Value`. A blanket `impl<G: Generator> ErasedGenerator
for G` derives it for every generator, so implementing the typed trait is all a new
generator needs; the erased path (deserialize, then always `validate` before `generate`)
comes for free. The `Registry` maps ids to `Box<dyn ErasedGenerator>` and exposes
`infos()` (enumerate ids/titles/descriptions/schemas), `default_params()`, `validate()`,
and `generate()`. The schema is hand-authored serde data (`ParamSchema` / `FieldSchema` /
`FieldType`) rather than derived from a reflection crate, to keep the dependency set to
geometry, model, drc, and serde, which is what the browser build tolerates.

## Consequences

- Lanes 2B/2C implement one trait (`Generator`) and get registry enumeration, JSON
  invocation, and schema publication automatically. The frozen surface they build on is
  `Generator`, `GenParams`, `GenOutput`, `GenError`, `ParamSchema`/`FieldSchema`/
  `FieldType`, and `Registry`/`GeneratorInfo`.
- Lane 2D drives everything through `Registry` with ids and JSON, and renders the same
  `ParamSchema` twice: a UI form and a model-facing tool schema. Validation is centralized
  in `GenParams::validate`, which the registry runs before every generate, so a form and a
  model see identical rejection behavior.
- The hand-authored schema is a small maintenance cost (a generator lists its fields
  twice, once in the struct and once in `schema()`), accepted to avoid a heavyweight
  reflection dependency and to keep the browser build lean. A generator whose struct and
  schema drift is caught by its own tests, not the type system.
- `Registry` holds trait objects, which are not `Debug`; its `Debug` impl lists ids so it
  stays inspectable in test output and logs.

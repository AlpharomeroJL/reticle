# 0019, Structured DRC violations, enriched in place

## Context

The agent API's `get_violations` must return structured violations: the rule id, the
layer, the location, and the measured value versus the required threshold, so a harness
can reason about a failure and a model can correct it. The `reticle_model::Violation`
type carried only a rule name, a location, and a human-readable message; the measured
and required numbers existed only inside the message string.

Two shapes were possible: a new structured type in `reticle-agent-api` that wraps the
model violation, or enriching the model type in place. The wrapper cannot recover the
measured and required values, because they are computed inside the DRC engine and were
never stored.

## Decision

Enrich `reticle_model::Violation` in place with `kind`, `layer`, `other_layer`,
`measured`, and `required` fields, populated at construction by a new
`Violation::new(rule, measured, location, message)` constructor that copies the rule's
kind, layers, and threshold. Every DRC check site passes the value it already computed
(a width, area, gap, margin, overhang, or density); a sentinel of `i64::MIN` marks a
feature that is absent entirely (a shape with no enclosing layer, or a bbox-only angle
check). The model type does not derive serde: `reticle-agent-api` maps it to a
serde response, which keeps `reticle-geometry` serde-free (see ADR 0018).

This is a breaking change to a core type, done once in the serial Wave 0 with the whole
workspace updated in the same commit, which is exactly why the contract freeze is serial.

## Consequences

- `get_violations` returns real structured data, and a DRC violation now says how far
  off it is, which the propose-verify-correct loop and the SKY130 rule subset both need.
- The DRC engine and the one app test that built a violation literal were updated; the
  property and golden oracles still pass unchanged, because they compare violations by
  location.
- Consumers that only read `rule`, `location`, and `message` are unaffected; the new
  fields are additive to readers.

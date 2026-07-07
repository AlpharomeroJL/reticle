# 0061, v8 frozen-surface amendments: what may change, at which wave boundary, always additive

## Context

The standing frozen-surface manifest in `docs/TASKS.md` makes core contracts read-only
to lanes, with amendments allowed only at wave boundaries by the orchestrator with an
ADR. The v8 roadmap needs a small number of those surfaces to grow (comments persisted
in the document, multi-agent benchmark labeling, a second PDK) and creates new
contracts of its own (streamed-archive format, relay wire invariant, LEF/DEF design
type). Unplanned drift is the failure mode this ADR exists to prevent.

## Decision

All amendments are additive and land only at their stated boundary. reticle-proto:
Wave 1 freezes a tested wire invariant (the SyncMessage first-byte protobuf field tag,
0x0A update / 0x12 presence / 0x1A comment, which the Durable Object relay keys off);
Wave 4 bumps SCHEMA_VERSION to 2, extends Comment with resolved state and an anchor
point, adds Document.comments, and fills the migrate module with a committed V1 golden
fixture. reticle-model gains the matching comments field at Wave 4. reticle-agent-api
may gain additive commands at Wave 3/4 boundaries only if a lane needs them (ADR
0031/0048 precedent). reticle-bench gains optional task fields (hints, reference) and
optional multi-agent labeling fields on ResultRecord at Wave 7, where the leaderboard
record format freezes. Wave 5 adds IHP SG13G2 tech data as new files; the sky130 files
stay frozen. New v8 freezes: the .rtla archive v1 layout, TileSource trait, and
gds_stream event API freeze at the Wave 2 contract step; the relay conformance vector
format at the 1B merge; the LefDefDesign type at the 5A merge (gating 5B dispatch).
reticle-geometry, reticle-extract intent types, and reticle-demo are not amended.

## Consequences

Lanes can rely on every frozen signature in their briefs verbatim; any need outside
this list parks the lane rather than mutating a contract. Each amendment lands as its
own commit at the boundary with tests (wire-invariant test, V1 fixture round-trip), so
a reviewer can audit exactly when and why each surface moved. A future run repeats the
pattern: one amendment ADR up front, executed only at boundaries.

# 0050, The Generate panel: a schema-driven form with a live preview

## Context

Wave 2D exposes the six parameterized generators in the app so a layout engineer can
stamp out a guard ring or a via farm from a few numbers rather than drawing it by
hand. A generator publishes a machine-readable `ParamSchema` (field names, types,
ranges, defaults, docs) precisely so a form can be built from it (ADR 0042). The
generators are pure geometry and compile for wasm, so the panel and its preview must
work in the browser too, and the placement must sit on the app's existing undo history
so one Undo removes the whole generated structure.

## Decision

Put the panel logic in a new module,
[`crate::generate_panel`](../../crates/reticle-app/src/generate_panel.rs), and keep the
`app.rs` change minimal: one `generate` field, its init, a `generate_section` draw call
in the right-column stack, a `draw_generate_preview` canvas overlay next to the array
preview, and a `generate_apply` that commits the placement. The share section, the
Start-screen region, and the browser-open/drop regions of `app.rs` are left untouched
(other lanes' territory).

The form is built straight from the selected generator's `ParamSchema`: an `Int` field
maps to a `DragValue` clamped to the field's `[min, max]`, a `Bool` to a checkbox, and
an `Enum` to a combo box over the variants, with the field doc on hover. The field
values live in a per-generator `serde_json::Value` seeded from the generator's schema
defaults, so the form round-trips through the same JSON parameter path the agent and
MCP surfaces use, and each generator keeps its own values when the selection changes.

The live preview generates the current parameters into a scratch cell and draws the
resulting shapes as a canvas overlay in a distinct accent (empty, not an error, while a
mid-edit value is out of range; the panel surfaces the generator's message as text
below the form). Placement runs the same generation and applies each produced shape as
one `Edit::AddShape` through `History::apply_group`, so the whole structure lands as a
single logical undo step. All the geometry-producing logic in the module is pure (no
egui), so it is unit-tested without a UI context; `app.rs` owns only the thin form
rendering, the overlay, and the commit, which a headless egui pass and an
apply-then-undo test cover.

## Consequences

The Generate panel reuses the array/via-stack panel patterns already in `app.rs` (a
schema combo, drag/checkbox/combo widgets, a live overlay, an `apply_group` commit), so
it reads like the rest of the editing suite and adds no new UI concepts. Because
`reticle-gen` is wasm-safe and the panel's logic is pure, the panel and its live preview
work in the browser and the web build stays green. Adding a seventh generator surfaces a
seventh entry in the panel automatically, since the catalog and every form comes from the
registry's `infos()`. The one cost is that a generator whose parameters do not fit the
three schema field types (a free-form list, like the fill keep-outs) is not editable in
the form; those parameters take their JSON default, which the schema deliberately omits
from its fields (ADR 0046).

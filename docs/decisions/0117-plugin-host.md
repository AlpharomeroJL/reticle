# 0117, the production plugin host: the real query surface, the full v0 edit vocabulary, and the edit-decoder fuzz target

## Context

ADR 0116 built a spike `wasmi` host in `reticle-plugin/src/host.rs` and pinned the
v0 calling convention: the module contract, the four-function host table, the
`stage_edit` funnel through `EditableDocument::apply`, and the little-endian edit
wire format (of which the spike decoded only `AddShape`). It deliberately left three
things to wave B, and ledgered a `cargo-fuzz` target over the decoder to it:

- the read-query surface answered scalar stand-ins (`query_selection` returned a
  caller-supplied count) rather than real, document-grounded data;
- only `AddShape` (`0x01`) of the eight `reticle_model::Edit` variants had a wire
  encoding;
- the decoder's panic-freedom was proven only by in-process adversarial tests, not
  by a formal fuzz target.

This lane implements the pinned contract rather than re-deriving it. The hard
constraints from ADR 0116 hold unchanged: untrusted guest bytes must error and never
panic (a host panic aborts the wasm instance and kills the tab), every count-driven
read is capped against the bytes that remain, and the runtime stays native-only so
the wasm bundle is byte-unaffected (ADR 0115/0116/0098).

## Decision

### The read-query surface answers from the real pre-run snapshot

The host already snapshots the caller's `Document` before any edit applies, so a run
is reproducible regardless of what it stages. All three queries now answer from that
snapshot:

- `query_shapes(cell) -> i32` returns the real shape count of the named cell in the
  snapshot (unchanged from the spike, which was already real).
- `query_technology() -> i32` returns the snapshot technology's real
  `dbu_per_micron` (also already real in the spike).
- `query_selection() -> i32` is the one that was a stand-in. `HostContext` no longer
  carries a precomputed count; it carries the real selection as a `Vec<SelectedShape>`,
  each a `(cell, index)` reference into the document. The host resolves those
  references against the snapshot and returns how many name a shape that actually
  exists, so a stale reference (missing cell or out-of-range index) cannot inflate
  the answer and the count is reproducible exactly like the other two.

`reticle_model` has no selection type (selection is app state, and this lane owns
only `reticle-plugin`), so the selection enters through `HostContext` as model-level
references rather than the app's render-list indices. The v0 ABI is unchanged: all
three still return `i32`. `HostContext` moving from a scalar to real references is a
source change to an input struct that nothing outside `reticle-plugin` constructs
yet; `Host::run`'s signature is untouched, so the sample plugin and the manager UI
that build against it are unaffected.

### The rest of the v0 edit vocabulary, on the same framing and cap discipline

`decode_edit_v0` now decodes one opcode per `Edit` variant, on the little-endian
framing and the `Cursor` cap discipline ADR 0116 pinned. Two sub-encodings recur:

```text
name       := u16 len (capped at Limits::max_query_len) ++ len bytes UTF-8
transform  := u8 orientation (0..8, per Orientation::code)
              i32 tx, i32 ty                (translation, DBU)
              u32 mag_num, u32 mag_den      (magnification; den != 0)
```

```text
0x01 AddShape     : name cell, u16 layer, u16 datatype, i32 x0,y0,x1,y1
0x02 AddCell      : name cell                         (adds an EMPTY cell)
0x03 RemoveCell   : name cell
0x04 RemoveShape  : name cell, u32 index
0x05 AddInstance  : name cell, name child, transform
0x06 AddArray     : name cell, name child, transform,
                    u32 columns, u32 rows, i32 column_pitch, i32 row_pitch
0x07 AddLabel     : name cell, name text, i32 x, i32 y,
                    u16 layer, u16 datatype, u8 anchor (0..5, per Anchor)
0x08 RemoveLabel  : name cell, u32 index
```

Design points:

- Every string length is capped both against `max_name` and against the remaining
  bytes; every fixed field is read through the same `Cursor`, so a truncated,
  oversized, or hostile record returns a structured `EditDecodeError` rather than
  over-reading or over-allocating.
- `AddCell` stages an empty cell; a plugin populates it with subsequent `AddShape` /
  `AddInstance` / `AddArray` / `AddLabel` records. This keeps the record bounded and
  avoids a recursive cell encoding in v0.
- Index and count fields (`RemoveShape`/`RemoveLabel` index, array dimensions) are
  decoded raw and validated by the funnel: `EditableDocument::apply` bounds-checks an
  index and reports a `ModelError` the run records and skips, so a decoded record is
  always a well-formed `Edit` even when it will not apply to a given document. The
  decoder validates only what it must to build a value: orientation must be `0..8`,
  a magnification denominator must be non-zero, and an anchor must be `0..5`, each a
  new `EditDecodeError` variant (`BadOrientation`, `ZeroMagDenominator`, `BadAnchor`).
- Honest v0 limit: `AddShape` carries a rectangle only, as ADR 0116 pinned; the
  `Polygon` and `Path` `ShapeKind`s have no v0 wire encoding. That is a scoping of the
  wire format, not a missing opcode, and is a future wire-format addition under a
  version bump, not a silent one.

### A formal cargo-fuzz target over the decoder

`fuzz/fuzz_targets/plugin_decode_edit.rs` runs `decode_edit_v0` on arbitrary bytes;
it must only ever return an `Edit` or an `EditDecodeError`. A curated 62-input seed
corpus under `fuzz/corpus/plugin_decode_edit/` covers every opcode plus the
adversarial edges (unknown opcodes, truncations, over-cap and non-UTF-8 names, bad
orientation/magnification/anchor). The decoder is native-only, so the target builds
and runs under the native fuzz toolchain (WSL). The long soak goes to the campaign
valley queue; a short confirmation pass runs in-brief. Independently of WSL, the
in-process adversarial unit tests (every truncation of every opcode, and thousands of
byte-soup inputs) re-prove panic-freedom in the normal `just ci`, so the gate holds
the invariant on every platform.

## Consequences

- The plugin host is production for v0: real, reproducible reads and the full edit
  vocabulary funnel through the command/undo machinery, so a plugin's whole effect
  stays one contiguous, undoable, replayable run of undo-stack entries.
- Zero wasm bundle delta. `decode_edit_v0`, the host, and `HostContext` stay
  `cfg(not(target_arch = "wasm32"))`; `cargo tree -p reticle-plugin --target
  wasm32-unknown-unknown` still shows `serde` alone, with `wasmi`, `reticle-model`,
  and `reticle-geometry` absent and `wat` / `serde_json` dev-only.
- The v0 wire format is now fully pinned (ADR 0116 pinned `AddShape`; this pins the
  rest). ABI is still v0 and explicitly unstable until the v8.2.0 tag (ADR 0105), so
  the format and the `HostContext` shape are revisable by an honest version bump.
- Sibling wave-B work (the manager UI / execute wiring in the app) builds on the
  stable `Host::run` and the new `HostContext.selection`; it supplies the real
  selection references and renders the run outcome. That wiring is out of this lane's
  owned paths.

# 0116, the native embedded wasm plugin runtime (wasmi) and the v0 calling convention

## Context

ADR 0105 froze F5: the plugin `Manifest`, ABI v0 (`HostFn` / `Permission`), and the
committed static index, all in `reticle-plugin`. ADR 0115 set the native-first precedent
for the plugin runtime (a general script or plugin runtime is hundreds of KB to megabytes
of wasm; shipping one in the browser bundle is the wrong default for a browser-native
editor whose positioning is a small, fast bundle). Phase 4 wave B (`plugin-host`) has to
build the runtime and the calling convention the manager UI and the sample plugin depend
on. This spike proves a minimal-but-real runtime and pins the calling convention so wave B
builds against a settled spec instead of re-deriving it.

Three constraints bear on the choice:

- Untrusted wasm. A plugin is bytes from outside; a panic in the host kills the tab. Every
  path that touches plugin-controlled input (the binary, guest pointers, the edit payload)
  must error, never panic, and must cap every count against the bytes that remain.
- Browser-frugal bundle. ADR 0098's budget and ADR 0115's precedent: the runtime must not
  enter the wasm bundle.
- ABI v0 is unstable until the v8.2.0 tag (ADR 0105), so the calling convention can be
  pinned now and revised by an honest version bump later.

## Decision

### Runtime: `wasmi` 1.1.0, a pure-Rust interpreter, native-only

The host is `reticle-plugin/src/host.rs`, gated `#[cfg(not(target_arch = "wasm32"))]`.
`wasmi`, plus `reticle-model` and `reticle-geometry` (the funnel target and the geometry a
decoded edit builds), are `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`,
exactly as ADR 0115 scoped `reticle-script` and as `reticle-agent` scopes its live-room
stack.

`wasmi` was evaluated first and chosen over `wasmtime`:

- License-clean: dual Apache-2.0 / MIT.
- Pure Rust: the added native tree is `wasmi` + `wasmi_core` + `wasmi_ir` +
  `wasmi_collections` + `wasmparser` + `string-interner`. No C toolchain, no Cranelift; it
  builds on the existing Windows and wasm toolchains with no new system dependency.
- Sufficient: a v0 plugin is bounded by fuel and produces a staged edit list; there is no
  long-running hot loop that would motivate a JIT. The full proof suite (17 tests) runs in
  0.096 s on the interpreter (`cargo nextest run -p reticle-plugin`).
- `default-features = false` (drops the `wat` feature): the runtime accepts binary wasm
  **only**, shrinking the untrusted-input surface. The `.wat` fixtures are hand-authored
  and compiled to binary wasm by the test harness's `wat` dev-dependency, never by the host.

Honesty note: no `wasmtime` head-to-head benchmark was run, deliberately. Native-only
removes the bundle axis and fuel-bounded plugins remove the throughput axis, so wasmtime's
heavier C/Cranelift tree buys nothing v0 needs. If a future plugin class needs JIT
throughput, that is the trigger to benchmark the two and revisit, with the runtime staying
native-only so the bundle is unaffected either way.

### The v0 calling convention (the spec `plugin-host` implements)

Proven end-to-end by `tests/host_v0.rs` against `tests/fixtures/plugins/*.wat`.

**Module contract (guest -> host).** A plugin is a WebAssembly module in binary form. It
exports its linear memory as `memory` and an entry function named by `Manifest::entry` with
signature `() -> ()`. It imports host functions from the module namespace `reticle`. Only
the functions whose `Permission` the manifest grants are wired into the linker.

**Host-function table (host -> guest imports, namespace `reticle`).** All pointers and
lengths are `i32` indices into the plugin's exported memory. Every guest-supplied region is
bounds-checked through `wasmi`'s `Memory::read`; an invalid region yields a negative error
code, never a host panic or an out-of-bounds read. Each host function requires the
`Permission` that `HostFn::required_permission` maps it to:

| import | signature | permission | returns |
|---|---|---|---|
| `query_shapes` | `(cell_ptr, cell_len) -> i32` | `ReadDocument` | shape count of the named cell; `-1` bad pointer, `-2` bad UTF-8, `-3` no such cell |
| `query_selection` | `() -> i32` | `ReadSelection` | selected-shape count |
| `query_technology` | `() -> i32` | `ReadTechnology` | active technology `dbu_per_micron` |
| `stage_edit` | `(ptr, len) -> i32` | `StageEdit` | `0` ok; `-1` bad pointer, `-2` malformed record, `-3` staging buffer full |

**The edit funnel.** `stage_edit` does not mutate the document. It decodes the payload into
a `reticle_model::Edit` and appends it to a per-run staging buffer. After the entry returns,
the host replays the staged edits onto the caller's `EditableDocument` through
`EditableDocument::apply` (the command and undo machinery). A plugin's whole effect is
therefore one contiguous run of undo-stack entries: undoable and replayable by construction.
The query snapshot is cloned from the document **before** any edit is applied, so a plugin's
queries never observe its own in-progress edits, which is what makes a run reproducible.

**The v0 edit wire format** (the `stage_edit` payload, little-endian; untrusted, so every
count is capped against the remaining byte budget and a short or unknown record errors):

```text
u8  opcode                       0x01 = AddShape
-- AddShape --
u16 cell_name_len                capped at Limits::max_query_len
u8  cell_name[cell_name_len]     UTF-8
u16 layer, u16 datatype
i32 x0, y0, x1, y1               rectangle corners in DBU
```

`AddShape` is the only opcode the spike decodes; `plugin-host` adds the rest of the `Edit`
vocabulary against the same framing and the same `Cursor` cap discipline.

**Limits** (all enforced; `Limits::default` gives the values):

- Fuel: `wasmi` fuel metering; exhaustion is an `OutOfFuel` trap surfaced as `HostError::Trap`.
- Linear memory: a `StoreLimits` cap installed via `Store::limiter`; growth past it is denied
  at run time, trapping the plugin.
- Binary size: the plugin bytes are rejected before compilation past `max_wasm_bytes`.
- Staged edits: `max_staged_edits` bounds the host-side buffer against a hostile plugin.
- Payload / name lengths: `max_edit_len` and `max_query_len`.

**Capability gating (at instantiation).** Before instantiation the host scans
`Module::imports`. A `reticle` import that is unknown, or is not a function, is rejected
(`HostError::UnknownImport`); a known host function whose permission the manifest did not
grant is rejected (`HostError::PermissionDenied`). Only granted functions are then wired
into the linker, so even absent the pre-scan an ungranted import would be an unresolved
import; the pre-scan makes it a precise, testable error before any code runs.

### Browser path: ledgered, not built

Following ADR 0115 exactly. The runtime is desktop-only. The browser build offers browse and
preview of the plugin index (the committed F5 index is a pure-serde artifact that already
builds on wasm) plus an honest in-UI disclaimer that plugins run in the desktop app; it never
presents a live plugin run it cannot perform. This mirrors native-only rhai produce and the
native-only live agent. A browser plugin runtime is revisited only by a measured,
bundle-explicit amendment ADR, never a silent ceiling bump.

## Consequences

- Zero wasm bundle delta. The runtime is native-only, so
  `cargo tree -p reticle-plugin --target wasm32-unknown-unknown` shows the lib's wasm
  dependencies are `serde` alone, unchanged from before this lane; `wasmi`, `wasmtime`,
  `reticle-model`, and `reticle-geometry` are all absent, and `wat` / `serde_json` appear
  only as dev-dependencies (test-only, never linked into the bundle). The interpreter never
  reaches the browser, so the app's bundle is byte-unaffected and ADR 0098's budget is not
  touched.
- `plugin-host` (wave B) extends `host.rs` in place: the full read-query surface over the
  real `EditableDocument` (this spike answers `query_shapes` from a cloned snapshot and
  returns scalar counts for the other reads), the rest of the `Edit` opcodes, and the wiring
  into `commands.rs` / the manager UI. The calling convention above is the contract; wave B
  implements it rather than re-deriving it.
- Panic-freedom is structural, not incidental: the binary size cap, `Module::new`'s own
  validation of malformed bytes, the `Memory::read` bounds check on every guest pointer, and
  the `Cursor` cap discipline in the edit decoder each turn plugin-controlled input into a
  `HostError` or a negative code. The decoder is exercised by an in-process adversarial test
  over every truncation and thousands of byte-soup inputs; a formal `cargo-fuzz` target over
  `decode_edit_v0` is ledgered to wave B (fuzzing runs under WSL, out of this timebox).
- ABI still v0 and unstable (ADR 0105). This spike does not change `ABI_VERSION`; the wire
  format and host-function table it pins are the v0 surface, revisable by an honest version
  bump at or after the v8.2.0 tag.

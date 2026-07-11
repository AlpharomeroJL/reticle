# 0120, the plugin manager panel: browser browse, desktop run

## Context

ADR 0105 froze the F5 contract (`Manifest`, ABI v0, the committed static index).
ADR 0115 set the native-first precedent for Phase 4's plugin work: a general
plugin runtime is hundreds of KB to megabytes of wasm, the wrong default for a
browser-native editor whose positioning is a small, fast bundle. ADR 0116's
`plugin-spike` lane proved this out: `wasmi`, the v0 calling convention, and the
host (`reticle_plugin::Host::run`) all landed native-only
(`cfg(not(target_arch = "wasm32"))`), with the browser path explicitly
"ledgered, not built."

This lane (`plugin-ui`, Phase 4 wave B) builds the manager UI the campaign plan
called for: something a user actually clicks. Two things had to be decided
honestly rather than papered over:

- What does the browser build show, given it can never run a plugin? An empty
  panel is unhelpful; a panel that pretends to run something it cannot is
  dishonest (the plugin-moat claim shape, `reticle-claims`: "sandboxed plugins
  that run in the browser build" is only sayable if the browser host path
  shipped, which ADR 0115/0116 already decided it would not).
- The committed F5 index (`library/plugins/index.json`, the sibling
  `plugin-manifest-index` lane) had not merged when this lane shipped. Building
  against nothing was not an option.

## Decision

### Browser and native both browse; only native runs

`crate::plugin_panel::PluginPanelState` is one cross-platform struct. Browsing
and preview are identical on every build: it parses the F5 index
(`reticle_plugin::{Index, IndexEntry, Manifest}`, pure serde, ADR 0105) and
lists every entry's id, name, version, permissions, and provenance
(`wasm_sha256`, `source`). Installing and running are native-only in EFFECT
(`PluginPanelState::run_selected` is `cfg(not(target_arch = "wasm32"))`), and
the browser build says so in the panel itself
(`crate::plugin_panel::BROWSER_DISCLAIMER`): "Plugins run in the desktop app.
This browser build lists and previews the index only; it never runs a
plugin." This mirrors the native-only rhai producer (ADR 0115) and the native-only
live agent exactly: predicted/browsable information is real everywhere, the
live capability is desktop-only and named as such.

The four F6-reserved ids (`plugin.browse`/`install`/`enable`/`disable`, ADR
0106) moved from `RESERVED_CAMPAIGN_IDS` into `REGISTRY` with real `AppOp`
targets, unchanged labels, no chord:

- `plugin.browse` (`Scope::Global`): reveals the Inspector's Plugins section.
  Identical on every build; there is nothing native-only about opening a panel.
- `plugin.install` (`Scope::NativeOnly`, mirroring `xschem.import_probe`):
  picks a `.wasm` file from disk (`rfd::FileDialog`) and loads it as the
  selected entry's plugin bytes this session. Hidden from the web palette/menu
  because there is nothing to install in a build that can never run one; still
  has a real, compilable web arm (reachable directly, for example from a test),
  matching `xschem_import_probe`'s own precedent.
- `plugin.enable` (`Scope::Global`, mirroring `pcell.regenerate`): natively,
  actually RUNS the selected, installed plugin through
  `reticle_plugin::Host::run` and funnels its staged edits into the real
  document as one undoable step (`History::apply_group`). On the web, it marks
  the entry enabled (a harmless local flag, useful bookkeeping while browsing)
  and shows the disclaimer instead of a run it cannot perform.
- `plugin.disable` (`Scope::Global`): clears the enabled flag, keeping any
  installed bytes so re-enabling does not require reinstalling. Identical on
  every build; there is nothing native-only about a flag flip.

### The scratch-document run, then a real re-apply

`Host::run` needs a raw `&mut EditableDocument`: it replays every staged edit
into it directly through `EditableDocument::apply` as the run proceeds. The
app's real document is wrapped in `crate::history::History`, which keeps its
own undo/redo group-size bookkeeping (`undo_groups`/`redo_groups`) alongside
the raw `EditableDocument` it privately owns; handing the host that private
document directly would apply edits `History` never grouped, desyncing its
bookkeeping from the document's actual undo stack. `history.rs` is out of this
lane's owned paths (only `plugin_panel.rs`/`app.rs`/`commands.rs`/`lib.rs`/
`inspector_layout.rs`/`Cargo.toml` are), so no accessor was added to reach
in either.

Instead, `App::plugin_enable` clones the live document into a throwaway
`EditableDocument`, runs the plugin against that scratch copy, then re-applies
the SAME `RunOutcome::staged` edits into `History` for real through
`History::apply_group`, exactly as a boolean op's multi-edit batch lands: one
undo step, undo/redo-consistent by construction. This means a plugin's edits
are decoded and validated twice (once against the scratch copy, once for
real); for v0's fuel-bounded, single-opcode (`AddShape`) plugins this is cheap,
and it is the honest price of not reaching into a sibling lane's private
field.

### Fixture-first (ledgered)

`library/plugins/index.json` had not merged from `plugin-manifest-index` when
this lane shipped, so `PluginPanelState::new` embeds `reticle-plugin`'s own
contract fixture instead
(`crates/reticle-plugin/tests/fixtures/contracts/f5_index.json`, the same one
`reticle-plugin`'s `tests/f5_manifest.rs` pins), via `include_str!`, the same
pattern `trace_panel.rs`/`waveform_panel.rs` already use for their own F3/F4
contract fixtures. LEDGERED: swap the embedded path to the real committed
index once `plugin-manifest-index` merges; the fixture and the real index
share the exact same `Index`/`Manifest` shape (ADR 0105), so this is a one-line
path change, not a rewrite.

`reticle-plugin` is a PLAIN (not native-only) dependency of `reticle-app`: its
`Index`/`Manifest`/`IndexEntry` types are pure serde and need to build on wasm
for the browse path; only `reticle_plugin::host` (`Host`/`HostContext`/
`Limits`/`RunOutcome`) is itself `cfg(not(target_arch = "wasm32"))` inside
`reticle-plugin` (ADR 0116), so every reference `reticle-app` makes to those
types is separately `cfg`-gated. Not yet hoisted into the root
`[workspace.dependencies]` table (out of this lane's owned-paths scope; a
trivial follow-up path-dependency-to-workspace-shortcut swap).

## Consequences

- Bundle: measured with `just bundle-gate` (a fresh trunk release build),
  before and after this lane on the exact same host, back to back:
  - Before (this lane's base commit, 973cfc0): gz total 4,448,519 bytes,
    +438.9 KiB vs the v8.0 baseline (3,999,044 bytes); +11.1 KiB of headroom
    under the +450 KiB budget, matching the ~11 KiB the brief carried in from
    `embed`'s measurement.
  - After: gz total 4,455,368 bytes, +445.6 KiB vs baseline; PASS, +4.4 KiB of
    headroom left.
  - This lane's own delta: +6,849 bytes gz (+6.7 KiB), NOT the "~zero" the
    brief hoped for. Named honestly rather than rounded away: a
    names-retained `twiggy diff` of the release wasm (before vs after,
    `cargo build -p web --target wasm32-unknown-unknown --release`, matching
    `[profile.release]` so codegen is comparable, only wasm-opt/strip are
    skipped) attributes it almost entirely to
    `PluginPanelState::new` (+10,664 raw bytes, the dominant single symbol):
    the `#[derive(Deserialize)]` visitor machinery LTO-inlines into this one
    call site the first time `Index`/`IndexEntry`/`Manifest`/`Permission` are
    ever parsed in the browser build (`trace_panel`/`waveform_panel` already
    pay for `serde_json`'s shared parsing core; this lane is the first to pay
    for deserializing THESE new F5 types). The rest of the directly
    attributable new code (the four `plugin.*` command-handler stubs, a
    `Manifest` destructor, one field visitor) totals under 1.7 KB raw. The
    remaining raw delta is LTO/codegen-units=1 recompilation jitter spread in
    single- and double-digit byte amounts across ~3,100 pre-existing
    functions (confirmed by the diff: most of the largest-looking rows are
    exact-cancelling `-N`/`+N` pairs of the SAME function under a changed
    hash, not new code), not attributable to this lane's design.
  - Trimming further would mean not parsing the real F5 `Index`/`Manifest`
    types via serde in the browser at all (a hand-rolled lighter parser
    duplicating the F5 contract, which the brief's "pure serde over the
    committed F5 index" phrasing did not ask for and which would itself be a
    new honesty risk: two parsers of one contract can drift). Given the
    measured number stays under budget, this lane shipped it as measured
    rather than trading a real committed-fixture parse for a smaller
    unofficial one. Ledgered: headroom is now tight (+4.4 KiB); the next
    wasm-touching Phase 4 lane should re-measure before assuming space is
    free.
  - `cargo tree -p reticle-app --target wasm32-unknown-unknown` shows no
    `wasmi`/`wasmtime` anywhere in the graph; `reticle-plugin`'s own wasm
    dependency is `serde` alone, unchanged from ADR 0116's own claim.
    `cargo build -p web --target wasm32-unknown-unknown` (`just wasm-build`)
    is green.
- The plugin-moat claim stays reworded to the native capability (per
  `reticle-claims`): nothing in the panel, the ADR, or the status bar claims a
  browser-run plugin. `BROWSER_DISCLAIMER`'s wording is pinned by a
  native-compiled unit test (`plugin_panel::tests::
  browser_disclaimer_is_honest_about_the_desktop_only_boundary`) so the claim
  cannot silently drift.
- No persistence: `installed`/`enabled`/`last_run` are session-only
  (`PluginPanelState` fields with no save/load wiring). A restart starts every
  entry uninstalled and disabled. Ledgered follow-on, along with a plugin
  marketplace, richer preview, and wiring the real current selection count into
  `HostContext` (today `HostContext::default()`, i.e. zero).
- The native run path is proven end to end by a headless test
  (`app::tests::plugin_enable_runs_the_real_host_and_bumps_the_document_revision`):
  a hand-authored v0 plugin (WAT compiled to binary wasm by the `wat` crate,
  test-only, mirroring `reticle-plugin`'s own fixture harness) stages one
  `AddShape` against the demo document's real top cell, and dispatching
  `plugin.enable` bumps `History::revision()` by exactly one and leaves the run
  on the undo stack.
- ABI still v0 and unstable (ADR 0105/0116); this lane implements the manager
  UI against that surface, it does not change it.

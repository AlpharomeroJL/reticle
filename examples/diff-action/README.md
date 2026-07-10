# Reticle layout diff (GitHub Action example)

This directory holds a composite GitHub Action, `action.yml`, that runs
`reticle diff` (the `reticle-cli` `diff` subcommand) against two layout files
and fails the step when they differ. It exists so a pull request touching a
generated or hand-edited layout can be gated on "did the drawn geometry
actually change," the same question `git diff` answers for text.

## What `reticle diff` compares

`reticle diff <before> <after>` loads both files with the same GDSII/OASIS
importers `reticle import`/`reticle export` use (`.gds`/`.gdsii` is read as
GDSII, any other extension as the in-house OASIS subset; the two sides do not
need to share a format), flattens each file's top-cell hierarchy, and runs the
`reticle-diff` crate's pure geometric diff over the result: shapes are matched
by `(layer, exact geometry)`, so a shape that merely moved shows as one
removed plus one added, not "changed" (the `changed` field exists in
`reticle-diff`'s output but is always empty in this version; see
`docs/src/layout-diff.md` in the main repo). It prints:

```text
before: <path> (top: <cell>)
after:  <path> (top: <cell>)
added:   <n>
removed: <n>
changed: <n>
by layer:
  <layer>/<datatype>: +<added> -<removed>
```

The `by layer:` section is omitted when there is nothing to report.

## Exit-code contract

- **`0`** when `before` and `after` are geometrically identical (nothing
  added, removed, or changed).
- **non-zero** (`1` in every run we've observed, from
  `std::process::ExitCode::FAILURE`) when they differ in any way.
- A read/parse failure on either file (missing file, corrupt stream, a
  document with no cells) is also a non-zero exit, with the reason on
  stderr: this is a hard error, not "treated as a difference."

A workflow step that runs `reticle diff` therefore fails exactly when the
two layouts differ, which is what lets `action.yml` act as a PR gate.

## Usage

### As a composite action, from another repository

```yaml
name: Layout diff gate
on: pull_request

jobs:
  layout-diff:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Compare the tile layout against main
        uses: AlpharomeroJL/reticle/examples/diff-action@main
        with:
          before: golden/tile.gds
          after: build/tile.gds
          # Optional: pin the reticle build to a tag instead of `main`.
          # reticle-ref: v8.1.0
```

`before` and `after` are paths relative to the calling workflow's own
checkout (`$GITHUB_WORKSPACE`); the action checks out `reticle` itself into a
separate directory, builds it there, and runs the diff against your files.

### Directly, without the composite action

If your workflow already has a Rust toolchain and a checkout of this repo
(for example a workflow that lives inside `reticle` itself), skip the action
and call the CLI straight from a `run:` step:

```yaml
- name: Compare the tile layout against main
  run: |
    cargo run --release -p reticle-cli --bin reticle -- \
      diff golden/tile.gds build/tile.gds
```

The exit-code contract is identical either way; `action.yml` is a convenience
wrapper around exactly this command for consumers who don't already have the
`reticle` source checked out.

## The worked example

`example/before.gds` and `example/after.gds` are two tiny, real GDSII files
(178 and 242 bytes) generated from the same fixtures the automated tests use
(`crates/reticle-cli/tests/diff.rs`, `write_example_fixtures`, an `#[ignore]`d
test you can rerun after changing the example geometry; see that file for
the exact command). Both declare one top cell, `top`, on SKY130 met1
(layer 68, datatype 20):

- `before.gds`: one 2 x 2 um rectangle at the origin (spanning x = 0..2 um).
- `after.gds`: the same rectangle, plus a second, disjoint 2 x 2 um rectangle
  whose left edge sits at x = 3 um (a 1 um gap from the first rectangle's
  right edge).

Running the diff against them:

```console
$ reticle diff examples/diff-action/example/before.gds examples/diff-action/example/after.gds
before: examples/diff-action/example/before.gds (top: top)
after:  examples/diff-action/example/after.gds (top: top)
added:   1
removed: 0
changed: 0
by layer:
  68/20: +1 -0
$ echo $?
1
```

Diffing `before.gds` against itself reports all-zero counts, no `by layer:`
section, and exits `0`.

## Honest limits

- **No published `reticle-cli` binary exists yet.** `action.yml` always
  builds the CLI from source (`cargo run --release -p reticle-cli`). On a
  cold cache that is a real Rust compile of the whole `reticle-cli`
  dependency tree, not a download: expect it to be the slowest step in your
  job the first time. The action includes a `Swatinem/rust-cache` step so
  repeat runs only rebuild what changed; without any caching in your own
  workflow, every run pays the cold-compile cost.
- **`changed` is always `0`.** `reticle-diff` v1 never distinguishes a
  moved/resized shape from an independent remove-plus-add (see
  `docs/src/layout-diff.md`); the diff this action prints inherits that
  limit.
- **The comparison is geometric, not semantic.** It has no notion of nets,
  devices, or intent: two layouts that are electrically identical but drawn
  with different tiling or shape splitting will show as differences.
- **This action is an example, not a published/versioned Marketplace
  action.** It is exercised by this repo's own test suite
  (`crates/reticle-cli/tests/diff.rs`) via the built binary, and `action.yml`
  is validated as YAML, but the composite action itself (the checkout-and-build
  flow) has not been run inside actual GitHub Actions infrastructure as part
  of shipping this example: there is no GitHub Actions workflow in this repo
  to run it from (see the repo root docs: CI here runs locally via `just ci`).
  Treat `action.yml` as a documented, structurally valid starting point, and
  verify it end-to-end in your own workflow before relying on it.

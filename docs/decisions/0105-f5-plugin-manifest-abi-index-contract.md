# 0105, F5: the plugin manifest, ABI v0, and static index contract

## Context

The v8.2 campaign's plugin surface (Phase 4) spans several lanes: the wasm host, the
manifest parser, a deterministic committed index, a sample plugin, and the manager UI. The
manager UI must build before the host exists (fixture-first), and the host and the index
generator must agree on one manifest shape or they drift. The manifest is parsed from
plugin-provided bytes, so it is untrusted input.

Two campaign-wide rules bear directly on this contract. Untrusted input: every parser caps
count- and length-bearing fields and errors rather than panicking (a wasm panic kills the
tab). And no ABI stability promise: the plugin ABI is v0 and explicitly unstable until the
v8.2.0 tag, so a post-campaign break must be an honest version bump.

## Decision

F5 lives in `reticle-plugin` (`manifest.rs`). `Manifest { id, version, api_version, name,
entry, permissions }` names a plugin, the ABI it targets, and the capabilities it requests.
`ABI_VERSION` is `0`; `Manifest::abi_compatible` requires `api_version == ABI_VERSION`.
`Manifest::validate` enforces the untrusted-input caps (`MAX_ID_LEN`, `MAX_NAME_LEN`,
`MAX_ENTRY_LEN`, `MAX_PERMISSIONS`) and returns a structured `ManifestError`, never a
panic. The v0 host-function table is `HostFn { QueryShapes, QuerySelection,
QueryTechnology, StageEdit }`, and `HostFn::required_permission` maps each to the
`Permission` a caller must have been granted; edits funnel only through `StageEdit`, so a
plugin's effect goes through the command and undo machinery and is replayable and undoable
by construction. The static index is `Index { entries }`, deterministically ordered by
plugin id with a 64-char lowercase-hex `wasm_sha256` per entry; `Index::validate` rejects a
bad hash, a duplicate id, or an unsorted file, so the committed index hashes and diffs
stably (the leaderboard pattern: the record is the API, no server, no accounts).

The fixture is `crates/reticle-plugin/tests/fixtures/contracts/f5_index.json` (one sample
manifest + index entry); the cross-test (`tests/f5_manifest.rs`) validates it, checks the
host-table mapping, and confirms hostile manifests (wrong ABI, overlong fields, bad hash,
unsorted) are rejected without a panic.

## Consequences

The manager UI and the sample-plugin lanes build against a frozen shape now. Because the
ABI is v0 and the manifest carries `api_version`, the Phase 4 host can reject an
incompatible plugin honestly, and any post-tag ABI change is a visible version bump rather
than a silent break. The caps mean a malformed or hostile manifest is a clean error, not a
crash, which the wasm host relies on.

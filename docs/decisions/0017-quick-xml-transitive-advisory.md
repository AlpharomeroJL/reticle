# 0017, Ignore the quick-xml 0.39 advisories (RUSTSEC-2026-0194/0195) as unreachable, upstream-pinned

## Context

Two companion advisories were published against `quick-xml` before 0.41, both
denial-of-service bugs on untrusted XML: RUSTSEC-2026-0195 (a memory-exhaustion DoS,
where `NsReader` calls `NamespaceResolver::push` for every start tag and appends a
binding per `xmlns` declaration with no upper bound) and RUSTSEC-2026-0194 (quadratic
run time when checking a start tag for duplicate attribute names). The `just ci`
advisory gate (`cargo deny check`) began failing on both once they were published.

The vulnerable crate is `quick-xml 0.39.4`, and it enters the tree only transitively
through the Linux desktop accessibility stack: `eframe 0.35 -> winit 0.30.13 ->
smithay-client-toolkit -> wayland-scanner 0.31.10`, plus the `zbus_xml -> atspi` chain.
Reticle itself never parses XML with `quick-xml`; its own IO is GDSII (`gds21`) and an
in-house OASIS codec. None of the wayland or atspi crates compile on the targets Reticle
builds and ships (Windows native and `wasm32-unknown-unknown`); they are gated to
`cfg(target_os = "linux")` inside `winit`.

The clean fix is `quick-xml >= 0.41`, but `wayland-scanner 0.31.10` pins `quick-xml =
"^0.39"`, and that requirement is locked all the way up through `winit 0.30.13` to
`eframe 0.35.0`. `cargo update -p quick-xml --precise 0.41.0` fails the version
selection. Reaching 0.41 would require bumping the whole winit and smithay chain, which
is not compatible with the pinned `eframe 0.35` the application is built on.

## Decision

Add both `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195` to the `deny.toml` advisory ignore
list with a shared reason and a revisit trigger, and tighten the surrounding policy
comment. The policy is no longer "vulnerabilities are never ignored"; it is that an
advisory is ignored only when we cannot act on it (the crate is pinned by a transitive
dependency we do not control) and either it is an "unmaintained" notice or its vulnerable
code path is not reachable on any target we build or ship. Both quick-xml advisories
qualify on the second branch: not reachable on Windows or wasm, and no untrusted-XML path
through them in Reticle.

The revisit trigger is an `eframe` update that moves its `winit` and
`smithay-client-toolkit` chain onto `quick-xml >= 0.41`; at that point the ignore is
removed and the lock is bumped.

## Consequences

- `just ci` is green again without weakening the gate for advisories we can actually
  act on: the ignore is scoped to one advisory id, justified, and time-boxed by its
  revisit trigger rather than blanket-disabling the vulnerability class.
- The stated advisory policy is now precise about when a vulnerability (not just an
  unmaintained notice) may be ignored, so a future reviewer can tell a reachable
  vulnerability that must block the gate from an unreachable, upstream-pinned one.
- If a later lane introduces a first-party XML path that uses `quick-xml` (none is
  planned; the demo server and MCP transport are JSON), this ignore must be
  re-evaluated, because the "not reachable" premise would no longer hold.

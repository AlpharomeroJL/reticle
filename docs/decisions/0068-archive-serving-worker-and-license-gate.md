# 0068, the archive-serving Worker and the redistribution license gate

## Context

The `.rtla` streamed-archive format (ADR 0062) exists so a browser can fetch one
tile of renderable silicon with a single HTTP Range request over a byte-contiguous
slice of the archive. To make that real, the archives have to live somewhere the
browser bundle (served from the Pages origin `https://alpharomerojl.github.io`) can
range-fetch them, and the content that gets staged there has to be redistributable
in the first place. Two disjoint pieces are needed, neither of which depends on the
converter that builds a `.rtla` (lane 2A owns that): a serving Worker and a staging
gate.

Two constraints shape the Worker. The archives sit in a private R2 bucket
(`reticle-archives`), and R2's public `r2.dev` URL is rate limited, so the bytes must
be served through a Worker with an R2 binding, not the public URL. And the `Range`
header is attacker controlled: a serving path that trusts it can be driven to read
past an object or to allocate an absurd length.

## Decision

Ship an archive Worker under `worker/archive/`, separate from the relay's Durable
Object in `worker/` (they share nothing and version independently), as a plain
ES-module Worker (no wasm build step; the relay's workers-rs toolchain in ADR 0064
buys nothing for a stateless range proxy). It binds R2 as `ARCHIVES` and:

- answers `Range: bytes=a-b` with `206` + `Content-Range` and exactly those bytes; a
  no-Range request returns the whole object as `200`; `Accept-Ranges: bytes` always;
- puts the Cache API in front, keyed by object key plus range so each byte range is
  its own entry. The Cache API refuses a `206`/`Content-Range` response, so a ranged
  body is cached as a normalized `200` carrying the real status and range in internal
  headers and reconstructed on a hit;
- locks CORS to the Pages origin (never `*`) and answers the `OPTIONS` preflight;
- treats the `Range` header as untrusted. Parsing and clamping live in a pure
  `range.js` that is the trust boundary: it resolves every input to a bounded read or
  a `416` (with `Content-Range: bytes */size`), clamps every satisfiable range to
  `[0, size)`, and refuses malformed, backwards, overflowing, multi-range, and
  non-`bytes` requests. RFC 7233 permits ignoring a bad Range and serving `200`; this
  Worker deliberately answers `416` instead, so a probing client gets a bounded
  signal rather than a silent full transfer.

The parser is covered by a `node --test` unit suite (the hermetic half of the gate);
a `wrangler dev` local ranged fetch is the integration check; `wrangler deploy` is the
ship step. The Worker is deployed to workers.dev at
`https://reticle-archive.josefdean.workers.dev`.

For staging, add `xtask verify-licenses <dir>`. For every `*.rtla` archive in a staged
directory it reads a sibling NOTICE manifest (`<archive>.rtla.NOTICE`: a `Source:` URL
and an `SPDX-License-Identifier:`, the provenance style of
`corpus/tinytapeout/NOTICE.md`) and verifies the SPDX license against a small
redistribution allowlist: `Apache-2.0`, `MIT`, `CC-BY-4.0`, the CERN-OHL family (any
variant), and the public-domain dedications (`CC0-1.0`, `Unlicense`, `public-domain`).
The gate fails closed: an archive ships only when its license is positively verified.
Anything else is EXCLUDED with a printed `STATUS EXCLUDED` line naming the reason (no
manifest, no SPDX line, a compound SPDX expression it will not guess at, or a license
off the allowlist), and a run that excludes anything exits non-zero.

## Consequences

The Worker never parses a `.rtla`; it proxies ranges, so it is decoupled from the
archive layout and from lane 2A's builder. Serving through the binding keeps the
archives off the rate-limited public URL and behind one CORS origin. The Range trust
boundary is one small pure function, unit-tested against the adversarial cases, so the
security-critical logic is checked without a network or a live bucket.

The license gate is conservative by construction. Because it fails closed, a real
redistributable archive with a missing or malformed manifest is excluded until its
NOTICE is fixed; that is the intended bias for content going to the open web. Compound
SPDX expressions (`A OR B`, `A WITH ...`) are not evaluated and are excluded rather
than guessed; single-identifier manifests are the supported shape today, and richer
expression parsing is a documented follow-up. The converter that produces `.rtla`
archives is out of scope here (lane 2A); this lane delivers only the hosting and the
staging gate around it.

# Lane v8-2d-alpha-worker RESULT

Status: GREEN. Both deliverables complete; all gates pass. Parks green for the Wave 2
gate. The converter CLI (gds -> .rtla) was out of scope and was not touched.

## Commits (lane/v8-2d-alpha-worker)

| sha | summary |
| --- | --- |
| `cdedb7be7ef368c3054844e2a433f09b9220d3ea` | feat(worker): archive-serving worker with R2 range, Cache API, CORS lock |
| `91d5f47e6b38b9ae1df2f7f37f74ec8847db8238` | feat(xtask): verify-licenses redistribution gate |
| `84765481eba058de05837badd98bd237d8657aff` | docs(archive-hosting): CORS/Range/Cache design, license-gate policy, ADR 0068 |

## Deliverable 1: archive-serving Worker (`worker/archive/`)

An ES-module Worker (not workers-rs; a stateless range proxy needs no wasm toolchain,
and it is fully separate from the relay's `worker/wrangler.toml`, which was read only).

- R2 binding `ARCHIVES` -> `reticle-archives`; served through the binding, never the
  rate-limited public `r2.dev` URL.
- Ranged GET -> `206` + `Content-Range` + exactly those bytes; no-Range -> whole
  object `200`; `Accept-Ranges: bytes` always.
- Cache API in front, keyed by object key plus range. Because the Cache API refuses a
  `206`/`Content-Range` response, a ranged body is cached as a normalized `200`
  carrying the real status and range in internal headers, reconstructed on a hit.
- CORS locked to `https://alpharomerojl.github.io` (never `*`); `OPTIONS` -> `204`.
- Untrusted `Range` header parsed and clamped in a pure `src/range.js`: malformed,
  backwards, overflowing, multi-range, and non-`bytes` all -> `416` with
  `Content-Range: bytes */size`; satisfiable ranges clamped to `[0, size)`, so R2 is
  never read past the object and never asked for an absurd length.

**Deployed:** `https://reticle-archive.josefdean.workers.dev`
(account `Josefdean@protonmail.com`, `86e3e2cfeb39c385931af8bb9e1934e6`; version
`cddae1dc-7c12-4ada-a662-237f56ea2f87`).

### Worker gate (hermetic)

- `node --test` on `test/range.test.mjs`: 14/14 pass (the Range trust boundary).
- `wrangler dev --local` ranged-fetch check against a seeded 100-byte object: 18/18
  assertions pass (full `200`, ranged `206` + correct `Content-Range` + correct body,
  suffix and open-ended ranges, `416` on overflow/backwards/garbage with
  `bytes */100`, `OPTIONS` `204` + CORS, missing key `404`).
- `wrangler deploy --dry-run`: builds, 6.25 KiB, R2 binding resolved.
- Live check against the deployed URL (a temp object put to remote R2, then deleted):
  full `200` (100 bytes), `Range: bytes=10-19` -> `206` `bytes 10-19/100` body
  `0123456789`, `OPTIONS` -> `204`. (First live request right after deploy returned a
  transient Cloudflare edge error; correct on retry ~3s later, i.e. deploy
  propagation, not a Worker fault.)

## Deliverable 2: `xtask verify-licenses <dir>`

New `xtask/src/verify_licenses.rs` plus one additive match arm in `xtask/src/main.rs`.
For each `*.rtla` in the staged dir it reads the sibling `<archive>.rtla.NOTICE`
manifest (`Source:` URL + `SPDX-License-Identifier:`), verifies the SPDX license
against the redistribution allowlist (Apache-2.0, MIT, CC-BY-4.0, the CERN-OHL family,
public-domain dedications), and prints a `STATUS VERIFIED`/`STATUS EXCLUDED` line per
archive. Fails closed: any archive that cannot be verified is excluded (no manifest,
no SPDX line, compound SPDX expression, or license off the allowlist), and a run with
any exclusion exits non-zero.

### License-gate test (two-way, over committed fixtures)

`xtask/tests/fixtures/staged/` pins one verdict per fixture; a single run is the
two-way gate. Real-binary output:

```
STATUS VERIFIED cc0.rtla (CC0-1.0) [https://example.org/public]
STATUS VERIFIED ccby.rtla (CC-BY-4.0) [https://example.org/art]
STATUS VERIFIED cern.rtla (CERN-OHL-S-2.0) [https://ohwr.org/project/example]
STATUS EXCLUDED forbidden.rtla: license not on redistribution allowlist: LicenseRef-Proprietary
STATUS VERIFIED good.rtla (Apache-2.0) [https://github.com/TinyTapeout/tinytapeout-03]
STATUS VERIFIED mit.rtla (MIT) [https://example.org/cell-lib]
STATUS EXCLUDED nomanifest.rtla: no license manifest (expected nomanifest.rtla.NOTICE)
STATUS EXCLUDED nospdx.rtla: manifest has no SPDX-License-Identifier line
STATUS EXCLUDED unknown.rtla: license not on redistribution allowlist: NoSuchLicense-9.9
verify-licenses: 9 archive(s) in ...: 5 verified, 4 excluded
```

Exit code `1` (exclusions present). Eight unit tests in the module cover the allowlist
(accept and reject families), SPDX/source extraction, compound fail-closed, and each
exclusion reason.

## Gates (all green)

- `cargo clippy -p xtask --all-targets -- -D warnings`: clean.
- `cargo nextest run -p xtask`: 12 tests run, 12 passed (8 new license-gate tests).
- `worker/archive`: `wrangler deploy --dry-run` builds; `wrangler dev` ranged fetch
  18/18; `node --test` 14/14.
- `powershell -File scripts/check-style.ps1`: OK (no em-dashes, no banned words).
- `just lint` (fmt-check + full-workspace clippy, the pre-commit hook): green on each
  commit.

## Doc

`docs/src/archive-hosting.md` (Archive hosting subsection under Deployment, wired into
`SUMMARY.md`) states the Range/Cache/CORS design and the fail-closed license-gate
policy. ADR `docs/decisions/0068-archive-serving-worker-and-license-gate.md` records
both, with a README row.

## Honest gaps

- The Cache API is a no-op in `wrangler dev` local mode (always a miss), so the
  cache-hit reconstruct path (`toCacheable`/`fromCache`) was validated by its logic and
  by the range unit tests, not by observing a warm-cache HIT locally; it is exercised
  in production. The live check made a single fetch and so did not observe a HIT
  either.
- Compound SPDX expressions (`A OR B`, `A WITH ...`) are intentionally not evaluated:
  the gate excludes them rather than guess redistribution rights. Single-identifier
  manifests are the supported shape; richer expression parsing is a documented
  follow-up (ADR 0068).
- `verify-licenses` reports and fails; it does not physically relocate excluded bytes
  out of the directory. "Exclude" means the archive is dropped from the verified set
  and the run fails, which is the intended staging-gate behavior.
- `.rtla` fixtures are opaque placeholders (the gate never parses an archive). Real
  archive bytes come from lane 2A's builder, which is out of scope here.
- Deploy is a live workers.dev ship; the deployed URL is recorded above, but the
  Worker is not wired into the Pages app in this lane.

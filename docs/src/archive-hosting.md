# Archive hosting

A `.rtla` archive (the streamed-archive format, ADR 0062) is a network transport for
renderable silicon: a header, a tile directory, and byte-contiguous tiles, so a
browser fetches exactly one tile with a single HTTP Range request. Two pieces of
infrastructure put those archives on the open web safely: a Cloudflare Worker that
serves the bytes, and an `xtask` gate that decides which archives are allowed to be
staged for hosting in the first place.

## The serving Worker

`worker/archive/` is a Cloudflare Worker with an R2 binding to the private bucket
`reticle-archives`. It is separate from the collaboration relay in `worker/` (a
Durable Object); the two share nothing and deploy independently.

- **Served through the binding, never the public URL.** The bucket is reached only
  through the Worker's R2 binding, never the rate-limited public `r2.dev` URL, so the
  archives stay off the rate-limited path and behind the Worker's CORS lock.
- **Range.** A `Range: bytes=a-b` request is answered `206 Partial Content` with a
  `Content-Range` header and exactly those bytes; a request with no `Range` returns
  the whole object as `200`. `Accept-Ranges: bytes` is always advertised. This is the
  point of the format: one tile is one Range request over its `[offset, offset+len)`
  slice of the archive.
- **Cache.** The Cache API sits in front, keyed by object key plus requested range,
  so each distinct byte range is cached as its own entry. Because the Cache API will
  not store a `206` or a `Content-Range` response, a ranged body is cached as a
  normalized `200` that records the real status and range in internal headers, then
  reconstructed on a hit.
- **CORS.** `Access-Control-Allow-Origin` is locked to the Pages origin
  `https://alpharomerojl.github.io` (never `*`), and an `OPTIONS` preflight is
  answered `204` with the allowed methods and headers.

### The Range header is untrusted

The `Range` header is attacker controlled, so the parser (`worker/archive/src/range.js`)
is the trust boundary between the network and the R2 read. It resolves every input to
one of three verdicts: serve the whole object, serve a bounded `[offset, offset+len)`
slice already clamped to the object size, or reject. A malformed, backwards, or
overflowing range is answered `416 Range Not Satisfiable` with
`Content-Range: bytes */size`, never a silent full transfer and never an out-of-range
read. Every satisfiable range is clamped to the object, so the Worker can never be
driven to read past the object or to request an absurd length. The parser is covered
by a `node --test` unit suite; a `wrangler dev` local ranged fetch is the hermetic
integration check (see `worker/archive/README.md`).

## The redistribution license gate

Before an archive is staged for hosting, `xtask verify-licenses <dir>` decides whether
it may be redistributed at all. For every `*.rtla` archive in a staged content
directory it reads a sibling NOTICE manifest (`<archive>.rtla.NOTICE`, the provenance
style of `corpus/tinytapeout/NOTICE.md`: a `Source:` URL and an
`SPDX-License-Identifier:`), and it verifies the SPDX license is on a small
redistribution allowlist:

- `Apache-2.0`, `MIT`, `CC-BY-4.0`
- the CERN Open Hardware Licence family (`CERN-OHL-S`, `-W`, `-P`, any variant)
- public-domain dedications (`CC0-1.0`, `Unlicense`, `public-domain`)

The gate **fails closed**: an archive ships only when its license is positively
verified. Any archive whose terms cannot be verified is EXCLUDED, with a printed
`STATUS EXCLUDED` line naming the reason: no manifest, no SPDX line, a compound SPDX
expression the gate will not guess at, or a license not on the allowlist. A run that
excludes anything exits non-zero, so a staging step that would ship unverifiable
content fails loudly:

```text
STATUS VERIFIED good.rtla (Apache-2.0) [https://github.com/TinyTapeout/tinytapeout-03]
STATUS EXCLUDED mystery.rtla: license not on redistribution allowlist: NoSuchLicense-9.9
STATUS EXCLUDED orphan.rtla: no license manifest (expected orphan.rtla.NOTICE)
verify-licenses: 3 archive(s): 1 verified, 2 excluded
```

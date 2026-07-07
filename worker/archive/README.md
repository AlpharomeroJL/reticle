# reticle-archive worker

A Cloudflare Worker that serves `.rtla` archives from the private R2 bucket
`reticle-archives` over HTTP Range, with the Cache API in front and CORS locked to
the Pages origin `https://alpharomerojl.github.io`.

This is a separate Worker from the collaboration relay in `worker/` (a Durable
Object). The two share nothing and deploy independently; this one has its own
`wrangler.toml` here.

## What it does

- **Range serving.** A `.rtla` archive is a header, a tile directory, and byte
  contiguous tiles (`crates/reticle-index/src/archive.rs`), so a viewer fetches one
  tile with a single `Range: bytes=offset-end` request. The Worker answers `206`
  with `Content-Range`; a request with no `Range` returns the whole object as `200`.
  It never parses the archive, it proxies byte ranges.
- **Served through the R2 binding**, never the public `r2.dev` URL, so the archives
  stay off the rate-limited path and behind the CORS lock.
- **Cache API in front.** Each `(object key, range)` pair is cached as its own
  entry. Because the Cache API refuses a `206`/`Content-Range` response, a ranged
  body is stored as a normalized `200` with the real status and range in internal
  headers, then reconstructed on a hit.
- **CORS locked** to `https://alpharomerojl.github.io` (never `*`), with an
  `OPTIONS` preflight.
- **Untrusted `Range`.** The header is attacker controlled. `src/range.js` resolves
  every input to a bounded read or a `416`: a malformed, backwards, or overflowing
  range is answered `416` with `Content-Range: bytes */size`, and every satisfiable
  range is clamped to `[0, size)`, so an R2 read can never run past the object and
  the Worker cannot be driven to allocate an absurd length.

## Gate

Two levels, both hermetic (no network):

1. **Unit** (the Range trust boundary):

   ```
   npm test           # node --test, runs test/range.test.mjs
   ```

2. **Integration** (a real ranged fetch through `wrangler dev` local mode):

   ```
   # seed the local R2 store with a 100-byte object
   node -e 'const b=Buffer.alloc(100);for(let i=0;i<100;i++)b[i]=48+(i%10);process.stdout.write(b)' > test.rtla
   npx wrangler r2 object put reticle-archives/test.rtla --file=test.rtla --local
   npx wrangler dev --port 8799 --local
   # then, in another shell:
   curl -i -H 'Range: bytes=0-9' http://127.0.0.1:8799/test.rtla   # -> 206, Content-Range: bytes 0-9/100
   curl -i -H 'Range: bytes=200-300' http://127.0.0.1:8799/test.rtla # -> 416, Content-Range: bytes */100
   curl -i -X OPTIONS http://127.0.0.1:8799/test.rtla               # -> 204 + CORS
   ```

A build-only check is `npx wrangler deploy --dry-run`.

## Deploy

```
npx wrangler deploy
```

Requires an authenticated `wrangler whoami`. The bucket `reticle-archives` already
exists in the account.

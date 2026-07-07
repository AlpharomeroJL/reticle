// The archive-serving Worker.
//
// It streams `.rtla` archives out of the private R2 bucket `reticle-archives` over
// HTTP Range, with the Cache API in front and CORS locked to the Pages origin. It
// never parses the archive: the `.rtla` layout is header + directory + byte-
// contiguous tiles (see crates/reticle-index/src/archive.rs), so a viewer fetches
// exactly one tile with a single `Range: bytes=offset-end` request and this Worker
// just proxies those bytes out of R2. That is why the bucket is served through a
// Worker binding and never through the rate-limited public `r2.dev` URL.
//
// Untrusted input: the `Range` header is attacker controlled. Parsing and clamping
// live in ./range.js, which resolves every input to a bounded read or a 416; this
// file only turns those verdicts into responses and can never issue an R2 read past
// the object.

import { parseRange } from './range.js';

// The GitHub Pages origin that serves the browser bundle. CORS is locked to this
// single origin (never `*`) so the archives are only readable from the app.
const ALLOWED_ORIGIN = 'https://alpharomerojl.github.io';

// Headers that mark the cached body's real HTTP shape. The Cache API refuses to
// store a 206 response or one carrying `Content-Range`, so a ranged body is stored
// as a plain 200 and these headers carry the status and Content-Range across, to be
// reconstructed on a cache hit. They are internal and stripped before the client
// ever sees them.
const CACHE_STATUS_HEADER = 'x-rtla-cache-status';
const CACHE_RANGE_HEADER = 'x-rtla-cache-content-range';

function corsHeaders() {
  return {
    'Access-Control-Allow-Origin': ALLOWED_ORIGIN,
    'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS',
    'Access-Control-Allow-Headers': 'Range',
    'Access-Control-Expose-Headers': 'Content-Range, Content-Length, Accept-Ranges, ETag',
    // The response varies by request origin and by the requested byte range.
    Vary: 'Origin, Range',
  };
}

function errorResponse(status, message, extraHeaders) {
  return new Response(`${message}\n`, {
    status,
    headers: { 'Content-Type': 'text/plain; charset=utf-8', ...corsHeaders(), ...extraHeaders },
  });
}

export default {
  /**
   * @param {Request} request
   * @param {{ ARCHIVES: R2Bucket }} env
   * @param {{ waitUntil(p: Promise<unknown>): void }} ctx
   */
  async fetch(request, env, ctx) {
    // CORS preflight: answer before touching R2.
    if (request.method === 'OPTIONS') {
      return new Response(null, {
        status: 204,
        headers: { ...corsHeaders(), 'Access-Control-Max-Age': '86400' },
      });
    }
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      return errorResponse(405, 'method not allowed', { Allow: 'GET, HEAD, OPTIONS' });
    }

    const url = new URL(request.url);
    // The object key is the URL path, minus the leading slash. `decodeURIComponent`
    // turns `%2F` etc. back into the stored key; a leading-slash-only path is empty.
    let key;
    try {
      key = decodeURIComponent(url.pathname.replace(/^\/+/, ''));
    } catch {
      return errorResponse(400, 'bad request path');
    }
    if (key.length === 0) {
      return errorResponse(404, 'not found');
    }

    const rangeHeader = request.headers.get('Range');

    // Cache lookup. The key folds the object key and the requested range together so
    // each distinct byte range is cached as its own entry. GET only; a HEAD is cheap
    // and never populates or reads the cache.
    const cache = caches.default;
    const cacheKey = cacheKeyFor(url.origin, key, rangeHeader);
    if (request.method === 'GET') {
      const hit = await cache.match(cacheKey);
      if (hit) return fromCache(hit);
    }

    // A HEAD is enough to learn the size, validate the range, and answer 404/416
    // without ever reading a body.
    const head = await env.ARCHIVES.head(key);
    if (!head) return errorResponse(404, 'not found');
    const size = head.size;

    const parsed = parseRange(rangeHeader, size);
    if (parsed.type === 'unsatisfiable') {
      return errorResponse(416, 'range not satisfiable', {
        'Content-Range': `bytes */${size}`,
        'Accept-Ranges': 'bytes',
      });
    }

    const isRange = parsed.type === 'range';
    const status = isRange ? 206 : 200;
    const length = isRange ? parsed.length : size;
    const contentRange = isRange ? `bytes ${parsed.offset}-${parsed.offset + parsed.length - 1}/${size}` : null;

    const headers = new Headers({
      ...corsHeaders(),
      'Accept-Ranges': 'bytes',
      'Content-Type': head.httpMetadata?.contentType || 'application/octet-stream',
      'Content-Length': String(length),
    });
    if (contentRange) headers.set('Content-Range', contentRange);
    if (head.httpEtag) headers.set('ETag', head.httpEtag);

    // A HEAD returns the computed headers with no body and never reads R2 twice.
    if (request.method === 'HEAD') {
      return new Response(null, { status, headers });
    }

    const obj = isRange
      ? await env.ARCHIVES.get(key, { range: { offset: parsed.offset, length: parsed.length } })
      : await env.ARCHIVES.get(key);
    // The object could have been deleted between the head and the get.
    if (!obj || !obj.body) return errorResponse(404, 'not found');

    const response = new Response(obj.body, { status, headers });

    // Store the body for reuse. The Cache API will not accept a 206 or a
    // `Content-Range` response, so we stash a normalized 200 that records the real
    // status and range in internal headers, then reconstruct on the way out.
    ctx.waitUntil(cache.put(cacheKey, toCacheable(response)));
    return response;
  },
};

// Builds the cache key: the object key plus the normalized range as a query param,
// so `bytes=0-99` and `bytes=100-199` and a full fetch are three separate entries.
function cacheKeyFor(origin, key, rangeHeader) {
  const u = new URL(`${origin}/${encodeURIComponent(key)}`);
  u.searchParams.set('range', rangeHeader ? rangeHeader.trim() : 'full');
  return new Request(u.toString(), { method: 'GET' });
}

// Converts an outgoing (possibly 206 + Content-Range) response into a form the Cache
// API will accept: status 200, with the real status and Content-Range preserved in
// internal headers. The body is cloned so the original still streams to the client.
function toCacheable(response) {
  const headers = new Headers(response.headers);
  headers.set(CACHE_STATUS_HEADER, String(response.status));
  const contentRange = headers.get('Content-Range');
  if (contentRange) {
    headers.set(CACHE_RANGE_HEADER, contentRange);
    headers.delete('Content-Range');
  }
  return new Response(response.clone().body, { status: 200, headers });
}

// Reconstructs the client-facing response from a cached entry, restoring the real
// status and Content-Range and stripping the internal bookkeeping headers.
function fromCache(cached) {
  const headers = new Headers(cached.headers);
  const status = Number(headers.get(CACHE_STATUS_HEADER)) || 200;
  const contentRange = headers.get(CACHE_RANGE_HEADER);
  headers.delete(CACHE_STATUS_HEADER);
  headers.delete(CACHE_RANGE_HEADER);
  if (contentRange) headers.set('Content-Range', contentRange);
  return new Response(cached.body, { status, headers });
}

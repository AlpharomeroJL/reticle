# Deployment

This chapter covers running the public demo: the rate-limited demo server, the
collaboration relay a spectator watches, the browser bundle, and the release-time
secret scan. It is written for a small VPS behind a reverse proxy.

The demo server is `reticle-demo-server`, a composition binary (ADR 0024) that
brings up three things in one process:

- the rate-limited HTTP service from `reticle-demo` (`POST /submit`,
  `GET /status/{id}`, `POST /cancel`);
- an in-process `reticle-server` collaboration relay, so a visitor can watch the
  room each session draws into;
- a harness: the real `reticle-agent` propose-verify-correct loop when a key is
  present, otherwise a deterministic offline scripted loop (ADR 0025).

Run it locally with `just demo-up`.

## The server cannot be started unbounded

The service is built from a `reticle_demo::LimitConfig`, and there is no
constructor that omits it: `DemoServer::new(LimitConfig)` and
`DemoServer::with_harness(LimitConfig, harness)` both take the limits by value.
The type is defined in `crates/reticle-demo/src/limits.rs`, and every field is
enforced on the wire (the enforcement is covered by the abuse tests in
`crates/reticle-demo/tests/abuse.rs`). This is the point of the demo: it is safe
to expose to the open internet because it physically cannot run without limits.

## The mandatory limits and why each matters

`reticle-demo-server` builds a non-permissive `LimitConfig` (see
`demo_limits()` in `crates/reticle-demo-server/src/config.rs`). The values, and
what each protects against:

| Field | Value | On breach | Why it matters |
| --- | --- | --- | --- |
| `per_ip_rate_per_min` | 6 | `429` | Caps how fast one source IP can submit, so a single visitor cannot flood the queue. |
| `per_ip_concurrency` | 1 | `409` | One live session per IP, so one visitor cannot hold multiple agent loops at once. |
| `global_concurrency` | 4 | `503` | A hard ceiling on concurrent agent loops across the whole server, bounding CPU, memory, and (with a real key) model spend. |
| `token_budget` | 100000 | session cancelled | A runaway loop is cancelled before it can burn tokens. |
| `command_budget` | 200 | session cancelled | Bounds how many edits one session can issue, so a loop cannot grow a document without limit. |
| `max_prompt_len` | 400 chars | `400` | Bounds the input a visitor can submit. |
| `allowed_vocabulary` | task words | `400` | A prompt straying off the layout task vocabulary is rejected before it reaches a model, so the demo cannot be used as a general-purpose model proxy. |

The order of checks matters: cheap input validation (length, then vocabulary)
runs before any stateful counter is touched, so a malformed prompt never consumes
a rate token or a concurrency slot.

Tune these for the host. A larger VPS can raise `global_concurrency`; a
cost-sensitive deployment can lower `token_budget`. Keep `allowed_vocabulary`
non-empty in any public deployment so the proxy-abuse guard stays on.

## The API key is never baked into the image

The Anthropic API key is read only from the `ANTHROPIC_API_KEY` environment
variable, by `reticle_agent::AnthropicModel::from_env`. It is held in an `ApiKey`
wrapper that never prints, serializes, or logs the clear value, and it reaches the
wire only as the `x-api-key` header. It is never written to a file, a transcript,
or an artifact.

Consequences for deployment:

- **Do not** put the key in the `Dockerfile`, an image layer, a committed `.env`,
  or a compose file checked into git. Provide it at run time only.
- With Docker: `-e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"` (read from the host
  environment), or a Docker/Compose secret mounted into the environment at start.
- With systemd: an `EnvironmentFile=` that is `chmod 600`, root-owned, and outside
  the repo.
- Without a key, the server still runs: it uses the offline scripted harness, so
  `just demo-up` and a keyless container both work with no network.

Before every release, run the secret scan (`just check-keys`) to prove no key or
other credential has been committed anywhere in the tree. It scans for the
`sk-ant-` prefix, generic `api_key`/`secret`/`token` assignments, and long
high-entropy strings, and exits non-zero on a hit. Pass `-History` to also scan
the full git history.

## Running with Docker

The repository ships a multi-stage `Dockerfile` that builds the binary in release
and runs it as a non-root user (`reticle`, uid 10001). It links rustls (no OpenSSL
system dependency) and uses the vendored `protoc` (ADR 0008), so the runtime image
is a slim Debian with only `ca-certificates` added (needed for the outbound HTTPS
call to the Anthropic API when a key is set).

```sh
# Build from the repo root (the build context).
docker build -t reticle-demo .

# Offline (no key): the scripted harness runs.
docker run --rm -p 3040:3040 -p 3041:3041 -e HOST=0.0.0.0 reticle-demo

# Live model: the key comes from the host environment, never from the image.
docker run --rm -p 3040:3040 -p 3041:3041 -e HOST=0.0.0.0 \
    -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" reticle-demo
```

The container binds `HOST=0.0.0.0` inside, and the host maps ports 3040 (the demo
service) and 3041 (the relay). Override `PORT` and `RETICLE_RELAY_ADDR` to change
them.

## Behind a reverse proxy

Terminate TLS at a reverse proxy (nginx, Caddy, or Traefik) and forward to the
demo service. The relay is a WebSocket endpoint (`GET /ws/{room}`), so the proxy
must pass the upgrade headers for that path.

The service reads the real client IP from the `x-demo-client-ip` request header
when present (the header a trusted front proxy sets), falling back to the peer
address; this is `reticle_demo::CLIENT_IP_HEADER`. Set that header at the proxy to
the true client address so per-IP limits apply to real clients rather than to the
proxy. Do not expose the header to untrusted clients directly, or a client could
spoof its source IP; only a trusted proxy should set it.

A minimal nginx sketch (TLS omitted for brevity):

```nginx
# Demo HTTP service.
location /api/ {
    proxy_pass http://127.0.0.1:3040/;
    proxy_set_header x-demo-client-ip $remote_addr;
}

# Collaboration relay (WebSocket upgrade).
location /ws/ {
    proxy_pass http://127.0.0.1:3041;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
}
```

## Resource ceilings

The limits above bound the demo's own work, but set OS and container ceilings too
so a bug or an unusually heavy session cannot take down the host:

- **Memory**: cap the container (for example `docker run --memory=1g`). Each agent
  loop holds a small in-memory document and CRDT; the relay keeps an in-memory log
  per room (see the `reticle-server` room notes), so restart the process
  periodically or run behind an orchestrator that recycles it if rooms accumulate.
- **CPU**: cap CPU (for example `--cpus=2`). `global_concurrency` already bounds
  how many loops run at once; the CPU cap is a backstop.
- **Disk**: the demo server writes no artifacts (unlike the batch CLI), so its
  disk use is negligible; still, cap logs.
- **Network egress**: with a key, the only outbound traffic is to
  `api.anthropic.com`. An egress allowlist to that host is a reasonable hardening
  step.

## Publishing the browser bundle to GitHub Pages

The public link is the `crates/web` Trunk bundle, which mounts `reticle-app`. The
web entry point reads a `?view=` query parameter and defaults a public visitor to
the replay theater (ADR 0026): the theater plays a recorded agent transcript back
through a live session, so a visitor sees motion and DRC overlays with no key and
no setup. The `index.html` frames the theater with an always-visible link to the
full editor (`?view=editor`) and back.

The theater window itself is native-only today; on the wasm bundle the start view
is selected at the entry point and the framing is in place, while the in-page
theater window lands once the theater modules are un-gated for wasm (they are
model-free; this is tracked as `TODO(wave3)` in `crates/reticle-app/src/lib.rs`).
The native desktop app opens the theater fully via
`App::with_start_view(StartView::ReplayTheater)`.

The release step (Wave 3) publishes the bundle to `gh-pages`:

```sh
# Build the optimized wasm bundle.
just web-build           # -> crates/web/dist

# Build the book.
just book                # -> docs/book

# Publish dist/ (and the book) to the gh-pages branch, then enable Pages to serve
# from that branch. This project uses no GitHub Actions; the release skill builds
# the site locally and pushes the branch.
```

The published `index.html` and the web entry point default a public visitor to the
replay theater view (with the editor one click away). Nothing about the browser
bundle needs a key: the theater replays a committed scripted transcript.

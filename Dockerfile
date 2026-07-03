# Multi-stage build for the Reticle public demo server (reticle-demo-server).
#
# Stage 1 builds the demo binary in release from the workspace. Stage 2 is a slim
# runtime that runs it as a non-root user with the mandatory demo limits (which the
# binary enforces and cannot be started without, since DemoServer is built from a
# LimitConfig; see crates/reticle-demo/src/limits.rs).
#
# The ANTHROPIC_API_KEY is NEVER baked into the image. It is provided at run time
# via an environment variable or a secret; without it the server runs the offline
# scripted harness. See docs/deployment.md.
#
# Build (from the repo root, which is the build context):
#   docker build -t reticle-demo .
# Run (offline, no key):
#   docker run --rm -p 3040:3040 -p 3041:3041 -e HOST=0.0.0.0 reticle-demo
# Run (live model, key from the host environment, never in the image):
#   docker run --rm -p 3040:3040 -p 3041:3041 -e HOST=0.0.0.0 \
#       -e ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" reticle-demo

# ---- Stage 1: builder ------------------------------------------------------
FROM rust:1.94-slim-bookworm AS builder

WORKDIR /build

# The binary links rustls (via ureq and tokio-tungstenite), so no OpenSSL dev
# package is needed. protoc is provided vendored by reticle-proto (ADR 0008), so
# no system protoc is installed either. Only a C toolchain for a few build scripts.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

# Copy the whole workspace. reticle-demo-server embeds tech/sky130.tech via
# include_str!, so the tech directory must be present at build time.
COPY . .

# Build just the demo server (and its dependency graph) in release.
RUN cargo build --release -p reticle-demo-server \
    && cp target/release/reticle-demo-server /reticle-demo-server \
    && strip /reticle-demo-server

# ---- Stage 2: runtime ------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates is required for the outbound HTTPS call to the Anthropic API
# when a key is provided; nothing else is needed at run time.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    # A dedicated non-root user; the server never needs root.
    && useradd --system --no-create-home --uid 10001 reticle

COPY --from=builder /reticle-demo-server /usr/local/bin/reticle-demo-server

USER reticle

# Bind to all interfaces inside the container; the host maps the ports. The relay
# rides alongside on 3041. Override HOST/PORT/RETICLE_RELAY_ADDR as needed.
ENV HOST=0.0.0.0 \
    PORT=3040 \
    RETICLE_RELAY_ADDR=0.0.0.0:3041

EXPOSE 3040 3041

# No shell form, so signals reach the process directly.
ENTRYPOINT ["/usr/local/bin/reticle-demo-server"]

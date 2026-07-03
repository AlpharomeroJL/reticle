# Reticle automation and build gate.
#
# `just ci` is the authoritative CI. There is NO GitHub Actions in this project;
# this recipe is the sole gate and must be green before every commit and at every
# wave merge. Each `reticle-dev` MCP operation mirrors a recipe here so the build
# never blocks on the MCP being loaded.
#
# Windows note: recipes run under Windows PowerShell 5.1. Each gate step is a
# single native command so its exit code propagates to `just` (PowerShell only
# reliably forwards the exit code of a trailing native command). Composite steps
# end with `exit $LASTEXITCODE`.

set windows-shell := ["powershell.exe", "-NoProfile", "-Command"]

# List available recipes.
default:
    just --list

# Build the whole workspace (native).
build:
    cargo build --workspace

# Formatting check plus clippy.
lint: fmt-check clippy

# ---------------------------------------------------------------------------
# The gate (replaces GitHub Actions): fmt, clippy(-D warnings), tests, doc,
# wasm build, license/advisory check, spelling.
# ---------------------------------------------------------------------------
# Fail-fast order: the cheapest checks run first (style grep, fmt), then clippy,
# then tests, then the slow tail (docs, wasm, deny, typos), so a broken lane
# learns in seconds rather than minutes.
ci: check-style fmt-check clippy test doctest doc-build wasm-build deny typos
    Write-Output "ci: GREEN"

# ---- Formatting ----
fmt-check:
    cargo fmt --all -- --check

fmt:
    cargo fmt --all

# ---- Lints (warnings are errors) ----
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# ---- Tests ----
test:
    cargo nextest run --workspace --no-tests=pass

# nextest does not run doctests, so run them explicitly.
doctest:
    cargo test --workspace --doc

# ---- Docs (broken intra-doc links are errors) ----
doc-build:
    $env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps --document-private-items

# ---- WebAssembly build (proves the browser harness compiles to wasm32) ----
wasm-build:
    cargo build -p web --target wasm32-unknown-unknown

# ---- Supply chain: licenses + advisories ----
deny:
    cargo deny check

# ---- Spelling ----
typos:
    typos

# ---- Style: the voice rule forbids em-dashes (U+2014) ----
check-style:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/check-style.ps1

# ---- Secret scan: fail if any leaked key/secret pattern is in the working tree ----
# Pass `-History` to also scan the full git history (slower). Runs before every
# release; the real Anthropic key must only ever come from the environment.
check-keys *args:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/check-keys.ps1 {{args}}

# ---- Unused dependency audit (advisory; not part of the hard gate) ----
machete:
    cargo machete

# ---------------------------------------------------------------------------
# Benchmarks and performance
# ---------------------------------------------------------------------------
bench:
    cargo bench --workspace

# Run the agent benchmark suite against the deterministic mock model. With no
# args this runs the whole sample suite under benchmarks/layout-tasks; pass
# through flags to scope it, e.g. `just bench-agent --tier 1` or
# `just bench-agent --task t1_place_met1_rect`.
bench-agent *args:
    cargo run -p reticle-bench -- {{args}}

# Promote a mined candidate task (benchmarks/layout-tasks/candidates/<id>.toml)
# into the live suite. Refuses unless the candidate's checker passes its
# two-way vectors (accepts the good document, rejects the bad one); on success
# the manifest gains the task and its minor version is bumped.
bench-promote id *args:
    cargo run -p reticle-bench -- promote {{id}} {{args}}

perf-check:
    cargo run -p xtask --release -- perf-check

# ---------------------------------------------------------------------------
# WASM demo (Trunk) and book
# ---------------------------------------------------------------------------
# Trunk resolves the crate from its own directory (the workspace root is a virtual
# manifest), so these run from crates/web.
web-build:
    cd crates/web; trunk build index.html --release

web-serve:
    cd crates/web; trunk serve index.html

# ---------------------------------------------------------------------------
# GitHub Pages artifact (the public "front door")
# ---------------------------------------------------------------------------
# The site is served under the subpath https://alpharomerojl.github.io/reticle/,
# so Trunk MUST emit assets under `/reticle/` (via --public-url) or the browser
# fetches them at absolute root and 404s, hanging the page on the spinner.
#
# `deploy-pages` builds the release bundle with the subpath baked in, builds the
# book, assembles the FULL gh-pages artifact into a fresh scratch/pages/ (web
# bundle + .nojekyll + book/), and asserts the emitted index.html references
# `/reticle/`-prefixed assets with no bare `/web-` absolute-root reference left.
# It never touches git; the orchestrator publishes scratch/pages/ to gh-pages.
# scratch/ is gitignored, so nothing here is committed.
deploy-pages:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/deploy-pages.ps1

# `smoke-pages` is a DEPLOYED-URL check: it fetches the live index.html, extracts
# every asset it references, and asserts each returns 200 and sits under the
# `/reticle/` prefix. It only passes after the orchestrator redeploys the correct
# artifact; against the currently-broken live site it fails and says why. Pass a
# different base URL as the argument to point it elsewhere.
smoke-pages base="https://alpharomerojl.github.io/reticle/":
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-pages.ps1 -BaseUrl {{base}}

# ---------------------------------------------------------------------------
# End-to-end browser tests (Playwright), its own gate.
# ---------------------------------------------------------------------------
# Builds the Trunk demo bundle (root paths, served at root), then drives it in
# headless Chromium. Two projects run here: `webgl2` is the hard gate (WebGPU is
# hidden so wgpu takes its WebGL2 fallback, and the app must boot and render);
# `webgpu` launches with the WebGPU-enabling flags and asserts the WebGPU path
# where a real adapter exists, skipping those checks honestly where it does not
# (Playwright's headless Chromium ships without WebGPU). The `ghpages-subpath`
# project is excluded here because it needs the `--public-url /reticle/` build;
# run it via `just e2e-subpath`. See e2e/README.md and ADR 0027.
e2e:
    cd crates/web; trunk build index.html
    npm --prefix e2e install
    cd e2e; npx playwright install chromium
    cd e2e; npx playwright test --project=webgl2 --project=webgpu

# gh-pages subpath boot gate. Builds the bundle WITH `--public-url /reticle/`
# (the deploy shape) and runs the `ghpages-subpath` Playwright project, which
# serves that bundle under `/reticle/` and asserts the app boots with no 404 on
# the js/wasm. This is the fail-before-deploy guard for the base-path regression
# that broke the front door. A root-path build would 404 here, which is the point.
e2e-subpath:
    cd crates/web; trunk build index.html --release --public-url /reticle/
    npm --prefix e2e install
    cd e2e; npx playwright install chromium
    cd e2e; npx playwright test --project=ghpages-subpath

book:
    mdbook build docs

book-serve:
    mdbook serve docs

# ---------------------------------------------------------------------------
# Public demo server (reticle-demo-server)
# ---------------------------------------------------------------------------
# Build and run the rate-limited demo: the demo HTTP service plus an in-process
# collaboration relay a spectator can watch. Uses the real reticle-agent harness
# when ANTHROPIC_API_KEY is set in the environment, otherwise a deterministic
# offline harness so this works with no key and no network. Configure with the
# HOST/PORT and RETICLE_RELAY_ADDR environment variables (defaults 127.0.0.1:3040
# and 127.0.0.1:3041). See docs/deployment.md.
demo-up:
    cargo run -p reticle-demo-server --release

# ---------------------------------------------------------------------------
# Headless pipeline helpers (layout generation, DRC, routing, media)
# ---------------------------------------------------------------------------
gen-layout shapes="1000000" layers="8" depth="3" out="scratch/gen.gds":
    cargo run -p xtask --release -- gen-layout --shapes {{shapes}} --layers {{layers}} --depth {{depth}} --out {{out}}

drc-run file:
    cargo run -p reticle-cli --release -- drc {{file}}

route-run file:
    cargo run -p reticle-cli --release -- route {{file}}

capture-media:
    cargo run -p xtask --release -- capture-media

# ---------------------------------------------------------------------------
# Nightly-only: fuzzing and miri
# ---------------------------------------------------------------------------
fuzz target time="60":
    cargo +nightly fuzz run {{target}} -- -max_total_time={{time}}

miri:
    cargo +nightly miri test --workspace

# ---------------------------------------------------------------------------
# Coverage
# ---------------------------------------------------------------------------
coverage:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

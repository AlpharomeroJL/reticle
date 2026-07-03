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
# End-to-end browser tests (Playwright), its own gate.
# ---------------------------------------------------------------------------
# Builds the Trunk demo bundle, then drives it in headless Chromium. Two
# projects: `webgl2` is the hard gate (WebGPU is hidden so wgpu takes its WebGL2
# fallback, and the app must boot and render); `webgpu` launches with the
# WebGPU-enabling flags and asserts the WebGPU path where a real adapter exists,
# skipping those checks honestly where it does not (Playwright's headless
# Chromium ships without WebGPU). See e2e/README.md and ADR 0027.
e2e:
    cd crates/web; trunk build index.html
    npm --prefix e2e install
    cd e2e; npx playwright install chromium
    cd e2e; npx playwright test

book:
    mdbook build docs

book-serve:
    mdbook serve docs

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

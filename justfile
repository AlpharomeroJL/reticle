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
ci: fmt-check clippy test doctest doc-build wasm-build deny typos
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

# ---- Unused dependency audit (advisory; not part of the hard gate) ----
machete:
    cargo machete

# ---------------------------------------------------------------------------
# Benchmarks and performance
# ---------------------------------------------------------------------------
bench:
    cargo bench --workspace

perf-check:
    cargo run -p xtask --release -- perf-check

# ---------------------------------------------------------------------------
# WASM demo (Trunk) and book
# ---------------------------------------------------------------------------
web-build:
    trunk build --release crates/web/index.html

web-serve:
    trunk serve crates/web/index.html

book:
    mdbook build docs

book-serve:
    mdbook serve docs

# ---------------------------------------------------------------------------
# Headless pipeline helpers (layout generation, DRC, routing, media)
# ---------------------------------------------------------------------------
gen-layout shapes="1000000" layers="8" depth="3" out="scratch/gen.rgds":
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

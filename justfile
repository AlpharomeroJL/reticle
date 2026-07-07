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
# Lane worktree management (ADR 0028, 0030): the orchestrator creates a lane
# worktree BEFORE spawning its subagent and removes it after the wave merge.
# Lane isolation is a real worktree on disk, never an Agent-call parameter.
# ---------------------------------------------------------------------------
# Create an isolated worktree D:/dev/reticle-lanes/<name> on a new branch
# lane/<name> off main. Then spawn the subagent pinned to that directory with its
# own CARGO_TARGET_DIR=D:/dev/reticle-target-<name>.
lane name:
    git worktree add ../reticle-lanes/{{name}} -b lane/{{name}} main
    @echo "worktree ready: D:/dev/reticle-lanes/{{name}}  (branch lane/{{name}})"

# Remove a lane worktree and delete its branch after the integration merge.
# `git branch -d` refuses an unmerged branch, a deliberate guard against losing work.
lane-done name:
    git worktree remove ../reticle-lanes/{{name}}
    git branch -d lane/{{name}}

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
    cargo deny check --config .config/deny.toml

# ---- Spelling ----
typos:
    typos --config .config/typos.toml

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

# Run the whole agent suite against a LOCAL OpenAI-compatible model (Ollama).
# Configure the model from the environment first, then invoke:
#   $env:RETICLE_MODEL_NAME='gpt-oss:16k'; just bench-agent-ollama
# Optional env: RETICLE_MODEL_BASE_URL (default http://localhost:11434/v1),
# RETICLE_MODEL_API_KEY (only if your endpoint needs a key). Pass through extra
# flags to scope or annotate, e.g. `just bench-agent-ollama --tier 1` or
# `just bench-agent-ollama --quantization Q4_K_M`. Writes an aggregate results
# JSON under scratch/agent-suite-results and prints a Markdown summary.
# NOTE: hits a real local model (GPU load; non-deterministic proposals) and is
# NOT part of `just ci`.
bench-agent-ollama *args:
    cargo run -p reticle-agent -- --backend ollama --suite benchmarks/layout-tasks {{args}}

# Run the whole agent suite through CLAUDE CODE as an external agent system. Per
# task this writes an MCP config launching reticle-mcp (server-side transcript
# capture + budget), runs `claude -p` over it, then replays the captured
# transcript and runs the task's checker. Requires the `claude` CLI on PATH and an
# authenticated session; a missing or unauthenticated CLI is recorded as an honest
# not-run (never a fabricated pass/fail). Optional env: RETICLE_CLAUDE_BIN (the
# claude executable), RETICLE_MCP_BIN (the reticle-mcp executable). Pass through
# extra flags to scope or pick the model, e.g.
#   just bench-agent-claude-code --task t1_drc_clean_met1 --model sonnet
# A single-task smoke:
#   cargo run -p reticle-agent -- --backend claude-code --task benchmarks/layout-tasks/t1_drc_clean_met1.toml
# Writes suite-claude-code.json (ran records) and, if anything did not run,
# suite-claude-code-notrun.json under scratch/agent-suite-results, plus a summary.
# NOTE: hits the real Claude Code CLI (consumes quota; non-deterministic) and is
# NOT part of `just ci`.
bench-agent-claude-code *args:
    cargo run -p reticle-agent -- --backend claude-code --suite benchmarks/layout-tasks {{args}}

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

# Share-link LIVE transport e2e (ADR 0058). Builds the Trunk bundle AND the
# reticle-server relay binary, then runs the two-context `share-live` Playwright
# project: context A boots the editor and goes live (publishing into a relay room),
# context B opens the read-only viewer link and its browser transport streams A's
# live frames. `SHARE_LIVE=1` adds the relay webServer (serve-relay.mjs launches the
# prebuilt relay on 127.0.0.1:3030). The headless run proves the viewer bundle boots,
# the `?mode=view` socket opens, and real SyncMessage frames arrive and decode; the
# authoritative proof of the transport + read-only contract is the Rust relay test
# crates/reticle-server/tests/share_live.rs. See e2e/README.md.
e2e-share:
    cd crates/web; trunk build index.html
    cargo build -p reticle-server
    npm --prefix e2e install
    cd e2e; npx playwright install chromium
    cd e2e; $env:SHARE_LIVE='1'; npx playwright test --project=share-live

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

# UI-capture: drive the real editor window through the committed demo scripts
# (crates/reticle-app/demo-scripts/*.demo) and assemble the README media (hero still
# plus tour GIFs, each under 6 MB) from full-window screenshots. Pass a name to limit
# to one capture, e.g. `just capture-ui tour-drc`. Reproducible: same scripts, same
# media. Needs `gifski` on PATH and a GPU (opens the app window per capture).
capture-ui *args:
    cargo run -p xtask --release -- capture-ui {{args}}

# ---------------------------------------------------------------------------
# TinyTapeout precheck oracle (ADR 0054): run TinyTapeout's OWN precheck over a
# GDS as the authoritative GDS-mode submission gate. Additive and NOT part of
# `just ci`: it needs Docker and a multi-GB pinned image, exactly like the
# nightly-only fuzz/miri recipes below.
# ---------------------------------------------------------------------------
# Runs TinyTapeout's precheck (Magic DRC + KLayout + pin/boundary/layer/naming
# checks) over <gds> inside the PINNED `hpretl/iic-osic-tools` container (Magic +
# KLayout + gdstk + the SKY130 PDK baked in). The wrapper stages a minimal
# TinyTapeout project (info.yaml with top_module = the GDS stem, which the
# precheck requires), checks out `TinyTapeout/tt-support-tools`, runs
# `python precheck/precheck.py --gds <gds> --tech sky130A` in the container, and
# copies the reports (results.md, results.xml, magic_drc.txt, drc_*.xml) to the
# out dir. The exit code is the precheck's own (0 = passed). The Rust parser
# `reticle_cli::tt_precheck::parse_reports_dir` turns that out dir into a
# structured PrecheckReport the agent loop consumes like DRC violations. WSL is a
# documented fallback (see scripts/tt-precheck.ps1 and ADR 0054).
#   just tt-precheck scratch/tile.gds
#   just tt-precheck scratch/tile.gds scratch/precheck-reports
tt-precheck gds out="scratch/precheck-reports":
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/tt-precheck.ps1 -Gds {{gds}} -OutDir {{out}}

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

# ---------------------------------------------------------------------------
# Relay conformance (ADR 0062): one vector table, both relays.
# ---------------------------------------------------------------------------
# Wave-gate recipe (NOT part of `just ci`, which stays Node-free). The native
# half always runs in-process; the Durable Object half spawns `wrangler dev
# --local` (miniflare, no Cloudflare auth) and runs the identical vectors when
# worker/node_modules exists, which it bootstraps here with `npm ci`. The DO
# test self-skips when RETICLE_CONFORMANCE_DO is unset, so a plain
# `cargo nextest run -p reticle-relay-conformance` stays Node-free.
conformance:
    if (-not (Test-Path worker/node_modules)) { npm --prefix worker ci }
    $env:RETICLE_CONFORMANCE_DO = "1"; cargo nextest run -p reticle-relay-conformance --no-tests=pass; exit $LASTEXITCODE

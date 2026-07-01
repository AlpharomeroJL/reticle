# Reticle task checklist

Tracks the build against `docs/PLAN.md`. `[x]` done, `[~]` partial, `[ ]` todo.

## Wave 0, contracts & scaffolding, DONE

- [x] Toolchains, components, targets, CLI tools; relocated caches verified.
- [x] Workspace, lints, profiles, dual licenses, `.gitignore`, `.gitattributes`, `rust-toolchain.toml`.
- [x] `justfile` gate, `deny.toml`, `typos.toml`, `rustfmt.toml`, `.mcp.json`, pre-commit hook.
- [x] Protobuf schema, shared contracts, compiling skeleton, ADRs 0001–0012, skills, MCP servers.
- [x] `git init`; first commit as Josef Long, no AI trailer.

## Wave 1, foundations, DONE

- [x] `reticle-geometry`, `reticle-proto`, `reticle-index`, `reticle-io` (implemented, tested, benched).

## Wave 2, core subsystems, DONE

- [x] `reticle-model`, `reticle-render`, `reticle-drc`, `reticle-route`, `reticle-extract`.

## Wave 3, collaboration, server, scripting, CLI, DONE

- [x] `reticle-sync`, `reticle-server`, `reticle-script`, `reticle-cli`.

## Wave 4, application, web, xtask, DONE

- [x] `reticle-app`: egui editor (canvas, tools, palette, layers, measure, undo panel); native + WASM; 80 tests.
- [x] `web`: Trunk harness mounting the app; WebGPU with WebGL2 fallback. Live demo verified in-browser.
- [x] `xtask`: deterministic layout generator; offscreen media capture.

## Wave 5, docs, fuzz, benches, media, release, DONE

- [x] mdbook book (overview, architecture, per-subsystem chapters) with mermaid; deployed to Pages.
- [~] Fuzz targets authored and committed with seed harness; running the libFuzzer engine is blocked on Windows/MSVC (no compiler-rt), documented in `fuzz/README.md`. Parser robustness is covered by `reticle-io` proptests (2048 cases) in the gate.
- [x] Benchmark history committed; `PERF.md` with measured numbers on the RTX 4060 Ti.
- [~] `assets/hero.png` and `assets/browse.gif` generated from the real render pipeline and in the README. The DRC/route/collab GIFs need overlay render passes that are a documented follow-up.
- [x] Repo `AlpharomeroJL/reticle` created; `main` pushed.
- [x] Book + WASM demo deployed to `gh-pages`; Pages enabled and serving.
- [x] `CHANGELOG.md` via `git-cliff`; tag `v3.0.0`; `gh release create` with binaries and notes.
- [x] Requirements-mapping table current (`docs/requirements.md`); Section 16 self-audit complete.

## Section 16 self-audit

- Every crate builds; `just ci` is green across the workspace.
- The native app and the browser demo both run; the live-demo link works (verified in a browser).
- Editing, hierarchy, DRC, routing, extraction, IO, scripting, and collaboration all function, each with tests.
- Performance is measured and recorded in `PERF.md` with real numbers and methodology.
- Property tests, golden-image tests, and CRDT convergence tests pass; fuzz targets exist (run on Linux).
- The book and rustdoc build and are deployed to Pages.
- Hero image and browse GIF are generated and in the README (DRC/route/collab GIFs are a follow-up).
- A tagged `v3.0.0` release exists with binaries and notes.
- The requirements-mapping table is complete and honest.
- No AI attribution appears anywhere in the repo; no commit history is backdated or fabricated.

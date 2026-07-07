# RESULT - lane v8-5d-interop-pdk

Status: **GREEN.** All four mission pieces landed, the gate is green, and the container
comparison ran (Docker was available and the pinned image was on disk).

## Commits (on `lane/v8-5d-interop-pdk`, not pushed)

| sha | summary |
|-----|---------|
| `b553ccd` | refactor(gen): data-driven GenTech, generators read it via the tech arg |
| `eddc89a` | feat(gen): second PDK (IHP SG13G2) + both-PDK cleanliness proptests |
| `5a28979` | build: lock reticle-gen dev-deps (reticle-io, toml) |
| `6737114` | feat(io): conformant-OASIS writer + GDS/OASIS interop harness |
| `e7e8f61` | docs: interop + second-PDK chapters, ADRs 0068-0070, OASIS honesty rename |

## Gate (all green)

- `cargo nextest run -p reticle-io -p reticle-gen` -> **137 passed, 3 skipped**.
- `cargo clippy -p reticle-io -p reticle-gen --all-targets -- -D warnings` -> clean.
- `cargo doc -p reticle-io -p reticle-gen --no-deps` -> clean.
- Both-PDK cleanliness proptests pass (`crates/reticle-gen/tests/second_pdk.rs`: every
  generator over `[sky130, sg13g2]`, zero DRC violations under each process's deck).
- `powershell -File scripts/check-style.ps1` -> OK.
- Full-workspace `cargo clippy --workspace --all-targets -- -D warnings` clean (the
  pre-commit hook ran it on every commit): no downstream crate broke.

## 1. GenTech refactor (the linchpin)

`crates/reticle-gen/src/gentech.rs` introduces `GenTech`, a value carrying the process
numbers the six generators draw against (four interconnect conductors, three bridging
cuts, the substrate-tap cut, a conservative cut pitch). Generators read it via the
`Technology` argument threaded into `generate()` (`GenTech::for_technology`, defaulting
to SKY130). The SKY130 numbers are unchanged, so behavior on the default technology is
byte-for-byte identical. The refactor is internal-only and signature-preserving: the
`sky130` module was already private, and no `Generator`/`GenParams`/`Registry`
signature changed, so `reticle-app`/`reticle-mcp`/`reticle-bench`/`reticle-agent` are
untouched (public API only grew: `Conductor`, `Cut`, `GenTech`, `Residue`,
`derive_gentech`). ADR 0068.

**Proof across both PDKs:** the generalized oracle runs every generator over SKY130 and
SG13G2 and asserts zero DRC violations under each process's own committed deck. Two
provenance tests anchor SG13G2: `derive_gentech(parsed tech) == GenTech::sg13g2()` and
`.tech` rules == `.toml` subset. Making it hold took one generator change - the contact
chain now encloses each contact by both bridging conductor levels, so whichever level a
process requires (met1 encloses mcon on SKY130, Metal1 encloses Via1 on SG13G2) is
satisfied.

## 2. Second PDK (IHP SG13G2)

`tech/ihp-sg13g2.tech` (layers + physical stack + inline DRC subset) and
`tech/sg13g2-drc-subset.toml` (cited source of record). Every number is transcribed
from the open IHP-Open-PDK KLayout DRC runset (branch main, **Apache-2.0**) with rule
ids preserved and provenance/license recorded in both files. ADR 0069, book chapter
`docs/src/second-pdk.md`.

## 3. GDS round-trip interop harness + divergence report

`scripts/interop/` runs a fixture through Reticle, KLayout, and gdspy in the pinned
`hpretl/iic-osic-tools:2025.01` container (KLayout 0.29.10, gdspy 1.6.13), normalizes
every output with one authoritative reader, and writes a report. **The container
comparison RAN.**

**Divergence report:** `docs/interop/gds-oasis-divergence-report.md` (curated), with the
machine-generated `docs/interop/gds-roundtrip.generated.md` alongside.

Two-way result, as required:
- **Clean design round-trips:** Reticle, KLayout, and gdspy round-trip the clean
  fixture identically (geometry, labels, instances all match).
- **Seeded odd design surfaces a documented divergence:** on a reference rotated 45 deg
  with 2x magnification, Reticle recovers 90 deg while KLayout and gdspy keep 45 deg.
  Cause (honest): Reticle's `Orientation` model encodes only orthogonal orientations, so
  its GDS importer snaps a non-orthogonal angle to the nearest 90 deg rather than
  dropping it. A modelling limitation, not a reader/writer bug; magnification and origin
  round-trip correctly in all three tools.

## 4. Timeboxed conformant-OASIS writer - PASSED

`crates/reticle-io/src/oasis_std.rs` (`OasisStd`) is a genuine SEMI P39 OASIS writer for
a practical subset. **KLayout reads its output** - verified in-container by the harness
(both fixtures: `OASIS-READ OK`, 2 cells, 6 shapes, dbu = 0.001). The one allowed
`reticle-io/lib.rs` edit adds the `oasis_std` module. Export only (reader out of scope),
uncompressed, RECTANGLE/POLYGON/PATH/PLACEMENT/TEXT with fully explicit modal state,
CELLNAME+CELL tables, PLACEMENT type 18 with magnification and angle. ADR 0070.

## 5. Docs / OASIS honesty rename

The internal `oasis.rs` is renamed in the docs to "the Reticle container format
(OASIS-inspired, ADR 0004)" and stated plainly to be unreadable by KLayout/gdstk
(`docs/src/io.md`, `docs/src/positioning.md`, `reticle-io/lib.rs` docs, `README.md`).
The KLayout/gdstk round-trip scope is stated precisely in the interop chapter and report.

## Honest gaps

- **OASIS writer subset:** arrays are expanded to individual placements (no OASIS
  repetition; large arrays inflate the file), a label's anchor is dropped (OASIS TEXT is
  a point), and a path's round end cap is written flush (OASIS path extensions are flush
  / half-width / explicit only). Documented at the call site and in ADR 0070.
- **gdstk not preinstalled:** the pinned image lacks gdstk and PEP 668 blocks a clean
  in-container `pip install`, so the second tool is gdspy (its same-author predecessor,
  preinstalled). The report notes the exact command to add gdstk. KLayout is the other
  tool and the authoritative normalizer.
- **GDS instance-rotation divergence:** Reticle snaps a 45 deg instance rotation to
  90 deg (piece 3 above). This is the one real divergence the harness surfaces.
- **SG13G2 subset scope:** omits the wide-metal and pattern-density spacing variants
  (the DRC engine cannot express width-conditional spacing), the FEOL Activ/GatPoly
  contact enclosures (the generators draw no Activ/GatPoly), and the thick TopMetal
  stack. Passing it is not tape-out clean, exactly as for the SKY130 subset.
- **Test-only dependencies:** `reticle-gen` gains `reticle-io` and `toml` as
  dev-dependencies (so the both-PDK proptest can parse the committed tech files); the
  library itself stays `wasm32`-clean.
- **Harness driver placement:** the Reticle round-trip runs through a small detached
  standalone crate at `scripts/interop/reticle-roundtrip/` (its own `[workspace]`,
  path-dep on reticle-io) rather than a binary added to the frozen reticle-io crate.

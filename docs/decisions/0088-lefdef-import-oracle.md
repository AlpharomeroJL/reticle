# 0088, cross-validating LEF/DEF import against a pinned container oracle (OpenROAD)

## Context

`reticle-lefdef` imports a deliberate subset of LEF and DEF (ADR 0082) and lowers it to a
`LefDefDesign`. A subset importer is trustworthy only if its reading of a file matches what
a real EDA tool reads from the same file: the import tests assert the lowered document
against hand-computed expectations, but those expectations are the author's reading of the
format, not an independent tool's. ADR 0054 established a way to get an independent reading
without adding a heavy tool to the gate: run a real Linux-native tool in a pinned Docker
container as an external oracle, parse its structured output, and skip honestly when the
container is unavailable. The open questions here were which tool to use as the oracle,
which facts to compare (LEF/DEF is large and much of it, especially routing, is loosely
standardized), and how to prove the oracle both ways without the multi-GB image being
present in the ordinary gate.

## Decision

**Cross-check the import against OpenROAD in the pinned container, additively.** The
`reticle_cli::lefdef_oracle` module writes a LEF/DEF pair and a short Tcl script into a
work directory, mounts it into `hpretl/iic-osic-tools:2025.01` (amd64 digest
`sha256:a51257b7d85fc75d5a690317539f9787a401d6dd28583d73dceab174ccc9e78f`, the same
all-in-one image ADR 0054 pins for the precheck), and runs `openroad -no_init -exit`
over the script. The image is pinned by a dated tag, never `latest`, and the digest is
recorded in the module and here. The image entrypoint needs `--skip` as its first argument
to bypass the X11/VNC UI bootstrap and exec the assigned command, exactly as the precheck
wrapper documents.

**OpenROAD, not KLayout.** The image bundles both. OpenROAD's OpenDB Tcl API reads LEF/DEF
with `read_lef`/`read_def` and exposes the compared facts directly from the block and the
libraries (`getInsts`, `getBTerms`, `getDieArea`, `getMasters`), with no PDK and no flow
configuration, which makes the oracle script short and deterministic. The script prints
four facts as `ORACLE <key>=<value>` lines; `parse_oracle_output` reads them back into an
`OracleCounts` and ignores the tool banner and log lines.

**Compare four structural facts, not routing.** macros (LEF `MACRO` cells), components (DEF
`COMPONENTS` placed instances), pins (DEF `PINS`), and the die area (DEF `DIEAREA` box in
database units). Net-level routing is not compared: it is the richest and least
standardized part of DEF, and the four facts already discriminate a faithful import from a
corrupted one. `OracleCounts::agrees_with` requires components and pins to match exactly,
compares macros only when both sides report a count, and compares the die area coordinate
by coordinate within a documented tolerance. The tolerance exists for the case where a tool
reports the die area on a different unit grid; here it is zero, because both sides read DEF
database units directly.

## Consequences

The harness, the parser, and the fixtures are real and committed. Two layers of test prove
the cross-check both ways. The parser-level test always runs in the ordinary gate with no
Docker: it parses committed OpenROAD output (`oracle_faithful.txt`, `oracle_corrupt.txt`,
real stdout captured from the pinned image on 2026-07-07, see the fixture `NOTICE.md`) and
asserts the faithful counts agree with the import while a corrupted DEF (one component
deleted) diverges by exactly that component. The live container test runs OpenROAD over the
same LEF/DEF when Docker and the pinned image are present, asserts the faithful import
agrees and the corrupted DEF diverges, and returns `OracleOutcome::Skipped` with a
printable reason otherwise, so a machine without the image never fails the gate.

Unlike ADR 0054, whose live run was attempted but not completed, this lane's live container
cross-check **ran to completion on the development host**: the pinned image was present
(18.9 GB on disk, exact digest confirmed), and `container_cross_check_runs_or_skips_honestly`
ran OpenROAD over both the faithful and the corrupted DEF in about 22 seconds, agreeing on
the faithful design (macros 2, components 3, pins 2, die `0 0 20000 20000` DBU) and
diverging on the corrupted one (components 2). On a machine without Docker or the image the
same test skips honestly.

The fixtures are authored, not third-party: to stay inside what OpenROAD's strict reader
accepts they omit the optional `BUSBITCHARS`/`DIVIDERCHAR` header lines (OpenROAD rejects
the non-standard plural `DIVIDERCHARS` that appears in some tool output) and keep their
routing via-free (an undefined via is a hard OpenROAD error). `reticle-lefdef`, being
lenient by design, imports the same files without complaint, which is itself a small piece
of evidence: the two readers accept the same input and agree on what it contains.

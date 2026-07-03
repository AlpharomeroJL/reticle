# 0023, Resume orientation: docs/TASKS.md is the authoritative plan, the named v5 packet file is absent

## Context

The resume handoff for the v5.0.0 run names `reticle-v5-unified-packet.md` as the
authoritative plan and directs the run to follow that file's resume protocol. That
file does not exist anywhere in the repository. What does exist is
`reticle-autonomous-build-plan.md` (the original build plan, with the Appendix B
Windows and PowerShell rules the handoff also cites) and `docs/TASKS.md`, whose
opening "Resume protocol" (read this file, `git log --oneline -15`, and
`git worktree list`, then continue from the first unfinished item) matches the
handoff word for word. Halting to ask which document governs was not an option: the
run is unattended and the operating rules require resolving ambiguity and continuing.

## Decision

Treat `docs/TASKS.md` as the single authoritative run tracker and
`reticle-autonomous-build-plan.md` as the standing spec (crate responsibilities,
honesty rules, Appendix B). The absent `reticle-v5-unified-packet.md` is understood
to be an aspirational name for the same content that already lives in these two
files plus the handoff's own step list, which is itself a detailed and internally
consistent plan for closing Wave 2, running Batch 3 (lanes H and I), and executing
Wave 3 to the v5.0.0 release. The handoff's explicit step ordering governs sequencing
where TASKS.md is silent.

## Consequences

The run proceeds without a blocking gap, and a reviewer who goes looking for the
named packet finds this record instead of an unexplained detour. The risk is that the
absent file contained intent not captured in TASKS.md or the handoff; that risk is low
because the three sources agree on every concrete item (lanes, gates, release target)
and the handoff enumerates the remaining work directly. If the packet later surfaces,
reconcile it against TASKS.md at the next wave boundary and supersede this ADR.

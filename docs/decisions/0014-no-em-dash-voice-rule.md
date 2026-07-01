# 0014, Voice rule: no em-dashes, enforced by check-style

## Context

The repository should read as human-authored. A recurring tell of machine-written
prose is the em-dash (U+2014) used as a sentence break. To keep the voice consistent
and human, the project bans the em-dash character everywhere: docs, README, comments,
and doc-comments.

## Decision

Remove every em-dash from the tree and forbid new ones. Add a `just check-style`
recipe (scripts/check-style.ps1) that scans every tracked text file, read as UTF-8 so
multi-byte characters are seen correctly on Windows PowerShell 5.1, and fails with a
file and line list if any em-dash is found. Wire `check-style` into `just ci` so the
rule is enforced on every commit and wave merge. Prefer commas, colons, parentheses,
or restructured sentences in place of an em-dash.

## Consequences

- The initial sweep replaced spaced em-dashes with commas and any remaining em-dashes
  with hyphens across 61 files; the gate now reports zero.
- The check uses .NET UTF-8 line reading rather than `Select-String`, whose default
  encoding on PowerShell 5.1 misreads UTF-8 multi-byte characters.
- Hyphens (U+002D) and en-dashes are unaffected; only the em-dash is banned.

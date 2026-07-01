# 0001, Keep the working name "Reticle"

## Context

The spec ships under the working name **Reticle** (the patterned photomask master
used in chip lithography) and invites a rename before Wave 1 via a global
find-and-replace. The target GitHub repository is `AlpharomeroJL/reticle`. A rename
would touch every crate name, the workspace, docs, and the repo slug.

## Decision

Keep the name "Reticle". It is apt for an IC-layout editor, matches the intended
repository slug, and avoids a large, error-prone rename before the contracts are
even frozen. All crates use the `reticle-` prefix.

## Consequences

No rename churn. If a rename is ever desired it must be done with a workspace-wide
find-and-replace of `reticle`/`Reticle` and a repo transfer, ideally before any
external links (Pages, release) exist. That window closes at Wave 5, so the name is
effectively final once the release ships.

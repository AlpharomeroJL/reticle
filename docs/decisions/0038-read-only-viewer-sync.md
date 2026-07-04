# 0038, Read-only session viewers: live sync, an independent camera, and follow-mode

## Context

The wedge's whole differentiation is a one-click share link that gives a viewer a
**live, read-only** view of a session: the viewer sees the sharer's edits arrive in
real time and the sharer's cursor and selection, but never mutates the shared
document. Reticle already had every piece a live viewer needs, just not wired for a
read-only viewer: the relay (`reticle-server`) is a generic room broadcast that fans
every binary frame to a room's peers and replays a late joiner's backlog; the CRDT
(`reticle-sync`, ADR 0007) makes applying those frames idempotent and commutative;
`Presence` already carries a cursor, a selection, and a viewport (frozen proto field
6); and the public web bundle already opens to a start view chosen from a `?view=`
query (ADR 0026). The open questions were how a viewer joins *read-only* (a generic
broadcast lets anyone publish), how a viewer's camera relates to the sharer's, and
how a viewer can optionally "ride along" with the sharer's viewport.

## Decision

A read-only viewer is built on the existing public-visitor path, not a parallel
mechanism. The share link for a viewer is a **page** URL, `?view=viewer&room=<id>&relay=<host>`
(`reticle_app::share::viewer_link`), which the web bundle opens; the viewer then
dials the relay with a new read-only flag, `GET /ws/{room}?mode=view`
(`reticle_server::JoinMode::View`). Read-only is enforced on **both** sides: the app
never publishes (the viewer state machine has no send path), and the relay drops any
binary frame a `View` connection sends, so it is never logged and never broadcast.
The viewer logic lives in `reticle_app::viewer::ViewerSession`, window-free and
pure: `apply_frame` merges the sharer's raw `yrs` frames into a private
`SyncDocument` mirror, `apply_presence` records the sharer's cursor/selection/viewport,
and the viewer owns its **own** `ViewCamera` so it pans and zooms independently.
Follow-mode is a toggle: when on, `sync_camera` snaps the local camera to frame the
sharer's viewport via the pure `follow_camera(viewport, screen)` (which reuses
`ViewCamera::zoom_to_fit`, letterboxing to the viewer's aspect ratio so nothing the
sharer sees is cropped); turning it off leaves the camera where it is, so the viewer
resumes independent panning. The sharer's viewport travels on the already-additive
`Presence.viewport` field, so no proto change was needed; a `Presence::with_viewport`
builder and a round-trip test document that the viewport is the follow-mode channel.

## Consequences

A viewer is safe to hand to anyone: even a viewer that ignored the app and dialed the
relay directly cannot mutate the shared document, because the server drops its frames
(proven by relay tests that a viewer receives the sharer's frames but its own frame
reaches no peer and is absent from the replayed log). The follow-mode math and the
read-only sync are ordinary unit-tested code (`follow_camera` frames and centers the
sharer's viewport; the viewer applies frames idempotently, tracks a moving viewport
when following, and stays put when not), so the behavior is proven without a browser.
The `?view=viewer` link parses back with the same pure parser the desktop Share
section builds it with, so page and link agree on the format. What these Rust tests
cannot prove is the full two-context browser round-trip (a real sharer editing while
a second browser context follows live over the relay); that is the Wave 1 end-to-end
merge gate, and the web entry currently recognizes and reports a viewer link while
the live socket-pumping into a `ViewerSession` is what that gate drives. The cost is
a second, read-only actor id and mirror per viewer, and a viewer whose relay flag is
stripped by a hostile proxy would still be blocked only by the app-side never-publish
rule, which is why the server-side drop exists as the backstop.

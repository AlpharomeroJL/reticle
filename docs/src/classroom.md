# Classroom mode

Classroom mode layers a teaching workflow on the [collaboration](collaboration.md)
machinery: an instructor's roster shows who else is in the session, a student
can follow the instructor's viewport live, and the instructor can broadcast
their current view or release ("unlock") a student to work independently
again. It adds no new wire message and no `reticle-sync` field ([ADR
0111](../decisions/0111-classroom-mode.md)): the roster is built from the
existing `Awareness` presence map, and following rides the same live-published
`Presence.viewport` the read-only viewer already uses ([ADR
0038](../decisions/0038-read-only-viewer-sync.md)).

## Roles and the roster

`crate::classroom::ClassroomState` (`crates/reticle-app/src/classroom.rs`) is
egui-free: a roster of every other known peer, classified instructor or
student, each with a locally-tracked follow flag, plus the instructor's last
broadcast viewport. `sync_roster` rebuilds the roster from `Awareness` every
frame, reusing the same identity/color/name resolution
(`crate::viewer::participants`) the session chip's avatar row already uses, so
a classroom peer reads exactly like any other collaborator's presence, with a
role label layered on top.

## Following the instructor

A student's "Follow instructor" toggle does not add a second camera-follow
path: it flips the same `ViewerSession` follow flag the collaboration chapter's
session chip already drives, so the existing per-frame `sync_camera` snaps the
student's camera to the instructor's live viewport (ADR 0038). Turning follow
off leaves the student's camera where it is, free to pan and zoom
independently until they follow again.

## Bring everyone, and unlock

The instructor's **Bring everyone here** records their current camera viewport
as the broadcast target and marks every known student as following, so a
following student's next camera sync lands exactly there. **Unlock** (per
student row, or the palette's `classroom.unlock_student`, which targets the
first currently-following student in roster order) clears one student's follow
flag without touching anyone else's. Both are ordinary, pure state
transitions on `ClassroomState`, unit-tested against `Awareness` values built
directly in the test (there is no byte-shape contract fixture here, unlike the
F1-F6 producer/consumer pairs: nothing downstream depends on a frozen wire
record).

## What this depends on, honestly

Today the app publishes presence from exactly one identity
(`crate::livesync::SHARER_ACTOR`), and a read-only viewer never publishes at
all, by design (ADR 0038). That means an **instructor's** live roster is
genuinely empty until a future lane wires a write-capable "join and publish my
own presence" path; the classroom panel renders an honest empty state naming
this rather than a fabricated row. A **student's** half already works end to
end over whatever relay is configured, because it only depends on the
instructor's already-flowing viewport. Either way, a classroom that spans more
than one machine still needs a reachable relay: the share server default stays
`127.0.0.1:3030` (`crate::share::DEFAULT_SERVER`), and a deployed public relay
is operator-owned, tracked as backlog item H1
(`scratch/campaign/v82-backlog.md`). This module does not change that default
and does not attempt to work around it.

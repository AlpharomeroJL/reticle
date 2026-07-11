# 0111, Classroom mode: roster and follow over the existing presence channel, no new wire message

## Context

Phase 3 asks for a teaching workflow over the multi-writer collaboration relay: an instructor
broadcasts their current view, students can follow it live, and the instructor can release
("unlock") a student to work independently. `reticle-sync::presence` already carries a
per-actor `Presence` (cursor, selection, viewport) and an `Awareness` map merging them
(ADR 0007), and the read-only viewer (ADR 0038) already snaps its camera to a followed
sharer's live-published viewport. But the app today publishes presence from exactly one
identity, `SHARER_ACTOR` (`crate::app::App::local_presence`), and a read-only `ViewerSession`
never publishes at all by design ("the app never publishes", ADR 0038): the wire has no
directive channel (`Frame` is `Update | Presence | Comment`, `reticle.proto`) and no spare
`Presence` field for a per-student instruction. Extending the wire is out of this lane's owned
paths (`reticle-proto` is a separate, frozen crate), and the brief's own ledger prefers an
app-side model over a `reticle-sync` change when the roster and follow-state can be built on
what already exists.

## Decision

`crates/reticle-app/src/classroom.rs` adds an egui-free `ClassroomState`: a roster derived
from `Awareness` (reusing `crate::viewer::participants` for identity, color, and name
resolution, exactly as the session chip already does), each known actor's local follow
bookkeeping, and the instructor's last broadcast viewport. `bring_everyone` marks every known
student as following and records the instructor's current viewport; `unlock_student` clears
one student's flag; both are pure and directly tested against constructed `Awareness`
fixtures (no contract fixture, per the brief: there is nothing to freeze a byte shape for). The
three F6-reserved commands (`classroom.bring_everyone`, `classroom.follow`,
`classroom.unlock_student`) move from `RESERVED_CAMPAIGN_IDS` into `REGISTRY` (ADR 0106) with
no chord, matching every prior F6-shipping lane. A student's "Follow instructor" toggle does
not duplicate state: it flips the same `ViewerSession` follow flag the existing session-chip
checkbox already drives, so the existing per-frame `sync_camera` (ADR 0038) stays the one and
only camera-follow code path. A single marked hook in `app.rs` (`App::classroom_panel`) renders
the roster in a small window, shown only when the session is classroom-capable (this app went
live as a sharer, or is a read-only viewer), so an ordinary offline session never sees it.

## Consequences

The student half of classroom mode is real today: a student who toggles follow rides the
instructor's live viewport exactly as an ADR-0038 viewer already does, over whatever relay is
configured (the share server default stays `127.0.0.1:3030`; a deployed public relay is
operator-owned, tracked as backlog item H1, `scratch/campaign/v82-backlog.md`). The instructor
half is honest about a real gap: because only one identity ever publishes presence today, an
instructor's live roster is empty until a future lane wires a write-capable "join and publish
my own presence" path (a read-only viewer, deliberately, never will). `bring_everyone` and
`unlock_student` are fully implemented and tested against that future roster shape, so nothing
here needs to change when that path lands; only `sync_roster`'s input grows. The roster panel
renders an honest empty state naming this rather than a fabricated row. The
`reserved_campaign_ids_are_well_formed_unique_and_disjoint_from_the_registry` test in
`commands.rs` enforces that the three moved ids are no longer reserved, so this cannot drift
back out of sync with `REGISTRY`.

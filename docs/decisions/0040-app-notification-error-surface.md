# 0040, One app-level notification surface every failure path reports through

## Context

Reticle grew several independent failure paths that a user could not see. An import
could reject a file, an opened file could carry non-fatal warnings, a dropped file
could have an unreadable path or an unknown extension, and the example gallery could
in principle fail to load a compiled-in design. Some of these were silent, some were
only on the console, and the one visible surface (the import-warnings window) covered
just warnings from a successful open, not hard errors. First contact is exactly where
a silent failure is most damaging: a new user drops a file, nothing happens, and there
is no explanation. Three lanes were also converging on this area at once (the Start
experience, browser-file open, and share), and without a shared sink each would invent
its own way to tell the user something went wrong.

## Decision

Add one pure, egui-free notification model in `reticle-app` (`crate::notify`): a
`Severity` (Info, Warning, Error), a `Notification` (severity, one-line summary,
optional longer detail, and an age counter), and a bounded `Notifications` queue that
pushes, ages, auto-expires non-error toasts, keeps errors until dismissed, and caps
its length so a burst cannot grow it without bound. The `App` owns one queue and
exposes the sink other code routes through: `report_error(summary, detail)` for a hard
failure and `notify(summary, detail)` for a neutral notice, plus `notifications()` to
read it. A thin egui toast area draws the queue over everything, colored by severity,
with a close button; the frame loop ages it each frame. The existing open paths route
through it: `open_outcome` posts an info notice on a clean open and a warning per
import warning, and the new `open_bytes_reporting`/`open_example_chip` wrappers report
an `OpenError` as an error notification instead of returning a `Result` the egui glue
would have to thread. The model carries no `egui`, GPU, or platform types, so it is
unit-tested in plain code and compiles unchanged on wasm.

## Consequences

Every failure path that routes through `report_error` is now visible and consistent,
and the sink is a clean public method sibling lanes call at merge (a share link that
cannot be formed, a browser file that will not open) without touching the toast
drawing. The behavior is proven without a window: the notify tests trigger an error
and assert a severity-tagged notification is queued, that non-errors expire while
errors persist, and that the queue is bounded; the app tests feed non-GDSII bytes and
assert an error toast naming the source is queued and the editor is left untouched.
The cost is a deliberately small amount of policy in one place (the auto-dismiss
window, the queue cap, the severity-to-color mapping), and paths that already own a
dedicated human-readable surface (the query bar's inline error, the operations panel
status, the replay-load error, the technology editor's own messages) are left on those
surfaces rather than also duplicated as toasts, so this ADR does not claim to have
funneled literally every message through one widget, only every otherwise-silent
failure.

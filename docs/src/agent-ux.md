# In-app agent UX

The editor's agent panel is a preview: it narrates a fixed, scripted
propose-verify-correct run on a built-in demo cell, with no model and no API key (see
[Agent API and harness](agent.md) for how a real run works). It illustrates the
plan/approve/execute agent planned for a later release; it does not read or edit your
open design or its DRC results. On top of that narration the editor adds two
affordances that make the preview feel like a collaborator rather than a batch job: a
conversation you can steer mid-run and a browser for reopening past runs.

Everything here is UI-side. The panel logic lives in
[`agent_panel`](https://docs.rs/reticle-app) and the history browser in
`agent_history`, both window-free and unit-tested without an egui context; the `app`
module owns only the thin drawing glue. Both features build and run on both
native and `wasm32-unknown-unknown`, with the one filesystem-touching seam (scanning
for past transcripts) guarded behind `cfg` with a clean bundled fallback in the
browser.

## Conversation mode

The agent panel keeps a conversation transcript alongside the raw narration feed: a
list of turns, each authored by either the user or the agent. Starting a run opens
the conversation with the prompt as the first user turn. While the run is active, a
follow-up box lets you send an additional instruction or constraint; submitting it
appends the message to the conversation as a user turn, adds an agent
acknowledgement, and records the instruction on the panel's follow-up list.

That follow-up list is the honest seam. On the UI side today the acknowledgement is
scripted, because the panel narrates a recorded transcript rather than driving a
live model. A live scoped harness (Wave 3) reads the follow-up list and forwards
each instruction to the model as a new constraint on the running session; the UI
records them regardless, so the affordance is real before that harness exists.

A follow-up is only accepted while a run is active (an instruction has no session to
attach to otherwise) and only when its trimmed text is non-empty. The conversation
transcript is distinct from the engine transcript: a conversation entry is
human-facing text, not a replayable command. It is capped so a long session drops
its oldest turns rather than growing without bound, and starting a fresh run clears
it.

Each verify the run crosses is also surfaced as an agent turn (`verified: DRC clean`
or `verified: N violation(s) remaining`), so the conversation reads as a dialogue
that tracks the propose-verify-correct loop and not just a command log. Replaying a
transcript in the theater does not write into the panel's conversation; only the
panel's own run does.

## Session history browser

A finished agent run leaves a `*.transcript.jsonl` file next to its other artifacts
(`reticle-agent` names them `<task-id>.transcript.jsonl`). The history browser lists
those transcripts so you can reopen a past run with one click, loading it straight
into the [replay theater](agent.md) through the same
[`store`](https://docs.rs/reticle-app) seam the theater already loads through.

Listing is on demand: pressing Refresh scans, and drawing never touches the disk, so
the browser costs nothing per frame. The platform difference is the browser's only
`cfg`:

* Native scans a directory (the conventional `runs/` by default, editable in the UI)
  for `*.transcript.jsonl` files.
* wasm has no filesystem, so the browser lists the single bundled demo transcript the
  theater already carries, and selecting it plays that.

The interesting part, turning a set of file names into a sorted, labelled entry list,
is the platform-free `entries_from_names`: it keeps only the transcript files, labels
each by its task id (the base name with the suffix and directory stripped, for either
path separator), sorts by label, and collapses duplicates. It is unit-tested over a
synthetic listing with no disk touched.

## Preview status

The agent panel is a preview of a planned capability, not a working agent. It runs a
fixed scripted propose-verify-correct loop on a built-in demo cell so you can see the
shape of the interaction (plan, narrated steps, verify results, replayable
transcript). It does not read or edit your open design, and it does not write into the
DRC panel or the canvas markers, which track your real layout. A real
plan/approve/execute agent over the editor's command tools is planned for a later
release; when it lands, this panel becomes its front end.

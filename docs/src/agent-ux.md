# In-app agent UX

The editor's agent panel narrates a real propose-verify-correct run without a model
or an API key (see [Agent API and harness](agent.md) for how the run itself works).
On top of that narration the editor adds three affordances that make the agent feel
like a collaborator inside the tool rather than a batch job: a conversation you can
steer mid-run, a browser for reopening past runs, and a one-click "ask the agent to
fix this" from a design-rule violation.

Everything here is UI-side. The panel logic lives in
[`agent_panel`](https://docs.rs/reticle-app) and the history browser in
`agent_history`, both window-free and unit-tested without an egui context; the `app`
module owns only the thin drawing glue. All three features build and run on both
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

## Ask the agent to fix a violation

Selecting a violation in the DRC panel reveals an "Ask agent to fix" button. It
assembles the violation's region and the rule it broke into a scoped context string
and launches an agent run seeded with it. The context pins the agent to an objective
target and a bounded area rather than the whole design:

* the region, as the violation's bounding-box corners in DBU plus its width and
  height;
* the rule, by name and kind, the layer (or two layers, for spacing, enclosure, and
  extension rules), and the measured-versus-required values;
* the original violation message as trailing context.

The kind is tagged with the same keywords the agent API's `run_drc` reports and
parses, so a scoped fix names constraints the way the checker does.

### The Wave-3B seam

The MINIMAL context pack and the real *scoped* harness (which would clip the session
to the violation's region and constrain the agent's edits to it) are Wave 3 Lane 3B.
What ships here is the UI affordance and the assembled context string that harness
consumes. Assembling the string
([`drc_panel::fix_violation_prompt`](https://docs.rs/reticle-app)) and handing it off
(`App::ask_agent_to_fix`) are separated deliberately: today `ask_agent_to_fix` seeds
the agent panel's prompt and starts the ordinary narrated run, and a Wave-3B harness
replaces only that hand-off with a scoped-session launch that reads the same context.
Everything upstream, the DRC button and the assembled string, stays unchanged. The
seam is honest about being a seam: the scoped run is not yet clipped to the region;
it is the same model-free narrated run pointed at the violation's context.

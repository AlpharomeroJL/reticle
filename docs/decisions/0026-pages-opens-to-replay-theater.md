# 0026, The public Pages bundle opens to the replay theater

## Context

Deliverable 3 of Lane 2H makes the replay theater the default view a public
visitor sees when they open the deployed web bundle (the `crates/web` Trunk build,
which mounts `reticle-app`). The replay theater (Lane 2D) plays a recorded agent
transcript back through a live session, so it is the most legible first impression
for a drive-by visitor: motion, DRC overlays, and the agent narrating, with no
setup and no API key.

Today `reticle-app` opens with the interactive editor and the replay theater as a
window a user must open (`replay_open` starts `false`). The theater and the agent
panel are also gated to native builds (`#[cfg(not(target_arch = "wasm32"))]`),
because the agent-run path pulls the blocking model client. But the *replay* path
does not need the model at all: it materializes a recorded transcript through a
`Session`, which is wasm-buildable (Lane 2D already made `reticle-agent-api`
wasm-buildable, commit 5a7b5ff).

Two ways to make the theater the default were considered:

- flip the app's initial state so `replay_open` starts `true`. Simple, but it
  changes the default for the native app too and hard-codes a policy into the app.
- let a visitor (and the release step) request the theater without changing the
  editor's native default, via a signal the web mount reads.

## Decision

Make it a *requested* mode the web layer opts into, not a new hard-coded default:

- `reticle-app` gains a small startup-intent seam: `App::with_start_view(StartView)`
  alongside `App::new()`, where `StartView::Editor` is today's behaviour and
  `StartView::ReplayTheater` opens the theater window on the first frame and loads
  the built-in scripted demo run so there is something playing immediately. The
  replay theater and its state are compiled on wasm too (the replay path is
  model-free); only the live agent-run controls stay native-gated.
- The web mount (`crates/web`) reads the desired start view from the page URL:
  `?view=replay` (or the default when the bundle is published as the public demo)
  selects `StartView::ReplayTheater`. `index.html` for the published bundle points
  the visitor at the theater, and a small on-page control links back to the full
  editor (`?view=editor`).

The Wave 3 release step publishes `crates/web`'s Trunk `dist/` to `gh-pages`; that
publish is documented in the deployment doc, and this lane does the code and
config so the published default is the theater. This lane does not touch gh-pages.

## Consequences

- The start view is data the entry point chooses, not a policy baked into the app,
  so the same app binary serves both the editor and the public demo framing. The
  native app is unchanged (it defaults to the editor), and
  `App::with_start_view(StartView::ReplayTheater)` opens the theater directly with
  the built-in scripted run loaded.
- The web mount reads `?view=` (defaulting a public visitor to the theater) and the
  `index.html` frames the theater with an always-visible link to the full editor
  (`?view=editor`) and back, so the published bundle presents the theater as the
  landing experience.
- Honest gap, marked `TODO(wave3)`: the replay theater window is still native-only
  today (its window glue and the scripted-run generator live in the native-gated
  `agent_panel`/`replay` modules). On the wasm build the start view is recorded and
  the framing is in place, but the in-page theater window opens only once those
  modules are un-gated for wasm (they are model-free, so the work is decoupling them
  from the native agent-run path, not new functionality). Until then the wasm bundle
  honours `?view=` at the entry point and shows the demo scene behind the theater
  framing; the native app opens the theater fully.
- No secret and no network are involved: the theater replays a committed scripted
  transcript, so the public demo needs no API key.

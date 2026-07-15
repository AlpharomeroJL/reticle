# 0124, Wave B fixes the three derived-view lesions pointwise; owned invalidation and a fuller browser harness are deferred

## Context

The v8.2.1 Wave A defect map (`scratch/findings/DEFECT-MAP.md`) traced the reported
symptoms (an example loading into a still-bound archive, selection highlighting geometry
that is not on screen, a mislabeled or unlistable layer, batch DRC verifying the wrong
document) to three root causes, all instances of one architectural gap. The app derives
several views of the current document, the pick index (`SceneIndex`), the GPU draw scene
(`RetainedScene`), the layer table (`LayerState`), and the archive tile browser
(`self.archive`), and keeps them in agreement by hand-written imperative invalidation
triggered on specific events. The trigger coverage was inconsistent: `install_document`
did not exit archive browse or clear several old-document caches (RC1), pick had no
visibility concept while the renderer did (RC2), and the layer table was rebuilt only on
document load, never on an edit (RC3). No invariant or type owned the agreement.

A second gap made these shippable: the app had no browser-level test. 241 unit tests, a
KLayout oracle, golden pixel tests, and two-way checkers all ran without ever loading the
app in a browser and driving it, so a defect that only manifests in what the live app
paints and enables after a real in-session open passed every gate.

## Decision

Fix the three root causes pointwise now, and defer the architectural change and the fuller
harness, rather than blend them into this wave.

1. **Ship three targeted fixes**, each pinned by a committed red-to-green test:
   a commit-zero honesty guard (batch DRC and SPICE export report unavailable while
   browsing a streamed archive, since there is no editable document to verify), RC1
   (`install_document` exits archive browse and completes its reset list), RC2 (pick,
   hover, and double-click-fit ignore hidden layers), RC3 (`rebuild_scene` re-derives the
   layer table for edited geometry). e2e-revive revived the headless Playwright gate,
   wired `e2e-ci` into `just ci`, and added the RC1 browser-level doc-switch acceptance
   test, the first browser regression guard in the project.

2. **Defer the architectural fix** to a next campaign, `arch/owned-invalidation`: a
   document-scoped state container, or a single revision/epoch every derived view must gate
   on, so a newly added derived view cannot silently go stale. Patching three lesions
   leaves the class intact, and every derived view added from here re-enters the same
   lottery; but the owned-invalidation change is broad and riskier than the three scoped
   fixes, so it is sequenced after them, with the Wave B tests as its regression safety
   net, not folded into this wave.

3. **Defer a fuller browser harness** as a follow-on to e2e-revive: one that loads the
   deployed bundle and exercises document switch, zoom, pick, and verify across every
   start-screen example. The doc-switch test is the first step; the full harness would have
   caught every root cause in this map.

Both deferred items are recorded in `docs/honest-limits.md` (App/UI-gaps) so the gaps are
visible, not silently dropped.

## Consequences

The three symptoms are fixed and guarded by tests that go red without their fix. The
shared root cause is documented and owned by a named next campaign rather than implied.
The honest position is that Wave B removed the three known lesions but not the class:
until owned invalidation lands, a new derived view still depends on a developer wiring its
invalidation into the right trigger by hand. The Wave B tests bound the regression surface
for that follow-on.

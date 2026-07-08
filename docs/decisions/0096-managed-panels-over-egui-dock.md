# 0096, managed panels over egui_dock

## Context

The packet asks whether egui_dock can host the side panels and the floating 3D
stack and Cross-section windows alongside the custom split-view system
(viewports.rs). The audit (audit.md AUD-01, AUD-02) shows the floating windows
occluding canvas content and the streaming HUD at first paint. egui_dock 0.20.1
was surveyed: it is egui ^0.35 compatible, offers DockState, DockArea
show_inside, and TabViewer, with serde-optional persistence. The original plan
timeboxed a half-day spike; the wall-clock revision pre-decides instead, because
the survey already exposes the deciding facts and managed panels were always the
fully acceptable fallback.

## Decision

Ship managed panels; decline egui_dock this packet. Deciding facts:

- Persistence simplicity: reticle-app is deliberately no-serde. Persisting a
  DockState (tree of surfaces, nodes, tabs) would need a hand-rolled key=value
  walk of a foreign recursive structure; persisting managed panels is four
  scalars and a csv of collapsed flags in the existing session format.
- Dependency budget: the packet already adds fonts, icons, kittest, and rfd;
  a layout engine that egui panels can approximate is not worth its surface.
- The north star wants fewer floating layers, not rearrangeable ones: the
  occlusion class of bugs is eliminated by construction with docked panels and
  an overlay layout manager, which docking would not give for canvas overlays.

Shipped design: egui::Panel left/right (resizable, widths persisted); the right
panel's fourteen stacked sections become a segmented control over four groups
(Inspect, Review, Automate, Settings) of token-styled collapsible sections; the
3D stack and Cross-section become managed panels opened from View > Panels and
never overlap rulers or canvas controls.

## Consequences

- Lane 2C's brief is written directly against this design; no spike time is
  spent.
- Users cannot rearrange panels into custom layouts this release; that is the
  trade for zero-occlusion guarantees and tiny persistence. If docking earns
  reconsideration it is a future rider with this ADR as the baseline, not a
  revisit inside this packet.
- The split-view system (viewports.rs) is untouched: it subdivides the canvas
  rect, which stays a CentralPanel affair regardless of side-panel management.

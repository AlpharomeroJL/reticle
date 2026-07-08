# UX audit of the v8.0.0 interface (before the v8.1 redesign)

Method: heuristic evaluation over the deployed bundle `web-cc73d6608fe18660`
(captures in `baseline/`, manifest there records provenance) plus code inspection
of `crates/reticle-app`. Heuristics: Nielsen's ten (tagged N1..N10) plus the eight
packet principles (tagged P1..P8: canvas first, progressive disclosure, one visual
truth, keyboard-first twice-discoverable, density with hierarchy, functional
motion, proven contrast, viewer/editor separation). Interior states that need
in-app interaction cite code; their screenshots backfill from the v8.0.0 worktree
in the Wave 5 capture queue.

Severity: HIGH = misleads, blocks, or occludes; MED = friction or inconsistency;
LOW = polish. Every finding carries its owning disposition (lane or catalog item)
so the Wave 5 audit can close the loop.

## HIGH

- **AUD-01 (P2, N8) Floating windows open over canvas content at first paint.**
  The default landing shows the Replay theater window plus the collapsed 3D stack
  and Cross-section title bars stacked over the canvas and each other at fixed
  `default_pos` values. Evidence: `home-default--1280x800.png`;
  `view3d.rs:167`, `xsection.rs:309`, `app.rs:4573`. Disposition: 2C (managed
  panels), 3A (overlay manager), ADR 0096.
- **AUD-02 (P1, N1) The streaming HUD is occluded by the collapsed windows.** In
  `?archive=` mode the HUD's fetched/residency lines are covered by the 3D stack
  and Cross-section bars: the flagship demo hides its own proof-of-streaming.
  Evidence: `archive-stream--1280x800.png` (HUD lines legible only around the two
  bars). Disposition: 3A overlay layout manager (collision-free by construction).
- **AUD-03 (P8, N2) Viewer mode is the editor in disguise.** A share-link viewer
  gets the full editor chrome: History with a debug button, DRC, Layout diff,
  Comments, Agent, draw tools. The share/follow controls live below the fold of
  the right-panel scroll. Evidence: `viewer-empty-room--1280x800.png`.
  Disposition: 2D viewer chrome (catalog 23).
- **AUD-04 (N1, N5) Read-only modes show stale editor state.** In archive and
  viewer modes the Layers panel lists the built-in demo document's layers and
  History reports "Scene shapes: 309" while an unrelated die streams; "Add demo
  rectangle" stays clickable. Evidence: `archive-stream--1280x800.png`,
  `viewer-empty-room--1280x800.png`. Disposition: 2C (layers source of truth),
  2B (mode-aware sections), 2A (debug relocation, catalog 70).
- **AUD-05 (P4, N6) No menu bar; one wrapped toolbar holds everything.** Roughly
  25 controls of four widget kinds (selectable labels, buttons, checkboxes,
  segmented-ish labels) wrap into two uneven rows with no grouping, no icons, no
  overflow rule, and no shortcut hints. Convert-GDS, a file action, sits between
  tool selection and Fit. Evidence: `home-default--1280x800.png` top rows;
  `app.rs:2414`. Disposition: 2A (menu bar + grouped toolbar), 1E (registry).
- **AUD-06 (N8, P5) The right panel is a 14-section mega-scroll.** Properties,
  DRC, Diff, Comments, Agent, then Operations, Productivity, Generate, Snap,
  View and export, Search, Technology editor stacked in one scroll with large
  blank stretches; low-frequency sections push high-frequency ones apart; nothing
  collapses. Evidence: `home-default--1280x800.png` (dead space between DRC and
  Layout diff); `app.rs:2697-2747`. Disposition: 2B Inspector rebuild.
- **AUD-07 (N4, P3) No hover, focus, or pressed styling exists anywhere.** The
  app never touches `Style`/`Visuals` beyond stock dark/light, so keyboard focus
  is invisible and interactive affordance is flat. Code fact: zero `style_mut`/
  `visuals_mut`/`set_fonts` calls; `app.rs:7200` is the only styling line.
  Disposition: 1A tokens, 4A states, 3C focus traversal (catalog 83).
- **AUD-08 (N1) Ruler labels collide into a smear.** At common zooms the top
  ruler prints overlapping tick labels near the origin corner (unreadable strip
  in every desktop capture). Evidence: `home-default--1280x800.png` top edge.
  Disposition: 3A rulers (catalog 31).
- **AUD-09 (N7, P4) Debug-register copy in the primary surface.** "Add demo
  rectangle" is the first button of the right panel in every mode; tool labels
  like "Vertices" and "Cut" sit unexplained beside file actions. Evidence:
  `home-default--1280x800.png`; `app.rs:3263`. Disposition: 2A Help > Developer
  (catalog 70), tooltips (catalog 25).
- **AUD-10 (N1) Blank boot.** The wasm init shows a bare loading overlay around
  0.6 s with no progress or tip; first paint arrives unannounced. Code fact:
  `crates/web/index.html` #overlay; measured cold load in docs/PERF.md.
  Disposition: 3A splash (catalog 97).

## MED

- **AUD-11 (N6) Start screen cards are undifferentiated gray strips.** Full-width
  boxes, small right-aligned Load/Start buttons, no thumbnails, no
  technology/size/license badges, no streaming demos; "Skip to the editor" clips
  off the bottom at 1280x800. Evidence: `view-editor--1280x800.png`.
  Disposition: 2D start screen (catalog 14, 16, 96).
- **AUD-12 (P4) The palette is a plain floating list.** Fixed default position,
  no shortcut hints, no grouping or recents, no argument commands; it is the only
  home for several actions (nothing is menu-discoverable). Evidence:
  `app.rs:5294`, `command.rs`. Disposition: 3C (catalog 79, 80), 2A menus.
- **AUD-13 (P4) The shortcuts window is a rebind editor, not a reference.** No ?
  overlay, not generated from a registry, hidden behind a toolbar button.
  Evidence: `app.rs:5344`. Disposition: 3C (catalog 18).
- **AUD-14 (N9, N1) Failures and long tasks surface as quiet text.** Errors stack
  as plain notification lines without Retry/Copy actions; the viewer's reconnect
  state is tiny status-bar text ("reconnecting to the shared session (attempt
  3)..."); open progress is a bare window without stages or cancel. Evidence:
  `viewer-empty-room--1280x800.png` status bar; `notify.rs`, `app.rs:1441`.
  Disposition: 3B toasts + long-task pattern (catalog 71, 72, 73), presence chip
  (catalog 74, 75).
- **AUD-15 (N4) Widget styles mix without hierarchy.** Selectable labels, push
  buttons, and checkboxes sit adjacent in the toolbar and panels; primary
  actions (Run DRC, Share this session) look identical to Clear. Evidence:
  captures; disposition: 1C component library, 2A/2B adoption.
- **AUD-16 (N1) Status bar readouts are passive debug text.** fps with
  milliseconds always on; zoom in px/DBU only; no click affordances; no
  selection summary. Evidence: capture bottom edges; `app.rs:5263`.
  Disposition: 3A actionable status bar (catalog 32, 55, 68).
- **AUD-17 (N6) Minimap has no interaction and a fixed corner.** No draggable
  viewport rectangle, no click-to-jump, no residency shading while streaming; it
  also crowds the top ruler's right end. Evidence: `home-default--1280x800.png`
  top right; `minimap.rs`. Disposition: 3A (catalog 30).
- **AUD-18 (N10, P4) No tooltips with names and shortcuts.** Toolbar controls
  rely on their short labels; complex tools (Vertices, Cut line) get no one-line
  description; nothing shows its chord. Code fact: no `on_hover_text` on toolbar
  controls in `app.rs:2414` region. Disposition: 1B IconButton spec + 2A
  adoption (catalog 25).
- **AUD-19 (N5) Empty states name the void, not the next action.** "No
  selection", "No comments", "Not run", "Files you open will appear here" with
  no verbs or shortcuts. Evidence: captures + panel code. Disposition: 1C
  empty-state block, adopted by 2B/2D/4C (catalog 16, 17).
- **AUD-20 (P8) The default public view is the replay theater inside full editor
  chrome.** First-time visitors get an empty theater window titled "Nothing drawn
  yet" floating over an unexplained editor. Evidence:
  `home-default--1280x800.png`. Disposition: 2D start/viewer IA with 4C tour
  (catalog 15, 16, 20).

## LOW

- **AUD-21 Split mode is three always-visible labels** (Single, Split H, Split V)
  spending toolbar width; a segmented control with icons reads faster.
  Disposition: 2A (1C segmented component).
- **AUD-22 Path options cause toolbar layout shift** (width/endcap appear inline
  when Draw path is active, shoving neighbors). Disposition: 2A grouped toolbar.
- **AUD-23 The stock light theme is incoherent with the canvas-first dark
  chrome** and doubles the visual surface untested. Disposition: removed this
  packet per tokens.md; honest-limits entry at release.
- **AUD-24 Window drag can bury windows under panel edges** and positions do not
  persist across reloads on web (egui memory only). Disposition: 2C managed
  panels + session persistence.
- **AUD-25 The tour highlights rectangles but the copy is terse** and there is no
  viewer-variant tour; completion state is a single seen-flag. Code:
  `tour.rs`. Disposition: 4C rebuild (catalog 15).

## Cross-cutting facts the redesign builds on (verified in code)

- Styling is greenfield: one `set_visuals` call, ~77 `Color32::from_rgb*`
  literals, ~12 size literals, no fonts, no icons (`theme/` does not exist yet).
- The keymap (14 actions, rebindable, TOML) and palette catalog (fuzzy filter)
  are real registries to build on; effects already funnel through `run_command`.
- The e2e seams (`__reticle_stats`, query params, console signals) are healthy
  and must survive unchanged; they are gate canaries, not audit findings.

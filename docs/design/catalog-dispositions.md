# Catalog dispositions: the v8.1 completeness gate

Every numbered item of the packet's Improvement Catalog (Appendix A, 100 items),
walked one by one, with a disposition and a committed evidence pointer. This is
the sweep that closes the completeness contract in brief.md: a silent drop is a
sweep failure, and a missed P1 without a recorded genuine blocker is a
release-hold condition.

Method. Every SHIPPED row is backed by a
committed artifact a reader can open: a command id in
`crates/reticle-app/src/commands.rs`, a module or symbol under
`crates/reticle-app/src/`, a bench row in `benches/history/baseline.json`, a
token in `docs/design/tokens.md`, or an ADR under `docs/decisions/`. Where a
claim looked soft, the merged code was grepped and the
finding recorded here (item 38 is the clearest example: the Settings toggle
exists but the canvas does not yet consume it).

Disposition vocabulary:

- **SHIPPED**: the item's capability is present in the merged, deployed product.
- **SHIPPED (partial)**: the item's primary capability is usable; a named
  sub-feature is ledgered with a reason. Counts as shipped in the tally.
- **LEDGERED**: the item is not in the product; the reason is concrete. For P2
  and P3 items this is a schedule or seam ledger, mostly the pre-authorized 3A
  tail plus cross-crate seams frozen this packet.
- **LEDGERED (partial)**: the defining capability is deferred; some groundwork
  (a tested model, a persisted field, a partial UI) shipped. Counts as ledgered.

## POTENTIAL HOLDS

**None.** All 59 P1 items are shipped. No P1 is a silent drop, and no P1 is
deferred without a recorded reason, so there is no P1 release-hold condition.

Three P1 items shipped with a documented deviation from the exact catalog
wording. None removes the P1 capability; each is recorded here so the call is
the reader's, not hidden:

- **35 (permalink at current view + shortcut).** The command
  `share.copy_permalink` and its effect ship and are menu and palette
  discoverable; the default chord is intentionally left unbound per
  ia-inventory section 4 and is user-assignable in the shortcut editor. The
  "copy permalink at current view" capability is present; only the pre-bound
  key is absent by design.
- **63 (DRC violations navigable: click-zoom, n/N, stale indicator).**
  Click-zoom, previous/next cycling, the stale indicator, and the empty state
  all ship. The `n`/`N` key pair is not bound because `N` is already
  `view.minimap`; the previous/next buttons deliver the same cycling. Capability
  present, one key pair reassigned to buttons.
- **66 (agent transcript: collapsible steps, status icons, stop, scrubber).**
  Instant stop, the run-state status line, sample prompts, the collapsible
  plan/conversation/history sections, and the replay scrubber all ship. The
  deeper per-command collapsible-step timeline with a status icon per command is
  only partially realized through the existing plan and history views. The
  controllable, navigable transcript is present; the per-step icon refinement is
  ledgered.

## Summary tally

| tier | items | shipped | ledgered | rejected |
|---|---|---|---|---|
| P1 | 59 | 59 | 0 | 0 |
| P2 | 36 | 19 | 17 | 0 |
| P3 | 5 | 3 | 2 | 0 |
| **total** | **100** | **81** | **19** | **0** |

Shipped includes 6 P2 shipped-with-partial (3, 34, 49, 54, 56, 57) and the 3 P1
deviations above. Ledgered includes 4 partials where a tested model or persisted
field shipped but the surfaced capability did not (9, 31, 38, 69). No numbered
Appendix A item is rejected; the packet's rejected design directions (light
theme, docking, i18n, and the rest) are separate from the catalog and are listed
at the end of this file.

Of the 17 P2 ledgers, 11 are the pre-authorized 3A P2/P3 tail named in
catalog-map.md (31, 33, 37, 38, 41, 50, 58, 76, 77, plus 34 and 49 shipped in
part); the remainder are cross-crate seams frozen this packet (multi-document,
PWA manifest, presence/relay broadcast, comment-to-toast wiring) or the
deferred capture-track item 95.

## A. Opening and files

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 1 | P1 | 3B | SHIPPED | `file.open_dialog` (Ctrl+O) in commands.rs; `rfd` picker on native and wasm feeding `dialogs.rs`; bundle-ledger v8.1-wave2 row records the +124 KiB rfd cost |
| 2 | P1 | 3B | SHIPPED | `file.open_url`; `dialogs::validate_open_url` and `cors_diagnostic` (pure, tested) |
| 3 | P2 | 3B | SHIPPED (partial) | `handle_paste_to_open` in app.rs opens a pasted URL; raw file bytes are not exposed to a web page by the clipboard API, so the file half stays drag-and-drop (documented, not a schedule ledger) |
| 4 | P2 | 3B | LEDGERED | the editor is single-document; a per-file result list has no multi-target to open into. Deferred until multi-document exists |
| 5 | P1 | 3B | SHIPPED | full-window drag overlay with `dialogs::SUPPORTED_FORMATS` hint |
| 6 | P1 | 3B | SHIPPED | `dialogs::OpenStage` (pure, tested) drives the staged, cancelable progress card (parse, tessellate, upload) |
| 7 | P1 | 3B | SHIPPED | post-open info toast with shape/layer count, bbox, and a Fit action |
| 8 | P1 | 3B | SHIPPED | `dialogs::unsupported_file_diagnostic` and `open_error_diagnostic`: cause, format list, convert suggestion |
| 9 | P2 | 2D | LEDGERED (partial) | `startscreen::RecentPins` pin/unpin ships in-session; the cached render thumbnail, its IndexedDB persistence, and pin persistence are ledgered (the frozen 1B recent-files model stores a label, not a cache) |
| 10 | P2 | 2D | LEDGERED | session restore needs persisted document bytes; the 1B recent store keeps labels, not bytes, and no document-persistence seam is owned here. Recent-file reopen plus `?gds=`/permalinks cover the practical need |
| 11 | P2 | 3B | LEDGERED | PWA file handlers need `file_handlers` in the web manifest and a launch-queue consumer in `crates/web`, outside this crate. The receiver `open_named_bytes` is ready |
| 12 | P2 | 2B | SHIPPED | `file.export_png`, `file.export_svg`, `file.export_metrology` consolidated in the Export section and File > Export |
| 13 | P3 | 3B | LEDGERED | save-back via File System Access is web-only and needs an export-to-original round-trip the editor does not have yet |
| 14 | P1 | 2D | SHIPPED | `startscreen::{GalleryCard,GALLERY}`, `start_gallery_section`, per-card badges (name, tech, size, source, license) |

## B. Onboarding and first contact

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 15 | P1 | 4C | SHIPPED | `tour.rs` `TourVariant` (Editor and Viewer); `help.tour`; viewer variant `VIEWER_STEPS` |
| 16 | P1 | 2D | SHIPPED | `start_hero_section`: exactly three primary actions (Open, Load an example, New TT tile) |
| 17 | P1 | 4C | SHIPPED | `onboarding::Hints`: Layers/DRC/Share fire once each, dismissal persisted |
| 18 | P1 | 3C | SHIPPED | `shortcuts_overlay` generated from the registry on `?`; `help.shortcuts` |
| 19 | P2 | 4C | SHIPPED | `onboarding::Checklist`: four tasks with progress and a permanent dismiss |
| 20 | P1 | 4C | SHIPPED | `App::with_tour`; `?tour=1` (crates/web/src/main.rs), `--tour` (native main.rs) |
| 21 | P2 | 2B | SHIPPED | agent sample prompts in the Automate group (inspector/agent panel) |
| 22 | P2 | 4C | SHIPPED | first-run GPU capability card reporting the live wgpu adapter and backend |
| 23 | P1 | 2D | SHIPPED | `viewer.rs`, `App::viewer_panels`, `viewer_top_bar`: presence, follow, view-only, one editor affordance |
| 24 | P3 | 2D | SHIPPED | `tt_wizard` and `draw_pin_map` (schematic pin map) |
| 25 | P1 | 2A | SHIPPED | toolbar `IconButton` tooltips carry name, a `KbdChip` for the live chord, and a one-line description (menu.rs, iconbutton-spec.md) |
| 26 | P2 | 4C | SHIPPED | `help.whats_new` renders embedded `changelog()` data newest-first |

## C. Canvas, navigation, fluidity

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 27 | P1 | 3A | SHIPPED | gentler wheel multiplier over the anchoring `ViewCamera::zoom_about` (app.rs:10299); pinch calibration in the tested `camera::apply_pinch` centroid invariant |
| 28 | P1 | 3A | SHIPPED | `view.zoom_fit`, `view.zoom_selection` (Shift+F), `view.zoom_one_to_one`, `view.zoom_layer_extents` |
| 29 | P1 | 3A | SHIPPED | `camera::CameraTween` (ease-out cubic, log-space zoom); reduced motion collapses to instant |
| 30 | P1 | 3A | SHIPPED | minimap routed through `overlay.rs`; click and drag recenter; `resident_tile_rects` shades resident tiles in archive mode |
| 31 | P2 | 3A | LEDGERED (partial) | hide option shipped (`view.rulers`); cursor tick marks, unit toggle, and origin badge are the ledgered P2 polish beyond the collision and legibility fix |
| 32 | P1 | 3A | SHIPPED | status-bar coordinates always on (tabular mono); optional full-pane crosshair via `draw_crosshair` |
| 33 | P2 | 3A | LEDGERED | grid pitch/subdivision/style/fade popover: pre-authorized 3A P2 tail |
| 34 | P2 | 3A | SHIPPED (partial) | `view.bookmark_save` plus nine camera slots plus palette recall (`RecallBookmark`) ship; named 1-9 labels and permalink-encoded recall are ledgered |
| 35 | P1 | 3A | SHIPPED | `share.copy_permalink` builds and copies the current-view permalink; default chord unbound per ia-inventory section 4, rebindable (see POTENTIAL HOLDS note) |
| 36 | P3 | 3A | LEDGERED | hold-Z bird's-eye peek: P3 tail |
| 37 | P2 | 3A | LEDGERED | middle-drag and space-drag pan parity: not implemented (grep of app.rs finds no middle or space pan path); P2 tail, the setting was to live in 4C's dialog |
| 38 | P2 | 3A | LEDGERED (partial) | `settings::WheelBehavior` and its Zoom/Pan toggle plus persistence ship in the 4C Settings dialog, but the canvas scroll handler (app.rs:10290-10303) always zooms and never branches on `self.wheel` (only passed to `session::capture`, app.rs:4019). The setting persists but does not yet change canvas behavior; P2 tail |
| 39 | P1 | 2C | SHIPPED | `layers.rs` hover peek (`peek_layer`, `palette_from_layers_peek` dims others to 22%); alt-click solo (`LayerState::solo`) |
| 40 | P1 | 3A | SHIPPED | double-click a shape frames it (animated); double-click empty fits all (app.rs) |
| 41 | P2 | 3A | LEDGERED | hierarchy click-through, breadcrumb, Esc-pops: P2 tail |
| 42 | P1 | 3A | SHIPPED | [fluidity] `streamed::LevelFade` LOD crossfade (streamed.rs:505, tested; reduced motion to 0) |
| 43 | P2 | 3A | SHIPPED | [fluidity] `archive::VelocityTracker`, `predicted_viewport`, `prefetch_viewport` (archive.rs, tested) with an additive `archive_prefetched` stat and honest HUD line |
| 44 | P1 | 3A | SHIPPED | [fluidity] `benches/pan_latency.rs` (`canvas_pan_pointer_latency`, 868.38 ns) recorded in benches/history/baseline.json with the 16 ms budget, enforced by `just perf-check` |

## D. Selection and editing

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 45 | P1 | 3A | SHIPPED | marquee shift-add, alt-subtract, same-layer filter (`selection::shapes_in_rect_on_layer`, `Selection::subtract`) |
| 46 | P1 | 3A | SHIPPED | hover pre-highlight of the click target (`hover_pick`, `draw_hover_highlight`) |
| 47 | P1 | 3C | SHIPPED | `commands::MenuContext` and `context_menu_body`: right-click menus from the registry |
| 48 | P1 | 3A | SHIPPED | `handle_arrow_nudge`: grid step, 10x with Shift, 1 DBU fine, one undo group, delta readout |
| 49 | P2 | 3A | SHIPPED (partial) | `edit.duplicate` (Ctrl+D) with offset ships; repeat-last-transform ledgered (P2 tail) |
| 50 | P2 | 3A | LEDGERED | align/distribute mini-toolbar on multi-select: P2 tail |
| 51 | P1 | 3A | SHIPPED | numeric transform popover (`transform_window`, `apply_transform`, `resize_direct_rect`) |
| 52 | P1 | 3A | SHIPPED | draw-tool ghost with live dimensions; Esc cancels (`cancel_in_progress_draw`) |
| 53 | P1 | 3A | SHIPPED | snap indicators (vertex/midpoint/center/edge) plus the Ctrl/Cmd bypass modifier (`snap_bypass` in `snap_world`) |
| 54 | P2 | 2B | SHIPPED (partial) | clickable multi-step undo timeline ships; rows are labeled by distance, not by edit name (History exposes depths only), ledgered until a labeled-history API exists |
| 55 | P1 | 3A | SHIPPED | status-bar selection summary: count, bbox, area (`selection_summary`) |
| 56 | P2 | 2B | SHIPPED (partial) | `select.same_layer` ships; select-same-cell ledgered (the flattened scene carries no per-shape cell attribution) |
| 57 | P2 | 2C | SHIPPED (partial) | layer locking via `LayerState` and the `is_pickable` gate ships; per-shape locking rides the same single gate as a future extension |
| 58 | P2 | 3A | LEDGERED | measure upgrade (chained, angle, copy, persists): P2 tail |

## E. Panels and IA details

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 59 | P1 | 2B | SHIPPED | `inspector_layout.rs` icon-rail collapse; `view.panels_toggle` (Tab) toggles all |
| 60 | P2 | 2B | SHIPPED | per-group gear menu (Expand all / Collapse all / per-section toggles); the `Collapsible` renders its own header, so the gear is group-level |
| 61 | P1 | 2B | SHIPPED | `session.rs` keys `panel_right_w`, `panel_group`, `panels_collapsed`, `panel_open` persist per device |
| 62 | P2 | 2C | SHIPPED | layer color editor popover and visibility presets (`layers.rs` `set_color`, `save_preset`/`apply_preset`/`delete_preset`) |
| 63 | P1 | 2B | SHIPPED | DRC list click-zoom, previous/next cycle, stale indicator (`drc_ran_revision` vs `history.revision()`), empty state; `n`/`N` reassigned to buttons (N is `view.minimap`) (see POTENTIAL HOLDS) |
| 64 | P1 | 2B | SHIPPED | diff change list: legend, per-layer filter, previous/next, click-to-center (`diff_overlay.rs`) |
| 65 | P1 | 2B | SHIPPED | comments: unresolved badge, resolved filter, per-row resolve/reopen (`comment_pins.rs`); resolved state client-side, not written to the shared proto |
| 66 | P1 | 2B | SHIPPED (partial) | agent transcript: instant stop, status line, sample prompts, collapsible plan/conversation/history, replay scrubber; per-command step icons ledgered (see POTENTIAL HOLDS) |
| 67 | P2 | 2B | SHIPPED | event-driven auto-expand (`reveal("drc")` on fresh violations, `reveal("comments")` on a new thread) |
| 68 | P1 | 3A | SHIPPED | actionable status bar: zoom readout opens the preset menu, fps opens a perf popover, archive label toggles the streaming HUD |
| 69 | P2 | 3B | LEDGERED (partial) | the retention MODEL ships and is tested (`notify::Notifications` bounded `history()`); the dedicated notification-center panel is not surfaced (belongs beside the 2B Inspector) |
| 70 | P1 | 2A | SHIPPED | `dev.add_demo_rect` ("Insert demo rectangle") and `dev.replay_theater` relocated to Help > Developer; the History-panel button removed |

## F. Feedback and errors

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 71 | P1 | 3B | SHIPPED | `notify::NotificationAction` (Retry/Copy/Undo/Fit) rendered through `components::Toast` with severity |
| 72 | P1 | 3B | SHIPPED | `notify::Diagnostic` (pure, tested): cause, next step, copyable block on every failure path |
| 73 | P1 | 3B | SHIPPED | `notify::TaskStage`/`LongTask` encode the 300 ms and 2 s thresholds; the cancelable card is the >2 s surface |
| 74 | P1 | 3B | SHIPPED | `notify::ConnectivityState` (pure, tested): offline badge and at-most-one reconnect toast per transition |
| 75 | P1 | 2D | SHIPPED | `session_chip`: state, participant count, avatars |
| 76 | P2 | 3A | LEDGERED | HUD compact pill and fetch sparkline: P2 tail (the HUD is honest and the residency minimap covers the overview) |
| 77 | P2 | 3A | LEDGERED | sustained low-fps detector and suggestion toast: P2 tail |
| 78 | P1 | 3B | SHIPPED | `Notifications::undoable`: do-then-Undo toast replacing a confirmation modal |

## G. Keyboard and palette

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 79 | P1 | 3C | SHIPPED | `command.rs` `build_items` sources the palette from the registry: fuzzy, recents, groups, hints, argument prompts (`palette.goto_coordinate`, `palette.goto_cell`) |
| 80 | P1 | 3C | SHIPPED | `palette_doc_targets`: the palette searches layers, cells, bookmarks |
| 81 | P2 | 3C | SHIPPED | `keymap::SEQUENCES` chorded shortcuts (`g c`, `g x`, `g f`), documented in the overlay |
| 82 | P3 | 3C | SHIPPED | `keymap_window`: remapping UI, live conflict check, TOML export/import |
| 83 | P1 | 3C | SHIPPED | `focus::FocusRegion` four-region ring; `focus.cycle` (F6); a focus ring painted per region |
| 84 | P1 | 3C | SHIPPED | `focus::esc_action` cascade (cancel tool, clear selection, close popover), pure and unit-tested; `apply_esc` maps it |

## H. Collaboration and share

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 85 | P1 | 3B | SHIPPED | `share.dialog`: mode toggle, copy, test-open, honest expiry line, live status, QR (`share.rs`) |
| 86 | P1 | 2D | SHIPPED | `viewer::{COLLAB_PALETTE,color_for_actor,idle_alpha}`, `draw_presence`: named cursors, palette colors, idle fade |
| 87 | P1 | 2D | SHIPPED | `viewer::{lerp_camera,ViewerSession::follow_step}`: follow by clicking an avatar, smooth camera, following chip |
| 88 | P2 | 2D | LEDGERED | bring-everyone-here needs a new one-shot broadcast on the frozen presence/relay seam; follow mode already lets viewers ride the sharer voluntarily |
| 89 | P2 | 3B | LEDGERED | in-session comment toasts: the toast mechanism is ready in `notify`; the comment/pin events are 2B's; one-line wiring deferred to that seam |
| 90 | P2 | 2D | SHIPPED | `draw_remote_edit_glow`: remote-edit attribution glow |
| 91 | P3 | 2D | SHIPPED | sharer-leave read-only freeze notice (`apply_live_event` status, `viewer_top_bar` banner) |
| 92 | P2 | 3B | SHIPPED | `qr.rs`: dependency-free byte-mode QR (versions 1-5, EC-L), unit-tested against canonical Reed-Solomon and format-info vectors |

## I. Viewer and demo experience

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 93 | P1 | 2D | SHIPPED | `view.presentation` (P), `AppOp::TogglePresentation`, `presentation_canvas`: hide all chrome |
| 94 | P2 | 2D | SHIPPED | `share::parse_embed`: `?embed=1` minimal chrome with the CORS/CSP doc note |
| 95 | P2 | W5 | LEDGERED | cinematic auto-pan capture mode: not built at the time of this sweep (no `cinematic`/`auto-pan` symbol in `crates/`, `scripts/`, or the capture harness). Deferred; it reuses the media capture harness, recorded here honestly as pending |
| 96 | P1 | 2D | SHIPPED | gallery info card ("what am I looking at"): the `Landmark` dropdown per card in `start_gallery_section` |

## J. Load, settings, help

| # | tier | owner | disposition | evidence |
|---|---|---|---|---|
| 97 | P1 | 3A | SHIPPED | `crates/web/index.html` boot splash: indeterminate progress bar plus rotating tip, reduced-motion aware, stopped on `TrunkApplicationStarted` |
| 98 | P1 | 4C | SHIPPED | `settings.rs` and `help.settings`: density, reduced motion, wheel behavior, touch mode, panel-layout reset, all persisted |
| 99 | P1 | 4C | SHIPPED | About dialog (`help.about`): version, bundle hash, platform, live GPU adapter and backend, copy-diagnostics, prefilled issue link |
| 100 | P1 | 4C | SHIPPED | `help::ZERO_TELEMETRY` in About; the claim was verified by a sweep of `crates/` and `crates/web/` for telemetry and analytics SDKs (none) |

## Ledgered items, grouped by reason

- **Pre-authorized 3A P2/P3 tail** (catalog-map.md names this set as ledger-if-squeezed):
  33 (grid popover), 36 (hold-Z peek), 41 (hierarchy breadcrumb), 50
  (align/distribute), 58 (measure upgrade), 76 (HUD pill), 77 (low-fps toast),
  plus the partials 31 (ruler polish), 34 (named bookmark labels + permalink),
  37 (pan parity), 38 (wheel-behavior effect), 49 (repeat-last-transform).
- **Cross-crate or frozen-seam ledgers**: 4 (multi-document), 9 (thumbnail
  cache + IndexedDB + pin persistence), 10 (document-bytes restore), 11 (PWA
  file handlers in the web manifest), 13 (File System Access save-back), 56
  (same-cell needs scene cell attribution), 69 (notification-center panel), 88
  (presence/relay one-shot broadcast), 89 (comment-event to toast wiring).
- **Capture track**: 95 (cinematic auto-pan), deferred.

## Rejected design directions (not catalog items)

The packet's explicitly considered-and-rejected set (brief.md) is separate from
the 100 numbered items. None is a catalog disposition; they are recorded here for
completeness and reused in the honest-limits ledger:

- **Light theme**: deferred by design; one dark theme this packet, the token
  table makes a second (light) table cheap later (ADR 0095, tokens.md).
- **Docking (egui_dock 0.20.1)**: surveyed and declined for persistence
  simplicity and dependency budget; managed panels ship instead (ADR 0096).
- **Internationalization**: no i18n infrastructure this packet; ledgered.
- **Vim-style modal navigation**: audience mismatch with the discoverability
  principle; chords stay simple and visible.
- **Telemetry-informed UX**: contradicts catalog item 100 (zero telemetry as a
  stated feature).
- **Idle-time pre-tessellation**: engine work beyond the three authorized
  fluidity items (42, 43, 44).
- **User-installable theme marketplace**: system integrity over customization.
- **Rebranding or logo work**: beyond a restrained Start-screen wordmark.

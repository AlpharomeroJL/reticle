# Catalog map: Appendix A ownership (the Wave 5 completeness contract)

Every numbered item of the packet's Improvement Catalog, its confirmed tier, and
its owning lane. The owner ships the item (or ledgers it with a reason) inside
its Wave 2 lane; Wave 1 lanes own only system plumbing. The Wave 5 audit walks
this table and records a disposition per item in catalog-dispositions.md.
[fluidity] marks the three authorized engine-adjacent exceptions.

Owner key: Wave 2 lanes 2A menus/toolbar, 2B inspector, 2C layers/views,
2D viewer/start, 3A canvas/nav/fluidity, 3B open/dialogs, 3C palette/keys,
4A states/motion, 4B touch/a11y, 4C onboarding/help; W5 = orchestrator Wave 5.

## A. Opening and files

| # | tier | owner | item (short) |
|---|---|---|---|
| 1 | P1 | 3B | Native file picker (rfd) on File > Open, toolbar, Ctrl+O |
| 2 | P1 | 3B | Open-from-URL dialog + CORS explainer |
| 3 | P2 | 3B | Paste-to-open (file or URL) |
| 4 | P2 | 3B | Multi-file open with per-file result list |
| 5 | P1 | 3B | Full-window drag overlay with format hints |
| 6 | P1 | 3B | Staged open progress (parse, tessellate, upload) + cancel |
| 7 | P1 | 3B | Post-open summary toast with Fit action |
| 8 | P1 | 3B | Corrupt/unsupported file: cause, formats, convert suggestion |
| 9 | P2 | 2D | Recent files with cached thumbnails (IndexedDB) + pinning |
| 10 | P2 | 2D | Session restore offer (opt-in toast) |
| 11 | P2 | 3B | PWA file handlers (OS Open with Reticle) |
| 12 | P2 | 2B | Export menu consolidation (GDS, OASIS, PNG, CSV) |
| 13 | P3 | 3B | Save-back via File System Access API |
| 14 | P1 | 2D | Gallery cards: name, tech, size, source, license, badges |

## B. Onboarding and first contact

| # | tier | owner | item |
|---|---|---|---|
| 15 | P1 | 4C | 60-second interactive tour, editor + viewer variants |
| 16 | P1 | 2D | Empty-canvas state with exactly three primary actions |
| 17 | P1 | 4C | Once-only contextual hints, dismissal tracked |
| 18 | P1 | 3C | Shortcuts overlay on ?, generated from the registry |
| 19 | P2 | 4C | Onboarding checklist card with progress |
| 20 | P1 | 4C | ?tour=1 deep link |
| 21 | P2 | 2B | Agent panel sample prompts |
| 22 | P2 | 4C | First-run GPU capability card |
| 23 | P1 | 2D | Viewer-mode banner: presence, follow, view-only clarity |
| 24 | P3 | 2D | New TT tile wizard with pin-map preview |
| 25 | P1 | 2A | Tooltips everywhere: name + shortcut + one-liners |
| 26 | P2 | 4C | What's new dialog from changelog data |

## C. Canvas, navigation, fluidity

| # | tier | owner | item |
|---|---|---|---|
| 27 | P1 | 3A | Zoom-to-cursor tuning, pinch calibration |
| 28 | P1 | 3A | Zoom presets: F, Shift+F, 1:1 DBU, layer extents |
| 29 | P1 | 3A | Animated view transitions (~150 ms, reduced-motion aware) |
| 30 | P1 | 3A | Minimap: draggable viewport, click-to-jump, residency shading |
| 31 | P2 | 3A | Rulers: cursor marks, unit toggle, origin badge, hide option |
| 32 | P1 | 3A | Status-bar coordinates always; optional crosshair |
| 33 | P2 | 3A | Grid popover: pitch, subdivisions, style, fade |
| 34 | P2 | 3A | Named view bookmarks (1-9), palette + permalink |
| 35 | P1 | 3A | Copy permalink at current view + shortcut |
| 36 | P3 | 3A | Hold-Z bird's-eye peek |
| 37 | P2 | 3A | Pan parity: middle-drag and space-drag (setting via 4C) |
| 38 | P2 | 3A | Wheel behavior setting: zoom vs pan (setting via 4C) |
| 39 | P1 | 2C | Layer-row hover peek dims others; alt-click solo |
| 40 | P1 | 3A | Double-click shape to fit; double-click empty to fit all |
| 41 | P2 | 3A | Hierarchy click-through, breadcrumb, Esc pops |
| 42 | P1 | 3A | [fluidity] LOD crossfade |
| 43 | P2 | 3A | [fluidity] Velocity-aware tile prefetch, honest HUD |
| 44 | P1 | 3A | [fluidity] 16 ms pointer-latency budget test in perf guard |

## D. Selection and editing

| # | tier | owner | item |
|---|---|---|---|
| 45 | P1 | 3A | Marquee select, shift-add, alt-subtract, same-layer filter |
| 46 | P1 | 3A | Hover pre-highlight of click target |
| 47 | P1 | 3C | Right-click context menus from the registry |
| 48 | P1 | 3A | Arrow-key nudge (grid step, 10x, fine) + delta readout |
| 49 | P2 | 3A | Ctrl+D duplicate with offset; repeat-last-transform |
| 50 | P2 | 3A | Align/distribute mini-toolbar on multi-select |
| 51 | P1 | 3A | Numeric transform popover (x, y, w, h) |
| 52 | P1 | 3A | Draw-tool ghost preview with dimensions; Esc cancels |
| 53 | P1 | 3A | Snap indicators (vertex, edge, center) + bypass modifier |
| 54 | P2 | 2B | Undo timeline panel (labeled, clickable) |
| 55 | P1 | 3A | Status-bar selection summary: count, bbox, area |
| 56 | P2 | 2B | Select-same: layer, cell |
| 57 | P2 | 2C | Layer and shape locking |
| 58 | P2 | 3A | Measure upgrade: chained, angle, copy, persists |

## E. Panels and IA details

| # | tier | owner | item |
|---|---|---|---|
| 59 | P1 | 2B | Panels collapse to icon rail; Tab toggles all |
| 60 | P2 | 2B | Per-panel gear menus |
| 61 | P1 | 2B | All panel state persisted per device |
| 62 | P2 | 2C | Layer color editor popover + visibility presets |
| 63 | P1 | 2B | DRC violations navigable: click-zoom, n/N, stale indicator |
| 64 | P1 | 2B | Diff change list: prev/next, per-layer filter, legend |
| 65 | P1 | 2B | Comments: unresolved badge, resolved filter, pin sync |
| 66 | P1 | 2B | Agent transcript: collapsible steps, status icons, stop, scrubber |
| 67 | P2 | 2B | Event-driven auto-expand (DRC on violations, Comments on thread) |
| 68 | P1 | 3A | Actionable status bar (zoom presets, perf popover, HUD toggle) |
| 69 | P2 | 3B | Notification center retaining recent toasts |
| 70 | P1 | 2A | Debug-register copy retired to Help > Developer |

## F. Feedback and errors

| # | tier | owner | item |
|---|---|---|---|
| 71 | P1 | 3B | Unified toast system: severity + action buttons |
| 72 | P1 | 3B | Every failure: cause, next step, copyable diagnostic |
| 73 | P1 | 3B | Long-task pattern: 300 ms spinner, 2 s progress + cancel |
| 74 | P1 | 3B | Offline badge (PWA) + reconnect toasts on existing states |
| 75 | P1 | 2D | Live session chip: state, viewer count, avatars |
| 76 | P2 | 3A | HUD compact pill + fetch sparkline on expand |
| 77 | P2 | 3A | Sustained low-fps detector, single suggestion toast |
| 78 | P1 | 3B | Do-then-Undo replaces confirmations where reversible |

## G. Keyboard and palette

| # | tier | owner | item |
|---|---|---|---|
| 79 | P1 | 3C | Palette: fuzzy, recents, groups, hints, argument prompts |
| 80 | P1 | 3C | Palette searches layers, cells, bookmarks |
| 81 | P2 | 3C | Chorded shortcuts (g then l), documented in overlay |
| 82 | P3 | 3C | Shortcut remapping UI + conflict detector + export/import |
| 83 | P1 | 3C | F6 cycles focus regions; visible focus everywhere |
| 84 | P1 | 3C | Esc cascade contract documented and tested |

## H. Collaboration and share

| # | tier | owner | item |
|---|---|---|---|
| 85 | P1 | 3B | Share dialog: mode toggle, expiry, viewer count, test-open |
| 86 | P1 | 2D | Named presence cursors: display name, colors, idle fade |
| 87 | P1 | 2D | Follow mode: click avatar, smooth camera, followed chip |
| 88 | P2 | 2D | Bring everyone here (viewport sync) |
| 89 | P2 | 3B | In-session comment toasts |
| 90 | P2 | 2D | Remote-edit attribution glow |
| 91 | P3 | 2D | Sharer-leave handoff or read-only freeze notice |
| 92 | P2 | 3B | QR code in Share dialog (client-side, in budget) |

## I. Viewer and demo experience

| # | tier | owner | item |
|---|---|---|---|
| 93 | P1 | 2D | Presentation mode on P (hide all chrome) |
| 94 | P2 | 2D | ?embed=1 minimal chrome + CORS/CSP notes |
| 95 | P2 | W5 | Cinematic auto-pan capture mode (capture harness reuse) |
| 96 | P1 | 2D | Gallery info card: what am I looking at |

## J. Load, settings, help

| # | tier | owner | item |
|---|---|---|---|
| 97 | P1 | 3A | Splash with progress + rotating tip during wasm init |
| 98 | P1 | 4C | Settings dialog, persisted (density, motion, wheel, touch, reset) |
| 99 | P1 | 4C | Diagnostics in About: versions, GPU, hash, copy, issue link |
| 100 | P1 | 4C | Zero-telemetry statement in About, verified true |

## Load summary (for dispatch and the ledger-if-squeezed call)

| lane | items | P1 | P2 | P3 | list |
|---|---|---|---|---|---|
| 3A | 31 | 18 | 12 | 1 | 27-38, 40-46 minus 39, 48-53, 55, 58, 68, 76, 77, 97 (heaviest; the P2/P3 tail is the designated ledger set) |
| 3B | 19 | 12 | 6 | 1 | 1-8, 11, 13, 69, 71, 72, 73, 74, 78, 85, 89, 92 |
| 2D | 15 | 8 | 5 | 2 | 9, 10, 14, 16, 23, 24, 75, 86, 87, 88, 90, 91, 93, 94, 96 |
| 2B | 12 | 6 | 6 | 0 | 12, 21, 54, 56, 59, 60, 61, 63, 64, 65, 66, 67 |
| 4C | 9 | 6 | 3 | 0 | 15, 17, 19, 20, 22, 26, 98, 99, 100 |
| 3C | 8 | 6 | 1 | 1 | 18, 47, 79, 80, 81, 82, 83, 84 |
| 2C | 3 | 1 | 2 | 0 | 39, 57, 62 (+ managed panels from the wave text) |
| 2A | 2 | 2 | 0 | 0 | 25, 70 (+ menu bar/toolbar from the wave text) |
| 4A/4B | 0 | 0 | 0 | 0 | no numbered items; scope is the packet's Wave 4 text (states, motion, frame guard; touch targets, text scaling, e2e-touch) |
| W5 | 1 | 0 | 1 | 0 | 95 |

Tally: 59 P1 + 36 P2 + 5 P3 = 100 (verified against the packet). 3A's tail (31,
33, 34, 36, 37, 38, 41, 49, 50, 58, 76, 77) is pre-authorized to ledger if its
P1 set is at risk; every other lane is expected to finish its list or ledger
with a specific reason.

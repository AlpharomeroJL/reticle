# Catalog map: the Appendix A improvement catalog

Every numbered item of the Improvement Catalog, with its confirmed priority tier
and a short description. `catalog-dispositions.md` records a disposition per item
(shipped, ledgered, or deferred) backed by a committed evidence pointer.
[fluidity] marks the three authorized engine-adjacent exceptions. The `area` column
groups items by the interface area they belong to.

## A. Opening and files

| # | tier | area | item (short) |
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

| # | tier | area | item |
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

| # | tier | area | item |
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

| # | tier | area | item |
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

| # | tier | area | item |
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

| # | tier | area | item |
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

| # | tier | area | item |
|---|---|---|---|
| 79 | P1 | 3C | Palette: fuzzy, recents, groups, hints, argument prompts |
| 80 | P1 | 3C | Palette searches layers, cells, bookmarks |
| 81 | P2 | 3C | Chorded shortcuts (g then l), documented in overlay |
| 82 | P3 | 3C | Shortcut remapping UI + conflict detector + export/import |
| 83 | P1 | 3C | F6 cycles focus regions; visible focus everywhere |
| 84 | P1 | 3C | Esc cascade contract documented and tested |

## H. Collaboration and share

| # | tier | area | item |
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

| # | tier | area | item |
|---|---|---|---|
| 93 | P1 | 2D | Presentation mode on P (hide all chrome) |
| 94 | P2 | 2D | ?embed=1 minimal chrome + CORS/CSP notes |
| 95 | P2 | W5 | Cinematic auto-pan capture mode (capture harness reuse) |
| 96 | P1 | 2D | Gallery info card: what am I looking at |

## J. Load, settings, help

| # | tier | area | item |
|---|---|---|---|
| 97 | P1 | 3A | Splash with progress + rotating tip during wasm init |
| 98 | P1 | 4C | Settings dialog, persisted (density, motion, wheel, touch, reset) |
| 99 | P1 | 4C | Diagnostics in About: versions, GPU, hash, copy, issue link |
| 100 | P1 | 4C | Zero-telemetry statement in About, verified true |

## Priority tally

59 P1 + 36 P2 + 5 P3 = 100 items, verified against the catalog. The disposition of
each item (shipped, ledgered, or deferred), with a committed evidence pointer, is
in `catalog-dispositions.md`.

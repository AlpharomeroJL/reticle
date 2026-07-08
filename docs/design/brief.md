# Design brief (v8.1 interface packet)

The binding taste document for every lane. The packet's principles are restated
here as the working contract; the token spec (tokens.md), IA inventory
(ia-inventory.md), audit (audit.md), and catalog map (catalog-map.md) are its
enforcement arms. Decision records: ADR 0094 (run structure), 0095 (tokens),
0096 (managed panels), 0097 (type and icons), 0098 (ratchet and bundle gate).

## North star

Reticle's UI should feel like a professional tool that happens to run in a
browser: the canvas is the hero and the chrome recedes; every action is
keyboard-first and discoverable twice; density and hierarchy match tools
engineers already respect. The current UI reads as an engine demo wearing debug
controls (see audit.md, 25 findings). The target: a first-time visitor
screenshots the interface itself.

## The eight principles (binding)

1. Canvas first: chrome neutral and desaturated; color in panels means data.
2. Progressive disclosure: rare controls live in menus, collapsed sections, or
   the palette; nothing floats over something clickable, ever.
3. One source of visual truth: every color, size, radius, spacing, and font
   comes from `theme/`; the check-style lint makes literals a CI failure.
4. Keyboard-first, twice discoverable: one command registry powers menus,
   palette, and the shortcuts overlay; menus show chords; nothing is
   palette-only or menu-only.
5. Density with hierarchy: compact like Linear, organized like Figma's panels,
   honest like Blender about being a deep tool; section headers, spacing
   rhythm, and empty states that say what to do next.
6. Motion is functional: transitions communicate state change; nothing animates
   for decoration; reduced motion is respected everywhere.
7. Dark-optimized with contrast proven, not eyeballed: token pairs carry WCAG
   AA unit tests (tokens.md table is re-proved in CI).
8. Viewer and editor are different products sharing a canvas: share links open
   clean viewer chrome; the full editor is one obvious affordance away.

## Taste anchors: what we take (and refuse) from each

- **Figma**: the Inspector discipline. Sections with consistent headers, one
  panel with grouped modes, properties that appear only when a selection gives
  them meaning. Refused: floating toolbars over content.
- **Blender**: deep-tool honesty. It is fine to have many capabilities visible
  as long as hierarchy is real (groups, collapse, search); the n-panel idea
  becomes our collapsible Inspector sections with persisted state. Refused:
  mode-switching complexity that hides where you are.
- **VS Code**: the palette as the spine. Every command lives in one registry;
  fuzzy search with recents and grouping; the shortcuts overlay is generated,
  never hand-maintained. Refused: notification spam; our toasts carry actions
  and expire.
- **Linear**: density, keyboard focus, restraint. Quiet chrome, small type done
  legibly (contrast-tested), kbd hint chips, almost no borders where spacing
  can separate. Refused: hiding depth; Reticle is a deep tool and says so.
- **KLayout**: domain expectations to meet or consciously exceed. Rulers with
  unit toggles, DBU-exact readouts in mono, a layer list that is fast to scan
  and filter, hierarchy click-through with a breadcrumb. Refused: its chrome
  aesthetics; we match its respect for precision, not its widget style.

## System facts (decided; lanes do not relitigate)

- Fonts: Inter (Regular, Medium; SIL OFL 1.1) for UI, JetBrains Mono (SIL OFL
  1.1) for readouts. Subsets committed; regeneration via
  `scripts/subset-fonts.ps1`.
- Icons: **Lucide, ISC license** (recorded here per the packet), embedded as a
  subset icon font, glyph constants generated into `theme/icons.rs`. One set;
  no mixing; no emoji in chrome.
- One dark theme this packet; the Theme enum stays for a cheap future light
  variant (tokens.md records the migration rules).
- Managed panels, not docking (ADR 0096: egui_dock 0.20.1 surveyed and
  declined for persistence simplicity and dependency budget).
- Bundle budget: the redesign adds at most 450 KB gzipped, gated by
  `just bundle-gate` against the v8.0 baseline row in bundle-ledger.md.
- Component library (`theme/components.rs`, owned by 1C in Wave 1, additive-only
  for 4A in Wave 2) is the only widget source from Wave 2 forward.

## Considered and rejected (taste made explicit, per the packet)

- Vim-style modal navigation: audience mismatch with the discoverability
  principle; chords stay simple and visible.
- Internationalization: no i18n infrastructure this packet; ledgered.
- Light theme: deferred by design; the token system makes it cheap later.
- Telemetry-informed UX: contradicts catalog item 100 (zero telemetry as a
  stated feature).
- Idle-time pre-tessellation: engine work beyond the three authorized fluidity
  items (42, 43, 44).
- User-installable theme marketplace: system integrity over customization.
- Docking (egui_dock): declined this packet, future rider (ADR 0096).
- Rebranding/logo work: a restrained wordmark treatment on the Start screen is
  2D's call; nothing beyond it.

## Completeness contract

catalog-map.md assigns all 100 Appendix A items to owning lanes with tiers
confirmed. P1 must ship (a miss without a recorded genuine blocker is a release
hold); P2 ships unless schedule forces a ledger entry; P3 is stretch. Every item
receives a Wave 5 disposition (shipped with evidence, ledgered with reason, or
rejected with reason) in catalog-dispositions.md; silent drops are a sweep
failure.

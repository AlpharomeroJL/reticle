# Token specification (v8.1 design system)

The single source of visual truth for the redesign. Lane 1A encodes this table as
`crates/reticle-app/src/theme/tokens.rs` and maps it onto `egui::Style`/`Visuals`;
the contrast pairs below become unit tests in `theme/contrast.rs`; the check-style
lint bans color and size literals outside `theme/`, so nothing can drift from this
file silently. Decision record: ADR 0095.

## One theme this packet

v8.1 ships a single dark theme, per the packet ("dark-optimized for layout work";
light is explicitly out of scope so visual testing is not doubled for a secondary
audience). Consequences, decided here:

- The v8.0 stock-egui Light toggle is removed from the UI. Shipping a tokened dark
  beside an untokened stock light would be exactly the inconsistency this packet
  exists to remove.
- The `Theme` enum stays (it moves into `theme/`), so a future light variant is a
  second token table, not an architecture change. Session files containing
  `theme=light` keep parsing and resolve to Dark; the tag is tolerated forever.
- This goes in the honest-limits ledger at release: light theme deferred by design,
  tokens make it cheap later.

## Color tokens (dark)

Chrome is neutral and desaturated so layer colors and geometry dominate; color in
panels means data or state, never decoration.

| token | hex | rgb | role |
|---|---|---|---|
| `bg_canvas` | `#101216` | 16,18,22 | canvas clear color (kept from v8.0; the hero backdrop) |
| `bg_panel` | `#16181d` | 22,24,29 | side/top/bottom panel fill |
| `bg_raised` | `#1c1f26` | 28,31,38 | menus, popovers, cards, modal frames |
| `bg_input` | `#0d0f12` | 13,15,18 | text inputs, wells, code/readout fields |
| `border` | `#2a2e37` | 42,46,55 | hairline separators, widget outlines |
| `border_strong` | `#3d4350` | 61,67,80 | panel edges, emphasized separators |
| `text` | `#e2e5ea` | 226,229,234 | primary text |
| `text_weak` | `#9aa0ab` | 154,160,171 | secondary text, captions, hints |
| `text_faint` | `#6a7180` | 106,113,128 | disabled text only (no AA claim; WCAG exempts disabled) |
| `accent` | `#6ea8fe` | 110,168,254 | links, focused borders, primary-action fill |
| `accent_text` | `#0c1220` | 12,18,32 | text on accent fills |
| `accent_muted` | `#25406b` | 37,64,107 | selection fills, active-toggle backgrounds |
| `danger` | `#f47067` | 244,112,103 | errors, destructive actions |
| `warning` | `#e5c07b` | 229,192,123 | warnings, stale indicators |
| `success` | `#57ab5a` | 87,171,90 | confirmations, pass states |
| `focus` | `#6ea8fe` | 110,168,254 | keyboard-focus ring (same hue as accent, 1.5px stroke) |
| `widget_bg` | `#22262e` | 34,38,46 | inactive widget fill |
| `widget_hover` | `#2a2f39` | 42,47,57 | hovered widget fill |
| `widget_active` | `#323845` | 50,56,69 | pressed/open widget fill |

Canvas data colors (layer palette, DRC heatmap, diff overlay, presence cursors) are
NOT chrome tokens; they stay owned by the technology table and the overlay code.
The lint exempts nothing, so any canvas-side literal the lanes still need moves into
`theme/tokens.rs` as a named canvas token (for example `canvas_selection`,
`canvas_tour_highlight`, `canvas_guide`), keeping one source of truth without
pretending data colors are chrome.

## Contrast (measured, WCAG 2.x relative luminance)

Computed for this table; `theme/contrast.rs` re-proves every row in CI. All tested
tokens are fully opaque (the test asserts alpha 255 first; Color32 is premultiplied).

| pair | ratio | requirement |
|---|---|---|
| text on bg_panel | 14.06:1 | >= 4.5 (AA text) |
| text on bg_raised | 13.06:1 | >= 4.5 |
| text on bg_input | 15.20:1 | >= 4.5 |
| text on widget_bg | 12.01:1 | >= 4.5 |
| text on widget_hover | 10.63:1 | >= 4.5 |
| text on widget_active | 9.30:1 | >= 4.5 |
| text on accent_muted (selection) | 8.21:1 | >= 4.5 |
| text_weak on bg_panel | 6.76:1 | >= 3.0 (large/UI secondary) |
| text_weak on bg_raised | 6.28:1 | >= 3.0 |
| accent on bg_panel | 7.35:1 | >= 4.5 (links read as text) |
| accent_text on accent | 7.74:1 | >= 4.5 (primary button label) |
| danger on bg_panel | 6.22:1 | >= 4.5 |
| warning on bg_panel | 10.28:1 | >= 4.5 |
| success on bg_panel | 6.24:1 | >= 4.5 |
| focus on bg_panel | 7.35:1 | >= 3.0 (non-text UI, WCAG 1.4.11) |
| border_strong on bg_panel | 1.79:1 | >= 1.5 (visibility floor, informational) |
| text_faint on bg_panel | 3.62:1 | >= 2.0 (disabled floor, informational) |

## Spacing (4px rhythm) and density

Two density modes; every value on the 4px grid or its 2px half-step. Touch mode
(lane 4B) raises `interact_size.y` to 40 on top of either density.

| egui `Spacing` field | comfortable | compact |
|---|---|---|
| `item_spacing` | 8 x 6 | 6 x 4 |
| `button_padding` | 10 x 5 | 8 x 3 |
| `interact_size.y` | 28 | 22 |
| `icon_width` / `icon_width_inner` | 16 / 10 | 14 / 8 |
| `icon_spacing` | 6 | 4 |
| `window_margin` | 12 | 8 |
| `menu_margin` | 8 | 6 |
| `indent` | 16 | 12 |
| `combo_height` | 240 | 200 |

## Type scale

Faces per ADR 0097: Inter (Regular, Medium) for UI, JetBrains Mono for coordinates,
readouts, and code; Lucide as the icon fallback family. Values in egui points.

| egui text style | comfortable | compact |
|---|---|---|
| Small | 11.0 | 10.0 |
| Body | 13.0 | 12.0 |
| Button | 13.0 | 12.0 |
| Heading (panel/section) | 17.0 | 15.5 |
| Monospace | 12.5 | 11.5 |

Section headers use the `ui-medium` family (Inter Medium) at Body size with
`text_weak`, uppercase is NOT used (deep-tool honesty over label shouting).
Numerals in the status bar and inspector use Monospace with tabular figures
(the `tnum` feature survives subsetting).

## Radius scale

| token | px | applies to |
|---|---|---|
| `radius_sm` | 2 | chips, kbd hints, swatches |
| `radius_md` | 4 | buttons, inputs, all interactive widgets |
| `radius_lg` | 6 | menus, popovers, toasts |
| `radius_xl` | 8 | windows, modal frames, cards |

## Elevation

Two shadow recipes only (motion principle: elevation communicates layering, not
drama). egui 0.35 `Shadow { offset, blur, spread, color }`:

| token | offset | blur | spread | color |
|---|---|---|---|---|
| `shadow_popup` (menus, popovers, toasts) | 0,2 | 8 | 0 | black at 45% |
| `shadow_window` (modals, floating panels) | 0,4 | 16 | 0 | black at 55% |

## State styling rules (encoded by 1A, exercised by 4A)

- Hover: `widget_hover` fill, border unchanged. Active/pressed: `widget_active`.
- Focus: 1.5px `focus` stroke, always visible when keyboard-driven (F6/Tab paths);
  never removed for aesthetics.
- Selected (toggles, segmented controls, list rows): `accent_muted` fill + `text`.
- Disabled: `text_faint` foreground, fills unchanged, no hover response.
- Motion: `Style::animation_time` 0.12s comfortable, 0.10s compact, 0.0 when
  reduced motion is on; scroll animation off under reduced motion.

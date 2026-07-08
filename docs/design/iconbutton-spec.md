# IconButton spec (lane 1C contract)

The one icon-driven button every panel and the toolbar build from. Lane 1C
implements it in `crates/reticle-app/src/theme/components.rs`; this page is the
frozen behavioural contract. It closes AUD-18 and catalog item 25 (tooltips
everywhere: name, shortcut, one-liner). Decision record: ADR 0097 (type and
icons); values come from `docs/design/tokens.md`.

The constructor takes the glyph as a `char`, so 1C has zero compile dependency on
the generated `theme/icons.rs`: a caller passes `icons::COMMAND` (a `char`
constant) and IconButton never names the icons module.

## API

```rust
// theme/components.rs (lane 1C owns the body; this signature is the contract).
pub struct IconButton { /* ... */ }

impl IconButton {
    /// A glyph plus its accessible name. `glyph` is a Lucide codepoint (an
    /// `icons::*` constant); `label` is the action name shown in the tooltip
    /// and used as the accessible name.
    pub fn new(glyph: char, label: impl Into<String>) -> Self;

    /// Chord hint rendered as a kbd chip in the tooltip, e.g. "Ctrl+P".
    /// Omit for actions with no default chord.
    pub fn shortcut(self, hint: impl Into<String>) -> Self;

    /// One-line description under the name in the tooltip. Omit for actions
    /// whose name is self-evident.
    pub fn description(self, text: impl Into<String>) -> Self;

    /// Toggle/segmented state. `true` paints the selected fill.
    pub fn selected(self, on: bool) -> Self;

    /// Interactive when true (default), disabled styling when false.
    pub fn enabled(self, on: bool) -> Self;

    /// Draws it and returns the click/hover response.
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response;
}
```

Callers read the click from the returned `Response` (`.clicked()`); IconButton
dispatches nothing itself, so it stays free of the command registry (lane 3C).

## Sizing

- The hit target is a square, side = `Density::interact_height()` (28
  comfortable, 22 compact; lane 4B's touch mode raises it to
  `TOUCH_INTERACT_HEIGHT` = 40 the same way every other widget does). The glyph
  is centered in that square.
- The glyph renders through the Proportional family (Lucide is its fallback, so
  a `char` glyph resolves) at the icon point size: 18.0 comfortable, 16.0
  compact (about two thirds of the square, matching Lucide's 24-unit grid with
  breathing room). These are the icon sizes; when lane 1A adds a
  `Density::icon_size()` accessor to `theme/tokens.rs` the component reads it,
  and until then the literals live in `components.rs`, which the check-style
  lint exempts.
- Corner radius `RADIUS_MD` (4). No border in the resting state; spacing
  separates buttons, per the design brief's restraint principle.

## States (all fills and text from `tokens.rs`, never literals)

| state | fill | glyph | notes |
|---|---|---|---|
| resting | none (panel shows through) | `text_weak` | quiet chrome |
| hover | `widget_hover` | `text` | pointer only; no hover response when disabled |
| active/pressed | `widget_active` | `text` | while the mouse is down |
| selected (toggle on) | `accent_muted` | `text` | segmented controls, active tools, toggles |
| selected + hover | `accent_muted` over `widget_hover` blend | `text` | selection reads through hover |
| disabled | none | `text_faint` | no hover, no pointer cursor |
| keyboard focus | resting/selected fill + 1.5 px `focus` ring | as above | ring never suppressed (F6/Tab paths) |

Motion: fill transitions use `Style::animation_time` (0.12 comfortable, 0.10
compact, 0.0 under reduced motion), the same hover animation every widget uses.

## Tooltip (`on_hover`)

Shown after egui's standard hover delay, in a `bg_raised` popover at
`RADIUS_LG`, `shadow_popup`:

1. **Name**: `label`, Body size, `text`.
2. **kbd chip** (only when `shortcut` is set): the hint text in a `bg_input`
   chip, `RADIUS_SM`, `border` hairline, Small size, `text_weak`. Sits inline
   to the right of the name or wraps below on a narrow tooltip.
3. **Description** (only when `description` is set): one line, Small size,
   `text_weak`, below the name row.

A tooltip with no shortcut and no description degrades to the bare name, so
every IconButton always has at least an accessible name (catalog 25: no icon is
mute).

## Accessibility

- `label` is the accessible name (egui AccessKit); the glyph is decorative and
  is never the only source of meaning.
- Selected toggles expose their on/off state through the `selected` fill and the
  response, so a toggle is distinguishable without color alone (the fill change
  is paired with the tooltip name stating the action).
- Focus ring contrast (`focus` on `bg_panel`, 7.35:1) and every glyph/fill pair
  above are the token pairs `theme/contrast.rs` already proves.

## Out of scope (so 1C does not over-build)

- Text-plus-icon buttons (primary/secondary/ghost/danger with labels) are
  separate components in the same module; IconButton is icon-only.
- Command dispatch, chord resolution, and menu placement are lane 3C's registry;
  IconButton only renders and reports clicks.
- The glyph constants themselves are generated into `theme/icons.rs` by lane 1B;
  callers pass a `char`, so the two land independently.

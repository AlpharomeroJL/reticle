# 0095, semantic token system in a theme module, one dark theme this packet

## Context

Through v8.0.0 the app never customized egui: one set_visuals call, stock
fonts, and roughly 89 hardcoded color and size literals across app.rs,
xsection.rs, and tech_editor.rs. The v8.1 packet demands one source of visual
truth with WCAG AA contrast proven by tests, density modes, and a lint that
makes drift impossible. A separate reticle-theme crate was considered; egui
0.35 also changed the styling surface (per-theme styles via set_style_of,
CornerRadius instead of rounding).

## Decision

- A module, not a crate: crates/reticle-app/src/theme/ (tokens, apply,
  contrast, icons, components, gallery). Tokens are egui types with exactly one
  consumer; a crate boundary would add workspace surface for no isolation win.
  The check-style lint excludes this path, making it the only legal home for
  color and size values.
- Semantic tokens per docs/design/tokens.md, mapped once onto egui::Style and
  Visuals starting from Visuals::dark(); applied at boot via set_style_of and
  re-applied only on a dirty flag (density, reduced motion). Density modes
  remap Spacing and text_styles; reduced motion zeroes animation_time.
- Contrast is proven by unit tests in theme/contrast.rs (WCAG relative
  luminance, no external crate); the pair table in tokens.md is the test data.
- One dark theme this packet. The stock-egui Light toggle is removed from the
  UI rather than shipped untokened beside a tokened dark; the Theme enum stays
  and session files with theme=light keep parsing (resolve to Dark). A future
  light theme is a second token table, not an architecture change.

## Consequences

- Every color or size change is a one-file edit with CI-proven contrast; the
  89 legacy literals drain to zero in lane 1A and the ratchet baseline is then
  deleted (ADR 0098).
- Removing the light toggle is a user-visible regression for anyone who used
  it; it is recorded in the honest-limits ledger at release with the
  rationale (consistency over an untokened secondary theme, tokens make light
  cheap later).
- Canvas data colors (layers, DRC heatmap, diff, presence) stay owned by the
  technology table and overlay code; chrome tokens and canvas tokens are
  distinct namespaces in tokens.rs so data color never masquerades as chrome.

# 0097, typography (Inter + JetBrains Mono) and the Lucide icon font

## Context

The app ships egui's default fonts and has no icons; buttons are words and
ASCII glyphs. The packet requires a UI face plus a mono face for coordinates,
one icon set with its license recorded, everything subset into the wasm bundle
within a +450 KB gzipped budget. egui-phosphor (the ready-made icon crate) caps
at egui ^0.34 and the workspace is on 0.35; git dependencies are against repo
policy. ab_glyph 0.2 (egui's rasterizer) renders only a variable font's default
instance, so static instances are required.

## Decision

- Faces: Inter Regular and Medium (SIL OFL 1.1) as the UI family, JetBrains
  Mono Regular (SIL OFL 1.1) as the monospace family for coordinates, readouts,
  and code. Static TTF instances, subset with pyftsubset to an explicit unicode
  range (latin plus punctuation, arrows, geometric glyphs; kern, liga, tnum
  retained; hinting dropped since egui ignores it).
- Icons: the Lucide icon font, ISC license, vendored and subset to the glyphs
  the IA inventory needs; a generator writes theme/icons.rs constants from the
  codepoint map. One set, no emoji in chrome, no mixed sets.
- Committed artifacts: the subset TTFs under crates/reticle-app/assets/fonts/
  plus scripts/subset-fonts.ps1 to regenerate them (fonttools via pip; a
  documented dev-time tool, never a build or CI dependency).
- Installation: FontDefinitions at boot; Proportional = inter-r then lucide,
  Monospace = jbmono then lucide, Name("ui-medium") = inter-m then lucide, with
  a FontTweak aligning icon baselines. Lucide as a fallback family means any
  label can inline a glyph constant.
- Offset attempt: disable epaint's default_fonts feature (eframe
  default-features off with the needed features re-enabled) to drop roughly
  1.38 MiB of raw embedded defaults; if the native build objects (the
  winit/default re-enable caveat), keep default_fonts and record it, since the
  budget gate measures the delta either way.

## Consequences

- The bundle ledger (ADR 0098) gets a measured row from lane 1B; the +450 KB
  gz budget is asserted at every gate, so font growth cannot land silently.
- Rendering text with subset fonts means unsubsetted glyphs fall back to
  Lucide then to nothing; the subset list is therefore part of the reviewable
  diff and extending it is a one-script rerun.
- License obligations (OFL, ISC) are recorded here and in the design brief;
  the font files carry their license notices alongside.

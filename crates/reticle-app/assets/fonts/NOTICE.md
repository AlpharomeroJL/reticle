# Embedded fonts (ADR 0097)

These are subset copies of upstream fonts, embedded into the app at boot by
`crates/reticle-app/src/theme/fonts.rs`. Each is subset with pyftsubset to the
Reticle chrome's unicode range (text faces) or glyph list (Lucide) by
`scripts/subset-fonts.ps1`; the originals are not committed. Regenerating from
the pinned upstream releases with the same fonttools version produces
byte-identical files.

| file | upstream | version | license |
|---|---|---|---|
| `inter-regular.subset.ttf` | rsms/inter, `extras/ttf/Inter-Regular.ttf` | v4.1 | SIL OFL 1.1 (`Inter-OFL.txt`) |
| `inter-medium.subset.ttf` | rsms/inter, `extras/ttf/Inter-Medium.ttf` | v4.1 | SIL OFL 1.1 (`Inter-OFL.txt`) |
| `jetbrains-mono-regular.subset.ttf` | JetBrains/JetBrainsMono, `fonts/ttf/JetBrainsMono-Regular.ttf` | v2.304 | SIL OFL 1.1 (`JetBrainsMono-OFL.txt`) |
| `lucide.subset.ttf` | lucide-icons/lucide, `lucide-font/lucide.ttf` | 1.23.0 | ISC (`Lucide-LICENSE.txt`) |

The Lucide glyph set is defined by `scripts/lucide-glyphs.txt`; the generated
constants are `crates/reticle-app/src/theme/icons.rs`.

The SIL Open Font License permits embedding, subsetting, and redistribution with
the license notice retained (the `*-OFL.txt` files above); the Reserved Font
Name clause is respected because the subsets are used only inside the app and are
not distributed as installable, renamed system fonts. The ISC license permits
redistribution with its copyright notice (`Lucide-LICENSE.txt`).

//! WCAG contrast proofs for the token palette.
//!
//! The design brief's seventh principle is "dark-optimized with contrast
//! proven, not eyeballed": every text and UI pair in `docs/design/tokens.md`
//! carries a measured contrast ratio, and this module re-proves each one in CI
//! so a token nudge that quietly breaks legibility fails the build. The math is
//! the WCAG 2.x relative-luminance ratio computed from first principles (sRGB
//! linearization then the luminance weights); no external crate is pulled in
//! for four lines of arithmetic.

use eframe::egui::Color32;

/// The WCAG relative luminance of an opaque sRGB color, in `[0.0, 1.0]`.
///
/// Each 8-bit channel is normalized to `[0, 1]`, linearized with the sRGB
/// transfer function (the `0.03928` knee is the WCAG-specified constant), then
/// combined with the luminance weights `0.2126 / 0.7152 / 0.0722`.
///
/// `Color32` stores premultiplied alpha, so this is only meaningful for fully
/// opaque colors; the contrast tests assert `a() == 255` before calling it.
#[must_use]
pub fn relative_luminance(c: Color32) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(c.r()) + 0.7152 * linearize(c.g()) + 0.0722 * linearize(c.b())
}

/// The WCAG contrast ratio between two opaque colors, always `>= 1.0`.
///
/// The ratio is `(L_light + 0.05) / (L_dark + 0.05)`; order of the arguments
/// does not matter because the lighter luminance is always placed on top.
#[must_use]
pub fn contrast_ratio(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::DARK;

    /// The `docs/design/tokens.md` contrast table, re-proved here. Each row is
    /// `(label, foreground, background, documented_ratio, minimum)`. The test
    /// asserts the computed ratio meets the minimum (the accessibility floor)
    /// and matches the documented number (so the table stays measured, not
    /// aspirational); a token change that moves a ratio must update tokens.md
    /// in the same commit.
    // The documented ratio 6.28 (text_weak on bg_raised) trips the approx-TAU
    // lint; it is a measured contrast number, not the constant.
    #[allow(clippy::approx_constant)]
    fn table() -> Vec<(&'static str, Color32, Color32, f64, f64)> {
        let t = DARK;
        vec![
            ("text on bg_panel", t.text, t.bg_panel, 14.06, 4.5),
            ("text on bg_raised", t.text, t.bg_raised, 13.06, 4.5),
            ("text on bg_input", t.text, t.bg_input, 15.20, 4.5),
            ("text on widget_bg", t.text, t.widget_bg, 12.01, 4.5),
            ("text on widget_hover", t.text, t.widget_hover, 10.63, 4.5),
            ("text on widget_active", t.text, t.widget_active, 9.30, 4.5),
            ("text on accent_muted", t.text, t.accent_muted, 8.21, 4.5),
            ("text_weak on bg_panel", t.text_weak, t.bg_panel, 6.76, 3.0),
            (
                "text_weak on bg_raised",
                t.text_weak,
                t.bg_raised,
                6.28,
                3.0,
            ),
            ("accent on bg_panel", t.accent, t.bg_panel, 7.35, 4.5),
            ("accent_text on accent", t.accent_text, t.accent, 7.74, 4.5),
            ("danger on bg_panel", t.danger, t.bg_panel, 6.22, 4.5),
            ("warning on bg_panel", t.warning, t.bg_panel, 10.28, 4.5),
            ("success on bg_panel", t.success, t.bg_panel, 6.24, 4.5),
            ("focus on bg_panel", t.focus, t.bg_panel, 7.35, 3.0),
            (
                "border_strong on bg_panel",
                t.border_strong,
                t.bg_panel,
                1.79,
                1.5,
            ),
            (
                "text_faint on bg_panel",
                t.text_faint,
                t.bg_panel,
                3.62,
                2.0,
            ),
        ]
    }

    #[test]
    fn tested_pairs_are_opaque() {
        // Color32 is premultiplied; the luminance math is only valid on opaque
        // colors, so every tested token must ship fully opaque.
        for (label, fg, bg, _, _) in table() {
            assert_eq!(fg.a(), 255, "{label}: foreground not opaque");
            assert_eq!(bg.a(), 255, "{label}: background not opaque");
        }
    }

    #[test]
    fn ratios_meet_wcag_minimums() {
        for (label, fg, bg, _, min) in table() {
            let ratio = contrast_ratio(fg, bg);
            assert!(
                ratio >= min,
                "{label}: contrast {ratio:.2}:1 is below the {min:.1}:1 floor"
            );
        }
    }

    #[test]
    fn measured_ratios_match_the_documented_table() {
        // Honesty check: the number printed in tokens.md must be what the code
        // computes, rounded to two decimals. If a token is nudged, this fails
        // until tokens.md is corrected in the same commit.
        for (label, fg, bg, documented, _) in table() {
            let ratio = contrast_ratio(fg, bg);
            assert!(
                (ratio - documented).abs() < 0.01,
                "{label}: computed {ratio:.4}:1 but tokens.md says {documented:.2}:1"
            );
        }
    }

    #[test]
    fn ratio_is_symmetric_and_at_least_one() {
        let a = DARK.text;
        let b = DARK.bg_panel;
        assert!((contrast_ratio(a, b) - contrast_ratio(b, a)).abs() < f64::EPSILON);
        assert!(contrast_ratio(a, a) >= 1.0 - f64::EPSILON);
        assert!((contrast_ratio(a, a) - 1.0).abs() < 1e-9);
    }
}

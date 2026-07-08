//! The design system's single source of visual truth (v8.1 packet, ADR 0095).
//!
//! Every color, size, radius, spacing, and font the chrome uses comes from this
//! module; `scripts/check-style.ps1` bans the corresponding literals everywhere
//! else in this crate and in `web`, so drift is a CI failure. The
//! human-readable specification these values encode is `docs/design/tokens.md`,
//! including the WCAG contrast table that [`tokens`]' values must keep passing.
//!
//! Layout of the module as the Wave 1 lanes fill it in:
//!
//! * [`tokens`], the semantic color/spacing/radius/type data (this commit).
//! * [`gallery`], the hidden component gallery rendered by `?gallery=1` and
//!   `--gallery`; its signature is frozen here so the visual-regression
//!   harness compiles against it while the component library lands (1C/1D).
//! * `apply` (lane 1A), mapping tokens onto `egui::Style`/`Visuals` per theme
//!   and density, applied once at boot.
//! * `contrast` (lane 1A), the WCAG relative-luminance proofs as unit tests.
//! * `fonts` (lane 1B), embedded subset faces installed via `FontDefinitions`.
//! * `icons` (lane 1B), generated Lucide glyph constants.
//! * `components` (lane 1C), the widget library every panel builds from.

pub mod apply;
pub mod components;
pub mod contrast;
pub mod gallery;
pub mod tokens;

/// The color theme selection, persisted with the session.
///
/// v8.1 ships a single tokened dark theme (ADR 0095); the stock-egui light
/// toggle is gone. The enum stays so a future light variant is a second token
/// table rather than an architecture change, and so session files written by
/// v8.0 that carry `theme=light` keep parsing. Any value other than `Dark`
/// resolves to the dark style at apply time (see [`apply`]); [`Theme::Light`]
/// therefore exists only to keep the persisted tag round-tripping and does not
/// change what the user sees this packet.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    /// The dark visuals (the only rendered theme this packet).
    #[default]
    Dark,
    /// A retired selection kept for tag compatibility; resolves to [`Theme::Dark`]
    /// when applied.
    Light,
}

impl Theme {
    /// The stable text tag used when persisting the theme.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    /// Parses a persisted tag, defaulting to [`Theme::Dark`] for anything else.
    ///
    /// The retired `light` tag is preserved so an older session file still
    /// round-trips through [`Theme::tag`]; it resolves to the dark style when
    /// applied.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "light" => Theme::Light,
            _ => Theme::Dark,
        }
    }
}

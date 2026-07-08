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

pub mod components;
pub mod gallery;
pub mod tokens;

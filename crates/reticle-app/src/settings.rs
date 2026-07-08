//! User settings that are not view state: the pointer and interaction preferences
//! surfaced by the Help > Settings dialog (catalog 98).
//!
//! Density and reduced-motion already live on [`SessionState`](crate::session)
//! because the theme reads them at boot; this module adds the two remaining
//! interaction preferences the Settings dialog persists, wheel behavior and touch
//! mode, plus the small pure helpers the dialog needs. The dialog rendering and the
//! persistence wiring live in [`crate::app`] and [`crate::session`]; everything here
//! is `egui`-free and unit-tested so the tags round-trip.
//!
//! ## Ownership
//!
//! Lane 4C owns the *setting* (its value, its persisted tag, and the dialog control
//! that flips it). The *effect* is owned elsewhere: the canvas (lane 3A) reads
//! [`WheelBehavior`] to decide whether an unmodified wheel zooms or pans, and the
//! touch layer (lane 4B) reads [`TouchMode`] to force or suppress the enlarged
//! touch targets. Both consumers read the persisted value; this lane only stores it.

/// What an unmodified mouse-wheel / trackpad scroll does over the canvas.
///
/// The modifier-held gesture is unaffected (holding Ctrl always zooms, Shift always
/// pans horizontally); this only picks the default for a bare wheel. Lane 3A's
/// canvas reads this to route the gesture.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WheelBehavior {
    /// A bare wheel zooms toward the cursor (the CAD default).
    #[default]
    Zoom,
    /// A bare wheel pans the view; zoom then needs the Ctrl modifier.
    Pan,
}

impl WheelBehavior {
    /// The stable text tag used when persisting the preference.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Zoom => "zoom",
            Self::Pan => "pan",
        }
    }

    /// Parses a persisted tag, defaulting to [`WheelBehavior::Zoom`] for anything
    /// unrecognized so a corrupt or newer file still loads.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "pan" => Self::Pan,
            _ => Self::Zoom,
        }
    }

    /// A short human label for the dialog's segmented control.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Zoom => "Zoom",
            Self::Pan => "Pan",
        }
    }

    /// The two behaviors in dialog order, for building the segmented control.
    #[must_use]
    pub fn all() -> [Self; 2] {
        [Self::Zoom, Self::Pan]
    }
}

/// Whether the enlarged touch targets and touch gestures are forced on, forced
/// off, or left to auto-detection of a coarse pointer.
///
/// Lane 4B owns the touch target sizing and gesture handling; this preference only
/// tells it which mode to assume. `Auto` (the default) follows the platform's
/// coarse-pointer signal; `On`/`Off` override it for a hybrid device whose signal
/// is wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TouchMode {
    /// Follow the platform coarse-pointer detection (the default).
    #[default]
    Auto,
    /// Always use the enlarged touch targets and touch gestures.
    On,
    /// Never use them, even on a touch device.
    Off,
}

impl TouchMode {
    /// The stable text tag used when persisting the preference.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    /// Parses a persisted tag, defaulting to [`TouchMode::Auto`] for anything
    /// unrecognized.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "on" => Self::On,
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }

    /// A short human label for the dialog's segmented control.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::On => "On",
            Self::Off => "Off",
        }
    }

    /// The three modes in dialog order, for building the segmented control.
    #[must_use]
    pub fn all() -> [Self; 3] {
        [Self::Auto, Self::On, Self::Off]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_tags_round_trip() {
        for w in WheelBehavior::all() {
            assert_eq!(WheelBehavior::from_tag(w.tag()), w);
            assert!(!w.label().is_empty());
        }
    }

    #[test]
    fn wheel_defaults_to_zoom() {
        assert_eq!(WheelBehavior::default(), WheelBehavior::Zoom);
        assert_eq!(WheelBehavior::from_tag("wombat"), WheelBehavior::Zoom);
        assert_eq!(WheelBehavior::from_tag(" PAN "), WheelBehavior::Pan);
    }

    #[test]
    fn touch_tags_round_trip() {
        for m in TouchMode::all() {
            assert_eq!(TouchMode::from_tag(m.tag()), m);
            assert!(!m.label().is_empty());
        }
    }

    #[test]
    fn touch_defaults_to_auto() {
        assert_eq!(TouchMode::default(), TouchMode::Auto);
        assert_eq!(TouchMode::from_tag("nonsense"), TouchMode::Auto);
        assert_eq!(TouchMode::from_tag("ON"), TouchMode::On);
        assert_eq!(TouchMode::from_tag("Off"), TouchMode::Off);
    }
}

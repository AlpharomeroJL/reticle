//! First-class labels and pins.
//!
//! GDSII TEXT records carry net and port names that the base geometry model
//! dropped on import. [`Label`] makes them first-class so they round-trip through
//! IO and seed connectivity intent. [`Pin`] names a terminal region: a labeled
//! area on a layer that a net connects to, used by intent verification and by the
//! standard-cell importer (which derives pins from pin-purpose datatype shapes).

use reticle_geometry::{LayerId, Point, Rect};

/// Which reference point of a label's box its [`Label::position`] denotes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[non_exhaustive]
pub enum Anchor {
    /// The center of the label.
    #[default]
    Center,
    /// The lower-left corner.
    SouthWest,
    /// The lower-right corner.
    SouthEast,
    /// The upper-left corner.
    NorthWest,
    /// The upper-right corner.
    NorthEast,
}

/// A text label placed on a layer at a point. Surfaces a GDSII TEXT record.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Label {
    /// The label text (typically a net or port name).
    pub text: String,
    /// Where the label sits, in database units.
    pub position: Point,
    /// The layer and datatype the label is on (usually a label-purpose datatype).
    pub layer: LayerId,
    /// Which point of the label box `position` denotes.
    pub anchor: Anchor,
}

impl Label {
    /// A center-anchored label with the given text, position, and layer.
    #[must_use]
    pub fn new(text: impl Into<String>, position: Point, layer: LayerId) -> Self {
        Self {
            text: text.into(),
            position,
            layer,
            anchor: Anchor::Center,
        }
    }
}

/// The signal direction a pin carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[non_exhaustive]
pub enum PinDirection {
    /// Bidirectional or unspecified (the default).
    #[default]
    Inout,
    /// An input terminal.
    Input,
    /// An output terminal.
    Output,
}

/// A named terminal: a labeled region on a layer that a net connects to.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Pin {
    /// The pin (net or port) name.
    pub name: String,
    /// The terminal region, in database units.
    pub region: Rect,
    /// The layer and datatype the terminal is on.
    pub layer: LayerId,
    /// The signal direction.
    pub direction: PinDirection,
}

impl Pin {
    /// An `Inout` pin with the given name, region, and layer.
    #[must_use]
    pub fn new(name: impl Into<String>, region: Rect, layer: LayerId) -> Self {
        Self {
            name: name.into(),
            region,
            layer,
            direction: PinDirection::Inout,
        }
    }
}

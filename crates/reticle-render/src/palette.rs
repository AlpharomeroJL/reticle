//! Layer color resolution.
//!
//! Colors are packed `0xRRGGBBAA` (see [`reticle_model::LayerInfo::color_rgba`]).
//! The renderer writes to a non-sRGB `Rgba8Unorm` target, so a packed byte value
//! `v` becomes `v / 255` in the shader and reads back as exactly `v`; this keeps
//! golden pixel tests exact. Layers without an entry in the technology table fall
//! back to a small fixed palette keyed by layer id.

use reticle_geometry::LayerId;
use reticle_model::Technology;

/// A linear RGBA color with components in `[0, 1]`, ready to upload to the GPU.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba {
    /// Red, green, blue, alpha in `[0, 1]`.
    pub components: [f32; 4],
}

impl Rgba {
    /// Builds a color from a packed `0xRRGGBBAA` value.
    #[must_use]
    pub fn from_packed(packed: u32) -> Self {
        let r = ((packed >> 24) & 0xff) as f32 / 255.0;
        let g = ((packed >> 16) & 0xff) as f32 / 255.0;
        let b = ((packed >> 8) & 0xff) as f32 / 255.0;
        let a = (packed & 0xff) as f32 / 255.0;
        Self {
            components: [r, g, b, a],
        }
    }

    /// The packed `0xRRGGBBAA` byte value this color rounds to. Inverse of
    /// [`Rgba::from_packed`] for the byte-exact values the palette produces.
    #[must_use]
    pub fn to_packed(self) -> u32 {
        let quantize = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u32;
        let [red, green, blue, alpha] = self.components;
        (quantize(red) << 24) | (quantize(green) << 16) | (quantize(blue) << 8) | quantize(alpha)
    }
}

/// A fixed fallback palette (opaque) for layers absent from the technology table.
/// Indexed by `layer_id % LEN`.
const FALLBACK: [u32; 8] = [
    0x1f77_b4ff, // blue
    0xff7f_0eff, // orange
    0x2ca0_2cff, // green
    0xd627_28ff, // red
    0x9467_bdff, // purple
    0x8c56_4bff, // brown
    0xe377_c2ff, // pink
    0x7f7f_7fff, // gray
];

/// Resolves layer ids to colors using a technology's layer table, falling back to
/// a fixed palette for unknown layers.
#[derive(Clone, Debug)]
pub struct Palette {
    entries: Vec<(LayerId, Rgba, bool)>,
}

impl Palette {
    /// Builds a palette from a technology's layer table.
    #[must_use]
    pub fn from_technology(tech: &Technology) -> Self {
        let entries = tech
            .layers
            .iter()
            .map(|info| (info.id, Rgba::from_packed(info.color_rgba), info.visible))
            .collect();
        Self { entries }
    }

    /// Returns whether the given layer should be drawn. Layers listed in the
    /// technology honor their `visible` flag; unknown layers default to visible.
    #[must_use]
    pub fn is_visible(&self, layer: LayerId) -> bool {
        self.entries
            .iter()
            .find(|(id, _, _)| *id == layer)
            .is_none_or(|(_, _, visible)| *visible)
    }

    /// The color for a layer: its technology entry if present, else a fallback
    /// keyed by layer id.
    #[must_use]
    pub fn color(&self, layer: LayerId) -> Rgba {
        self.entries
            .iter()
            .find(|(id, _, _)| *id == layer)
            .map_or_else(|| Self::fallback(layer), |(_, color, _)| *color)
    }

    /// The fallback color for a layer id, keyed by its layer and datatype numbers.
    #[must_use]
    pub fn fallback(layer: LayerId) -> Rgba {
        let key = usize::from(layer.layer) ^ (usize::from(layer.datatype) << 3);
        Rgba::from_packed(FALLBACK[key % FALLBACK.len()])
    }
}

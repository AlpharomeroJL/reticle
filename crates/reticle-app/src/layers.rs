//! Layer visibility state and filtering for the layer-manager panel.
//!
//! [`LayerState`] mirrors the document's technology layer table with a per-layer
//! visibility flag the user can toggle, plus a text filter that narrows which
//! layers the panel shows. Visibility here is what the canvas honors when culling -
//! a hidden layer's shapes are skipped entirely. Keeping it separate from the
//! document means toggling a layer is a cheap view-only operation that never
//! mutates (or undoes into) the model.

use reticle_geometry::LayerId;
use reticle_model::Technology;
use std::collections::HashMap;

/// One layer's display row: identity, name, color, and current visibility.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LayerRow {
    /// The layer/datatype identifier.
    pub id: LayerId,
    /// Human-readable layer name.
    pub name: String,
    /// Packed `0xRRGGBBAA` display color.
    pub color_rgba: u32,
    /// Whether the layer is currently drawn.
    pub visible: bool,
}

/// The layer table plus a text filter, driving the layer-manager side panel.
#[derive(Clone, Debug, Default)]
pub struct LayerState {
    rows: Vec<LayerRow>,
    /// Fast lookup from layer id to its index in `rows`.
    by_id: HashMap<LayerId, usize>,
    /// Case-insensitive substring filter over layer names (empty = show all).
    filter: String,
}

impl LayerState {
    /// Builds layer state from a document [`Technology`] table.
    ///
    /// Each layer's initial visibility is taken from its [`reticle_model::LayerInfo`].
    #[must_use]
    pub fn from_technology(tech: &Technology) -> Self {
        let rows: Vec<LayerRow> = tech
            .layers
            .iter()
            .map(|l| LayerRow {
                id: l.id,
                name: l.name.clone(),
                color_rgba: l.color_rgba,
                visible: l.visible,
            })
            .collect();
        let by_id = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        Self {
            rows,
            by_id,
            filter: String::new(),
        }
    }

    /// All layer rows, in technology order.
    #[must_use]
    pub fn rows(&self) -> &[LayerRow] {
        &self.rows
    }

    /// Mutable access to all layer rows (for the panel's checkboxes).
    pub fn rows_mut(&mut self) -> &mut [LayerRow] {
        &mut self.rows
    }

    /// The current name filter.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Mutable access to the name filter (bound to the panel's text field).
    pub fn filter_mut(&mut self) -> &mut String {
        &mut self.filter
    }

    /// Whether `layer` is currently visible.
    ///
    /// Unknown layers (not in the technology table) are treated as visible so
    /// stray geometry is never silently hidden.
    #[must_use]
    pub fn is_visible(&self, layer: LayerId) -> bool {
        self.by_id.get(&layer).is_none_or(|&i| self.rows[i].visible)
    }

    /// Sets the visibility of `layer`, returning `true` if the layer was found.
    pub fn set_visible(&mut self, layer: LayerId, visible: bool) -> bool {
        if let Some(&i) = self.by_id.get(&layer) {
            self.rows[i].visible = visible;
            true
        } else {
            false
        }
    }

    /// Toggles the visibility of `layer`, returning the new state if found.
    pub fn toggle(&mut self, layer: LayerId) -> Option<bool> {
        let &i = self.by_id.get(&layer)?;
        self.rows[i].visible = !self.rows[i].visible;
        Some(self.rows[i].visible)
    }

    /// Sets every layer visible.
    pub fn show_all(&mut self) {
        for r in &mut self.rows {
            r.visible = true;
        }
    }

    /// Sets every layer hidden.
    pub fn hide_all(&mut self) {
        for r in &mut self.rows {
            r.visible = false;
        }
    }

    /// The indices of the rows matching the current name filter, in table order.
    ///
    /// An empty filter matches every row; otherwise the match is a case-insensitive
    /// substring of the layer name.
    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.rows.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }
}

impl crate::inspector::LayerNamer for LayerState {
    fn layer_name(&self, layer: LayerId) -> Option<String> {
        self.by_id.get(&layer).map(|&i| self.rows[i].name.clone())
    }
}

/// Splits a packed `0xRRGGBBAA` color into its `(r, g, b, a)` byte components.
#[must_use]
pub fn rgba_components(color_rgba: u32) -> (u8, u8, u8, u8) {
    let r = (color_rgba >> 24) as u8;
    let g = (color_rgba >> 16) as u8;
    let b = (color_rgba >> 8) as u8;
    let a = color_rgba as u8;
    (r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn state() -> LayerState {
        LayerState::from_technology(&demo::demo_technology())
    }

    #[test]
    fn all_layers_start_visible() {
        let s = state();
        assert!(!s.rows().is_empty());
        for r in s.rows() {
            assert!(s.is_visible(r.id));
        }
    }

    #[test]
    fn toggle_hides_layer_and_affects_visibility() {
        let mut s = state();
        let id = s.rows()[0].id;
        assert_eq!(s.toggle(id), Some(false));
        assert!(!s.is_visible(id));
        assert_eq!(s.toggle(id), Some(true));
        assert!(s.is_visible(id));
    }

    #[test]
    fn unknown_layer_is_visible_by_default() {
        let s = state();
        assert!(s.is_visible(LayerId::new(999, 7)));
        // ...and toggling it reports not-found.
        let mut s = s;
        assert!(s.toggle(LayerId::new(999, 7)).is_none());
        assert!(!s.set_visible(LayerId::new(999, 7), false));
    }

    #[test]
    fn show_and_hide_all() {
        let mut s = state();
        s.hide_all();
        assert!(s.rows().iter().all(|r| !r.visible));
        s.show_all();
        assert!(s.rows().iter().all(|r| r.visible));
    }

    #[test]
    fn filter_narrows_rows() {
        let mut s = state();
        assert_eq!(s.filtered_indices().len(), s.rows().len());
        *s.filter_mut() = "metal".to_owned();
        let hits = s.filtered_indices();
        assert_eq!(hits.len(), 2); // METAL1, METAL2
        for i in hits {
            assert!(s.rows()[i].name.to_lowercase().contains("metal"));
        }
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut s = state();
        *s.filter_mut() = "PoLy".to_owned();
        assert_eq!(s.filtered_indices().len(), 1);
    }

    #[test]
    fn rgba_components_unpack() {
        assert_eq!(rgba_components(0x11_22_33_44), (0x11, 0x22, 0x33, 0x44));
        assert_eq!(rgba_components(0xFF_00_80_FF), (0xFF, 0x00, 0x80, 0xFF));
    }
}

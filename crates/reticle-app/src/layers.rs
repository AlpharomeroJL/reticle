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

/// How a layer's shapes are drawn in the layer-manager preview and the canvas
/// legend: filled, cross-hatched, or outline only.
///
/// This is view-only display metadata, chosen per layer in the layer manager. It
/// never leaves the app (the GPU palette keys on color and visibility), so a new
/// variant here is safe to add. Defaults to [`FillStyle::Solid`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FillStyle {
    /// Fully filled with the layer color. The default.
    #[default]
    Solid,
    /// Cross-hatched: the layer color drawn as diagonal lines over a clear body.
    Hatch,
    /// Outline only: the border in the layer color, no fill.
    Outline,
}

impl FillStyle {
    /// The three fill styles in menu order, for a picker.
    pub const ALL: [FillStyle; 3] = [FillStyle::Solid, FillStyle::Hatch, FillStyle::Outline];

    /// A short human-readable label for the picker.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FillStyle::Solid => "Solid",
            FillStyle::Hatch => "Hatch",
            FillStyle::Outline => "Outline",
        }
    }
}

/// One layer's display row: identity, name, color, fill style, and visibility.
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
    /// How the layer's shapes are stylized in the manager preview and legend.
    pub fill: FillStyle,
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
                fill: FillStyle::default(),
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

    /// Sets the display color of `layer` to a packed `0xRRGGBBAA` value, returning
    /// `true` if the layer was found.
    ///
    /// The canvas palette re-reads row colors, so a recolor takes effect on the next
    /// retained-scene rebuild.
    pub fn set_color(&mut self, layer: LayerId, color_rgba: u32) -> bool {
        if let Some(&i) = self.by_id.get(&layer) {
            self.rows[i].color_rgba = color_rgba;
            true
        } else {
            false
        }
    }

    /// Sets the [`FillStyle`] of `layer`, returning `true` if the layer was found.
    pub fn set_fill(&mut self, layer: LayerId, fill: FillStyle) -> bool {
        if let Some(&i) = self.by_id.get(&layer) {
            self.rows[i].fill = fill;
            true
        } else {
            false
        }
    }

    /// Shows only `layer` and hides every other, returning `true` if the layer was
    /// found (solo / hide-others).
    ///
    /// A no-op miss (an unknown layer) leaves visibility unchanged rather than
    /// hiding everything, so a stale id never blanks the canvas.
    pub fn solo(&mut self, layer: LayerId) -> bool {
        if !self.by_id.contains_key(&layer) {
            return false;
        }
        for r in &mut self.rows {
            r.visible = r.id == layer;
        }
        true
    }

    /// Moves the row at `index` one position earlier (toward the front of the
    /// table), returning `true` if it moved.
    ///
    /// The first row cannot move up, and an out-of-range index is a no-op. The
    /// id-to-index map is kept in sync so lookups stay correct after the swap.
    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.rows.len() {
            return false;
        }
        self.rows.swap(index - 1, index);
        self.reindex();
        true
    }

    /// Moves the row at `index` one position later (toward the back of the table),
    /// returning `true` if it moved.
    ///
    /// The last row cannot move down, and an out-of-range index is a no-op. The
    /// id-to-index map is kept in sync so lookups stay correct after the swap.
    pub fn move_down(&mut self, index: usize) -> bool {
        if index + 1 >= self.rows.len() {
            return false;
        }
        self.rows.swap(index, index + 1);
        self.reindex();
        true
    }

    /// Rebuilds [`Self::by_id`] from the current row order.
    ///
    /// Called after any reorder so `by_id` maps each layer id to its new index.
    fn reindex(&mut self) {
        self.by_id = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id, i))
            .collect();
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

    #[test]
    fn rows_start_solid_fill() {
        let s = state();
        assert!(s.rows().iter().all(|r| r.fill == FillStyle::Solid));
    }

    #[test]
    fn recolor_updates_row_and_reports_found() {
        let mut s = state();
        let id = s.rows()[0].id;
        assert!(s.set_color(id, 0x0A_0B_0C_0D));
        assert_eq!(s.rows()[0].color_rgba, 0x0A_0B_0C_0D);
        // A miss changes nothing and reports not-found.
        assert!(!s.set_color(LayerId::new(999, 7), 0x11_22_33_44));
    }

    #[test]
    fn set_fill_updates_row_and_reports_found() {
        let mut s = state();
        let id = s.rows()[0].id;
        assert!(s.set_fill(id, FillStyle::Hatch));
        assert_eq!(s.rows()[0].fill, FillStyle::Hatch);
        assert!(s.set_fill(id, FillStyle::Outline));
        assert_eq!(s.rows()[0].fill, FillStyle::Outline);
        assert!(!s.set_fill(LayerId::new(999, 7), FillStyle::Solid));
    }

    #[test]
    fn solo_shows_only_the_chosen_layer() {
        let mut s = state();
        let target = s.rows()[1].id;
        assert!(s.solo(target));
        for r in s.rows() {
            assert_eq!(r.visible, r.id == target);
        }
        assert!(s.is_visible(target));
    }

    #[test]
    fn solo_of_unknown_layer_is_a_noop() {
        let mut s = state();
        // Everything starts visible.
        assert!(s.rows().iter().all(|r| r.visible));
        assert!(!s.solo(LayerId::new(999, 7)));
        // A missed solo must not blank the canvas.
        assert!(s.rows().iter().all(|r| r.visible));
    }

    #[test]
    fn move_up_and_down_reorder_and_keep_lookup_in_sync() {
        let mut s = state();
        let original: Vec<LayerId> = s.rows().iter().map(|r| r.id).collect();
        assert!(
            original.len() >= 3,
            "demo tech has enough layers to reorder"
        );

        // Row 0 cannot move up; the last row cannot move down.
        assert!(!s.move_up(0));
        assert!(!s.move_down(s.rows().len() - 1));

        // Move row 1 up: rows 0 and 1 swap.
        assert!(s.move_up(1));
        assert_eq!(s.rows()[0].id, original[1]);
        assert_eq!(s.rows()[1].id, original[0]);

        // Visibility lookups still resolve to the moved rows.
        let moved = original[1];
        assert_eq!(s.toggle(moved), Some(false));
        assert!(!s.is_visible(moved));

        // Move it back down and confirm the order is restored.
        assert!(s.move_down(0));
        assert_eq!(s.rows()[0].id, original[0]);
        assert_eq!(s.rows()[1].id, original[1]);
    }

    #[test]
    fn move_out_of_range_is_a_noop() {
        let mut s = state();
        let before: Vec<LayerId> = s.rows().iter().map(|r| r.id).collect();
        assert!(!s.move_up(s.rows().len() + 5));
        assert!(!s.move_down(s.rows().len() + 5));
        let after: Vec<LayerId> = s.rows().iter().map(|r| r.id).collect();
        assert_eq!(before, after);
    }
}

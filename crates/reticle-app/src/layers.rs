//! Layer visibility state and filtering for the layer-manager panel.
//!
//! [`LayerState`] mirrors the document's technology layer table with a per-layer
//! visibility flag the user can toggle, plus a text filter that narrows which
//! layers the panel shows. Visibility here is what the canvas honors when culling -
//! a hidden layer's shapes are skipped entirely. Keeping it separate from the
//! document means toggling a layer is a cheap view-only operation that never
//! mutates (or undoes into) the model.

use reticle_geometry::LayerId;
use reticle_model::{LayerInfo, Technology};
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

/// One layer's display row: identity, name, color, fill style, visibility, and
/// lock state.
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
    /// When locked, the layer's shapes stay drawn but the canvas will not let the
    /// pointer pick them (catalog 57: visible-but-unselectable). View-only state,
    /// never mutating the document.
    pub locked: bool,
}

/// A named snapshot of which layers are hidden, so a user can flip a whole view
/// (catalog 62). Stored by hidden-layer id list so applying it sets every known
/// row's visibility from the saved set; layers not in the table are ignored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisibilityPreset {
    /// The preset's display name (unique within a [`LayerState`]).
    pub name: String,
    /// The ids that were hidden when the preset was saved.
    pub hidden: Vec<LayerId>,
}

/// The layer table plus a text filter, driving the layer-manager side panel.
#[derive(Clone, Debug, Default)]
pub struct LayerState {
    rows: Vec<LayerRow>,
    /// Fast lookup from layer id to its index in `rows`.
    by_id: HashMap<LayerId, usize>,
    /// Case-insensitive substring filter over layer names (empty = show all).
    filter: String,
    /// Saved visibility presets, newest additions at the end (catalog 62).
    presets: Vec<VisibilityPreset>,
}

/// Whether `name` is the synthesized `L{layer}D{datatype}` placeholder a bare GDS/OASIS
/// import assigns to a layer with no technology name (`reticle_io` writes
/// `format!("L{}D{}", layer, datatype)`). A real technology name is a word such as
/// `met1` or `nwell`, never this shape.
fn is_placeholder_layer_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('L') else {
        return false;
    };
    let Some(dpos) = rest.find('D') else {
        return false;
    };
    let (layer, datatype) = (&rest[..dpos], &rest[dpos + 1..]);
    !layer.is_empty()
        && !datatype.is_empty()
        && layer.bytes().all(|b| b.is_ascii_digit())
        && datatype.bytes().all(|b| b.is_ascii_digit())
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
                locked: false,
            })
            .collect();
        let by_id = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        Self {
            rows,
            by_id,
            filter: String::new(),
            presets: Vec::new(),
        }
    }

    // --- lane fix-layer-table-live: keep the table live with edited geometry ---
    /// Appends a row for every id in `drawn` that the table does not already have,
    /// returning whether any row was added.
    ///
    /// A new row is synthesized with [`LayerInfo::placeholder`] semantics: the
    /// `L{layer}D{datatype}` name and the [`reticle_model::fallback_layer_color`] the
    /// renderer already paints such a layer with, so the freshly listed row matches
    /// exactly what is on the canvas. It starts visible, solid, and unlocked.
    ///
    /// Existing rows are left completely untouched, so a surviving layer's user
    /// overrides (color, visibility, lock, fill, table order) are preserved -- this is
    /// the merge the panel needs after an edit changes the drawn layer set, in place of
    /// a wholesale [`Self::from_technology`] rebuild that would clobber them.
    ///
    /// Rows are never dropped here: a layer that loses its last shape keeps its row,
    /// matching the panel's existing "show every technology layer" behavior where an
    /// empty layer still lists. A layer surfaced by an edit thus stays listable exactly
    /// like a technology layer, rather than flickering out of the table when its
    /// geometry is deleted or undone.
    pub fn ensure_rows_for(&mut self, drawn: impl IntoIterator<Item = LayerId>) -> bool {
        let mut added = false;
        for id in drawn {
            if self.by_id.contains_key(&id) {
                continue;
            }
            let info = LayerInfo::placeholder(id);
            self.by_id.insert(id, self.rows.len());
            self.rows.push(LayerRow {
                id,
                name: info.name,
                color_rgba: info.color_rgba,
                visible: info.visible,
                fill: FillStyle::default(),
                locked: false,
            });
            added = true;
        }
        added
    }

    /// All layer rows, in technology order.
    #[must_use]
    pub fn rows(&self) -> &[LayerRow] {
        &self.rows
    }

    /// The number of layer rows with a real technology name, i.e. NOT a synthesized
    /// `L{layer}D{datatype}` placeholder. Zero means the document opened with no named
    /// technology grafted, which renders every layer as an opaque default fill that
    /// overpaints to one blob. Exposed to the browser stats seam so a headed guard can
    /// fail a white-blob example whose layermap was never applied.
    #[must_use]
    pub fn named_layer_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| !is_placeholder_layer_name(&r.name))
            .count()
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

    /// Whether `layer` is locked (drawn but not pointer-selectable).
    ///
    /// Unknown layers are treated as unlocked, so stray geometry stays selectable.
    #[must_use]
    pub fn is_locked(&self, layer: LayerId) -> bool {
        self.by_id.get(&layer).is_some_and(|&i| self.rows[i].locked)
    }

    /// Sets the lock state of `layer`, returning `true` if the layer was found.
    pub fn set_locked(&mut self, layer: LayerId, locked: bool) -> bool {
        if let Some(&i) = self.by_id.get(&layer) {
            self.rows[i].locked = locked;
            true
        } else {
            false
        }
    }

    /// Toggles the lock state of `layer`, returning the new state if found.
    pub fn toggle_lock(&mut self, layer: LayerId) -> Option<bool> {
        let &i = self.by_id.get(&layer)?;
        self.rows[i].locked = !self.rows[i].locked;
        Some(self.rows[i].locked)
    }

    /// The saved visibility presets, in creation order (catalog 62).
    #[must_use]
    pub fn presets(&self) -> &[VisibilityPreset] {
        &self.presets
    }

    /// Saves the current visibility as a preset named `name`, capturing which
    /// layers are hidden right now.
    ///
    /// A blank name is rejected (returns `false`); an existing name is overwritten
    /// in place so re-saving updates rather than duplicates.
    pub fn save_preset(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let hidden: Vec<LayerId> = self
            .rows
            .iter()
            .filter(|r| !r.visible)
            .map(|r| r.id)
            .collect();
        if let Some(existing) = self.presets.iter_mut().find(|p| p.name == name) {
            existing.hidden = hidden;
        } else {
            self.presets.push(VisibilityPreset {
                name: name.to_owned(),
                hidden,
            });
        }
        true
    }

    /// Applies the preset named `name`, setting every known layer visible unless
    /// the preset recorded it hidden. Returns `true` if the preset existed.
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some(preset) = self.presets.iter().find(|p| p.name == name) else {
            return false;
        };
        let hidden = preset.hidden.clone();
        for r in &mut self.rows {
            r.visible = !hidden.contains(&r.id);
        }
        true
    }

    /// Deletes the preset named `name`, returning `true` if one was removed.
    pub fn delete_preset(&mut self, name: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|p| p.name != name);
        self.presets.len() != before
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

/// Packs `(r, g, b, a)` byte components into a `0xRRGGBBAA` color. Inverse of
/// [`rgba_components`]; used when the color editor writes an edited swatch back.
#[must_use]
pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a)
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
    fn named_layer_count_excludes_l_d_placeholders() {
        // The synthesized "L{layer}D{datatype}" shape a bare import assigns is not a
        // named layer; a real technology name (a word) is.
        assert!(is_placeholder_layer_name("L64D5"));
        assert!(is_placeholder_layer_name("L68D20"));
        assert!(!is_placeholder_layer_name("met1"));
        assert!(!is_placeholder_layer_name("nwell"));
        assert!(!is_placeholder_layer_name("li1"));
        assert!(!is_placeholder_layer_name("L")); // no D
        assert!(!is_placeholder_layer_name("LD")); // empty numbers
        assert!(!is_placeholder_layer_name("L6xD3")); // non-digit
        // The demo technology has real, named layers, so the count is positive.
        assert!(state().named_layer_count() > 0);
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
    fn pack_rgba_round_trips_components() {
        for c in [
            0x11_22_33_44u32,
            0xFF_00_80_FF,
            0x00_00_00_00,
            0xDE_AD_BE_EF,
        ] {
            let (r, g, b, a) = rgba_components(c);
            assert_eq!(pack_rgba(r, g, b, a), c);
        }
    }

    #[test]
    fn rows_start_unlocked() {
        let s = state();
        assert!(s.rows().iter().all(|r| !r.locked));
        assert!(!s.is_locked(s.rows()[0].id));
    }

    #[test]
    fn lock_makes_layer_locked_and_reports_found() {
        let mut s = state();
        let id = s.rows()[0].id;
        assert_eq!(s.toggle_lock(id), Some(true));
        assert!(s.is_locked(id));
        assert_eq!(s.toggle_lock(id), Some(false));
        assert!(!s.is_locked(id));
        assert!(s.set_locked(id, true));
        assert!(s.is_locked(id));
        // An unknown layer is unlocked and reports not-found.
        assert!(!s.is_locked(LayerId::new(999, 7)));
        assert!(!s.set_locked(LayerId::new(999, 7), true));
        assert!(s.toggle_lock(LayerId::new(999, 7)).is_none());
    }

    #[test]
    fn locking_does_not_change_visibility() {
        let mut s = state();
        let id = s.rows()[0].id;
        assert!(s.set_locked(id, true));
        assert!(s.is_visible(id), "a locked layer stays drawn");
    }

    #[test]
    fn preset_saves_current_hidden_set_and_reapplies() {
        let mut s = state();
        let first = s.rows()[0].id;
        let second = s.rows()[1].id;
        // Hide the first layer, then snapshot that view.
        s.set_visible(first, false);
        assert!(s.save_preset("just-first-hidden"));
        // Change the view: show all, hide the second instead.
        s.show_all();
        s.set_visible(second, false);
        // Re-applying the preset restores exactly the saved hidden set.
        assert!(s.apply_preset("just-first-hidden"));
        assert!(!s.is_visible(first));
        assert!(s.is_visible(second));
    }

    #[test]
    fn save_preset_overwrites_same_name_and_rejects_blank() {
        let mut s = state();
        let first = s.rows()[0].id;
        s.set_visible(first, false);
        assert!(s.save_preset("view"));
        assert_eq!(s.presets().len(), 1);
        // Re-saving under the same name overwrites in place, not duplicates.
        s.show_all();
        assert!(s.save_preset("view"));
        assert_eq!(s.presets().len(), 1);
        assert!(s.presets()[0].hidden.is_empty());
        // A blank name is rejected.
        assert!(!s.save_preset("   "));
        assert_eq!(s.presets().len(), 1);
    }

    #[test]
    fn delete_and_apply_missing_preset() {
        let mut s = state();
        assert!(s.save_preset("a"));
        assert!(!s.apply_preset("nope"));
        assert!(s.delete_preset("a"));
        assert!(!s.delete_preset("a"));
        assert!(s.presets().is_empty());
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

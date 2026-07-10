//! The DXF layer-map dialog: DXF layer names are not GDS `(layer, datatype)`
//! numbers, so a DXF import needs a place to confirm (or edit) the numbers each
//! name lands on before the document is treated like any other opened layout.
//!
//! # Why this exists
//!
//! Every other format Reticle opens (GDSII, the two OASIS dialects) already
//! carries numeric `(layer, datatype)` pairs on its geometry, so an opened
//! document's layer table is exactly what the file said. DXF has no such
//! numbering: a layer is a name (group code 8), and `reticle_io::dxf::Dxf`
//! assigns each distinct name a synthetic id in first-seen order (see its module
//! docs). That numbering is a reasonable default (shapes stay grouped and
//! distinct), but it is arbitrary: two DXF files that use the same layer names
//! for the same physical layers can still end up with different numbers if the
//! names first appear in a different order. This module lets a user pin the
//! numbers down.
//!
//! # What lives here, and why it is testable
//!
//! Like [`crate::open`] and [`crate::dialogs`], this module keeps a hard line
//! between pure logic and `egui` glue:
//!
//! * **Pure, unit-tested (below, no `egui`):** [`DxfLayerMap`], built from an
//!   opened DXF document's [`Technology`] layer table
//!   ([`DxfLayerMap::from_technology`]), and [`DxfLayerMap::apply`], which
//!   rewrites every shape's and label's layer across the whole document from its
//!   DXF-assigned source id to the user's edited target.
//! * **UI glue ([`show`]):** a modal dialog, styled entirely through
//!   [`crate::theme::components`], with one row per DXF layer name and two
//!   numeric fields (target layer, target datatype). Never called with untrusted
//!   bytes directly, only with a [`DxfLayerMap`] already built from an opened
//!   document, so there is nothing here for a corrupt file to reach.
//!
//! # Using it
//!
//! ```ignore
//! let outcome = open_document_bytes(bytes, DocFormat::Dxf)?;
//! let mut map = DxfLayerMap::from_technology(outcome.document.technology());
//! // ... show the dialog, let the user edit `map.rows`, then on Apply:
//! map.apply(&mut outcome.document);
//! ```
//!
//! [`App::open_dxf_with_layer_map`](crate::app::App::open_dxf_with_layer_map) is
//! the App-level entry point that runs exactly this sequence.

use eframe::egui;
use reticle_geometry::LayerId;
use reticle_model::{Document, Technology};

use crate::theme::components;

/// One row of the layer-map: a DXF layer name, the `(layer, datatype)` the DXF
/// reader assigned it, and the target the user has confirmed or edited.
#[derive(Clone, PartialEq, Debug)]
pub struct DxfLayerRow {
    /// The DXF layer name (group code 8) this row maps.
    pub name: String,
    /// The id [`reticle_io::dxf::Dxf`] assigned this name (first-seen order).
    pub source: LayerId,
    /// The layer number to remap `source` to. Starts equal to `source.layer`.
    pub target_layer: u16,
    /// The datatype to remap `source` to. Starts equal to `source.datatype`.
    pub target_datatype: u16,
}

impl DxfLayerRow {
    /// A row seeded from an opened document's layer-table entry: a pass-through
    /// mapping (target starts equal to source) until edited.
    fn from_layer_info(info: &reticle_model::LayerInfo) -> Self {
        Self {
            name: info.name.clone(),
            source: info.id,
            target_layer: info.id.layer,
            target_datatype: info.id.datatype,
        }
    }

    /// The row's current target as a [`LayerId`].
    #[must_use]
    pub fn target(&self) -> LayerId {
        LayerId::new(self.target_layer, self.target_datatype)
    }

    /// Whether this row's target still equals its DXF-assigned source (no remap
    /// requested for this layer).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.target() == self.source
    }
}

/// The layer-map dialog's state: one row per distinct DXF layer name, built from
/// a just-opened DXF document's [`Technology`] table.
///
/// A fresh map (via [`from_technology`](Self::from_technology)) is always a
/// no-op mapping; [`is_identity`](Self::is_identity) tells a caller when it is
/// safe to skip the dialog entirely (nothing to confirm) and
/// [`apply`](Self::apply) performs the remap the user asked for.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DxfLayerMap {
    /// One row per distinct DXF layer name, in the DXF reader's first-seen
    /// (source id) order.
    pub rows: Vec<DxfLayerRow>,
}

impl DxfLayerMap {
    /// Builds a row per entry in `tech`'s layer table, each defaulting to a
    /// pass-through mapping. Call this on the [`Technology`] of a document just
    /// opened with [`DocFormat::Dxf`](crate::open::DocFormat::Dxf).
    #[must_use]
    pub fn from_technology(tech: &Technology) -> Self {
        Self {
            rows: tech
                .layers
                .iter()
                .map(DxfLayerRow::from_layer_info)
                .collect(),
        }
    }

    /// Whether every row is still a pass-through mapping, so
    /// [`apply`](Self::apply) would be a no-op. A caller can use this to skip
    /// showing the dialog for a DXF file whose layers need no attention (for
    /// instance a document with a single layer).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.rows.iter().all(DxfLayerRow::is_identity)
    }

    /// Rewrites every shape's and label's layer in `doc` from its DXF-assigned
    /// source id to this map's edited target, across every cell.
    ///
    /// A no-op fast path when [`is_identity`](Self::is_identity) holds, so
    /// applying a map the user never touched never walks the document. A layer
    /// id this map does not mention (there is none for a document whose
    /// [`Technology`] came from [`from_technology`](Self::from_technology), but
    /// this stays defensive for any other caller) is left untouched rather than
    /// erroring: an unmapped id is not malformed input, just nothing to do.
    pub fn apply(&self, doc: &mut Document) {
        if self.is_identity() {
            return;
        }
        let table: std::collections::HashMap<LayerId, LayerId> =
            self.rows.iter().map(|r| (r.source, r.target())).collect();
        let remap = |id: &mut LayerId| {
            if let Some(&mapped) = table.get(id) {
                *id = mapped;
            }
        };

        let names: Vec<String> = doc.cells().map(|c| c.name.clone()).collect();
        for name in names {
            let Some(cell) = doc.cell_mut(&name) else {
                continue;
            };
            for shape in &mut cell.shapes {
                remap(&mut shape.layer);
            }
            for label in &mut cell.labels {
                remap(&mut label.layer);
            }
        }

        // Reflect the new ids in the layer table too, so the layer panel shows
        // the numbers the shapes now actually carry.
        let mut tech = doc.technology().clone();
        for info in &mut tech.layers {
            remap(&mut info.id);
        }
        doc.set_technology(tech);
    }
}

/// What the user chose when the layer-map dialog closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DxfDialogChoice {
    /// Apply the (possibly edited) map and install the document.
    Apply,
    /// Discard the dialog without opening the document.
    Cancel,
}

/// Draws the DXF layer-map dialog as a modal over `ctx`: an explanation line,
/// one row per DXF layer name with two numeric fields (target layer, target
/// datatype), and Apply/Cancel actions.
///
/// Returns the user's [`DxfDialogChoice`] on the frame it was made, else `None`
/// (the dialog stays open). Styled entirely through [`components`] (the
/// [`components::Modal`] frame, [`components::Button`]); the two numeric fields are a plain
/// [`egui::DragValue`], the one interactive primitive the component library
/// does not wrap, matching how other dialogs in this codebase compose raw `egui`
/// leaf widgets for input types the library has no recipe for yet.
#[must_use]
pub fn show(
    ctx: &egui::Context,
    cctx: components::Ctx,
    map: &mut DxfLayerMap,
) -> Option<DxfDialogChoice> {
    let mut choice = None;
    components::Modal::new("Map DXF Layers").overlay(ctx, cctx, |ui, cctx| {
        ui.label(
            egui::RichText::new(
                "DXF layer names carry no GDS (layer, datatype) number. Confirm or edit \
                 the target each name opens on.",
            )
            .color(cctx.tokens.text_weak),
        );
        ui.add_space(cctx.density.item_spacing().y);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for row in &mut map.rows {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&row.name).color(cctx.tokens.text));
                        ui.label(egui::RichText::new("layer").color(cctx.tokens.text_weak));
                        ui.add(egui::DragValue::new(&mut row.target_layer));
                        ui.label(egui::RichText::new("datatype").color(cctx.tokens.text_weak));
                        ui.add(egui::DragValue::new(&mut row.target_datatype));
                    });
                }
            });
        ui.add_space(cctx.density.item_spacing().y);
        ui.horizontal(|ui| {
            if components::Button::primary("Apply")
                .show(ui, cctx)
                .clicked()
            {
                choice = Some(DxfDialogChoice::Apply);
            }
            if components::Button::ghost("Cancel").show(ui, cctx).clicked() {
                choice = Some(DxfDialogChoice::Cancel);
            }
        });
    });
    choice
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Path, Point, Rect};
    use reticle_model::{Cell, DrawShape, Label, ShapeKind};

    fn tech_with_two_layers() -> Technology {
        Technology {
            layers: vec![
                reticle_model::LayerInfo {
                    id: LayerId::new(0, 0),
                    name: "WALL".to_owned(),
                    color_rgba: 0xFFFF_FFFF,
                    visible: true,
                },
                reticle_model::LayerInfo {
                    id: LayerId::new(1, 0),
                    name: "DOOR".to_owned(),
                    color_rgba: 0xFFFF_FFFF,
                    visible: true,
                },
            ],
            ..Technology::default()
        }
    }

    #[test]
    fn from_technology_builds_identity_rows_in_encounter_order() {
        let map = DxfLayerMap::from_technology(&tech_with_two_layers());
        assert_eq!(map.rows.len(), 2);
        assert_eq!(map.rows[0].name, "WALL");
        assert_eq!(map.rows[0].source, LayerId::new(0, 0));
        assert_eq!(map.rows[0].target(), LayerId::new(0, 0));
        assert_eq!(map.rows[1].name, "DOOR");
        assert_eq!(map.rows[1].source, LayerId::new(1, 0));
        assert!(map.is_identity(), "a fresh map is always a pass-through");
    }

    #[test]
    fn editing_a_row_breaks_identity() {
        let mut map = DxfLayerMap::from_technology(&tech_with_two_layers());
        map.rows[0].target_layer = 42;
        assert!(!map.is_identity());
        assert!(!map.rows[0].is_identity());
        assert!(
            map.rows[1].is_identity(),
            "the untouched row is still identity"
        );
    }

    fn doc_with_two_layers() -> Document {
        let mut doc = Document::new();
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            LayerId::new(0, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
        ));
        cell.shapes.push(DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(5, 5)],
                0,
                reticle_geometry::Endcap::Flat,
            )),
        ));
        cell.labels.push(Label {
            text: "note".to_owned(),
            position: Point::new(1, 1),
            layer: LayerId::new(0, 0),
            anchor: reticle_model::Anchor::Center,
        });
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc.set_technology(tech_with_two_layers());
        doc
    }

    #[test]
    fn apply_remaps_shapes_and_labels_on_the_edited_layer_only() {
        let mut doc = doc_with_two_layers();
        let mut map = DxfLayerMap::from_technology(doc.technology());
        map.rows[0].target_layer = 9; // WALL: (0,0) -> (9,0)
        map.rows[0].target_datatype = 3;
        map.apply(&mut doc);

        let cell = doc.cell("TOP").expect("cell present");
        assert_eq!(cell.shapes[0].layer, LayerId::new(9, 3), "remapped");
        assert_eq!(cell.shapes[1].layer, LayerId::new(1, 0), "untouched (DOOR)");
        assert_eq!(
            cell.labels[0].layer,
            LayerId::new(9, 3),
            "label remapped too"
        );

        // The technology's own layer table reflects the new id.
        let wall = doc
            .technology()
            .layers
            .iter()
            .find(|l| l.name == "WALL")
            .expect("WALL entry present");
        assert_eq!(wall.id, LayerId::new(9, 3));
    }

    #[test]
    fn apply_is_a_no_op_for_an_identity_map() {
        let mut doc = doc_with_two_layers();
        let before = doc.clone();
        let map = DxfLayerMap::from_technology(doc.technology());
        assert!(map.is_identity());
        map.apply(&mut doc);
        assert_eq!(doc, before, "an untouched map must not change the document");
    }

    #[test]
    fn apply_leaves_an_unmentioned_layer_id_untouched() {
        // Defense in depth: a shape whose layer is not one of the map's rows at
        // all (should not happen for a document built from `from_technology`,
        // but the remap must not panic or clobber it if it ever does).
        let mut doc = Document::new();
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            LayerId::new(77, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1, 1))),
        ));
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc.set_technology(tech_with_two_layers());

        let mut map = DxfLayerMap::from_technology(doc.technology());
        map.rows[0].target_layer = 5; // remaps WALL, not layer 77
        map.apply(&mut doc);

        assert_eq!(
            doc.cell("TOP").unwrap().shapes[0].layer,
            LayerId::new(77, 0)
        );
    }

    #[test]
    fn show_renders_without_panicking_and_reports_no_choice_when_not_clicked() {
        let egui_ctx = egui::Context::default();
        egui_ctx.begin_pass(egui::RawInput::default());
        let cctx = components::Ctx::dark(crate::theme::tokens::Density::Comfortable);
        let mut map = DxfLayerMap::from_technology(&tech_with_two_layers());
        let choice = show(&egui_ctx, cctx, &mut map);
        let _ = egui_ctx.end_pass();
        assert_eq!(choice, None, "laying out the dialog alone clicks nothing");
    }
}

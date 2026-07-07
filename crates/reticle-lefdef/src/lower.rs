//! Lowering parsed LEF/DEF into a [`LefDefDesign`].
//!
//! This is where the two ASTs meet the Reticle model. It resolves the single
//! database resolution shared by both files, builds the [`Technology`] layer table
//! from the LEF layers, lowers each LEF `MACRO` to a [`Cell`], and lowers the DEF
//! placement and routing into a top [`Cell`] plus the run-level metadata a viewer
//! overlays (die area, rows, sites, nets, pins).
//!
//! # Resolution
//!
//! LEF geometry is written in microns and DEF geometry in DBU, but both must land
//! on one grid or the placed cells and the routing would not line up. The grid is
//! `UNITS DISTANCE MICRONS` from the DEF when present, else `DATABASE MICRONS` from
//! the LEF, else [`DEFAULT_DBU_PER_MICRON`]. LEF microns are converted with that
//! resolution; DEF coordinates are used as-is.

use std::collections::HashMap;

use reticle_geometry::{Dbu, LayerId, Path, Point, Rect, Transform};
use reticle_model::{
    Cell, Document, DrawShape, Instance, LayerInfo, Pin, PinDirection, ShapeKind, Technology,
};

use crate::def::{DefData, DefSeg};
use crate::design::{DesignPin, LefDefDesign, Net, NetSegment, Row, Site};
use crate::error::{LefDefWarning, WarningKind};
use crate::lef::{LefData, LefLayerKind, LefRect};

/// The resolution assumed when neither the DEF nor the LEF declares one: 1000
/// DBU/micron (a 1 nm grid), the common `OpenROAD` default.
pub(crate) const DEFAULT_DBU_PER_MICRON: i64 = 1000;

/// Fallback routing-wire width in DBU when a layer declares no `WIDTH` and the DEF
/// gives none: 100 DBU (0.1 micron on the default grid). Only used so a wire still
/// has a visible stroke; never a substitute for a real width.
const FALLBACK_WIRE_WIDTH: Dbu = 100;

/// A small, stable display palette for LEF layers, cycled by declaration index so
/// the imported technology renders in distinct colors without a foundry color map.
const LAYER_PALETTE: [u32; 8] = [
    0x4488_FFFF, // blue
    0x44CC_88FF, // green
    0xCC88_44FF, // orange
    0xCC44_88FF, // magenta
    0x8888_88FF, // gray
    0xCCCC_44FF, // yellow
    0x44CC_CCFF, // cyan
    0xAA44_CCFF, // violet
];

/// Interns LEF/DEF layer names into [`LayerId`]s and accumulates the display table.
struct LayerTable {
    ids: HashMap<String, LayerId>,
    widths: HashMap<LayerId, Dbu>,
    infos: Vec<LayerInfo>,
    next: u16,
}

impl LayerTable {
    fn new() -> Self {
        Self {
            ids: HashMap::new(),
            widths: HashMap::new(),
            infos: Vec::new(),
            // Start layer numbers at 1; 0 is left free as an unset sentinel.
            next: 1,
        }
    }

    /// Returns the [`LayerId`] for `name`, assigning a fresh one (and a palette
    /// color) the first time the name is seen.
    fn intern(&mut self, name: &str) -> LayerId {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = LayerId::new(self.next, 0);
        self.next = self.next.saturating_add(1);
        let color = LAYER_PALETTE[self.infos.len() % LAYER_PALETTE.len()];
        self.infos.push(LayerInfo {
            id,
            name: name.to_string(),
            color_rgba: color,
            visible: true,
        });
        self.ids.insert(name.to_string(), id);
        id
    }

    /// The default routing width for a layer in DBU, if one was recorded.
    fn width_of(&self, id: LayerId) -> Option<Dbu> {
        self.widths.get(&id).copied()
    }
}

/// Clamps an `i64` DBU value into the model's [`Dbu`] range.
fn clamp_dbu(v: i64) -> Dbu {
    v.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// Converts a micron dimension to DBU at the given resolution, clamped to range.
fn um_to_dbu(um: f64, dbu_per_micron: i64) -> Dbu {
    let scaled = (um * dbu_per_micron as f64).round();
    // Guard against non-finite products before the cast.
    if !scaled.is_finite() {
        return 0;
    }
    clamp_dbu(scaled as i64)
}

/// Maps a LEF/DEF direction string to a model [`PinDirection`].
fn direction_of(s: &str) -> PinDirection {
    match s.to_ascii_uppercase().as_str() {
        "INPUT" => PinDirection::Input,
        "OUTPUT" => PinDirection::Output,
        _ => PinDirection::Inout,
    }
}

/// Lowers a LEF/DEF pair into a [`LefDefDesign`].
pub(crate) fn lower(lef: LefData, def: DefData) -> LefDefDesign {
    let dbu_per_micron = def
        .dbu_per_micron
        .or(lef.dbu_per_micron)
        .unwrap_or(DEFAULT_DBU_PER_MICRON)
        .max(1);

    let mut warnings = lef.warnings;
    warnings.extend(def.warnings);

    let mut layers = build_layer_table(&lef.layers, dbu_per_micron);

    let mut document = Document::new();
    for m in &lef.macros {
        document.insert_cell(lower_macro(m, &mut layers, dbu_per_micron));
    }

    let top_name = if def.design_name.is_empty() {
        "top".to_string()
    } else {
        def.design_name.clone()
    };
    let mut top = Cell::new(top_name.clone());

    top.instances = lower_components(&def.components, &document, &mut warnings);
    let nets = lower_nets(&def.nets, &mut layers, &mut top);
    let pins = lower_pins(&def.pins, &mut layers, &mut top);

    let technology = build_technology(&top_name, dbu_per_micron, &mut layers);
    document.set_technology(technology);
    document.insert_cell(top);
    document.set_top_cells(vec![top_name.clone()]);

    LefDefDesign {
        document,
        design_name: top_name,
        die_area: lower_die_area(def.die_area),
        sites: lower_sites(&lef.sites, dbu_per_micron),
        rows: lower_rows(&def.rows),
        nets,
        pins,
        overlays: crate::design::ReportOverlays::default(),
        warnings,
    }
}

/// Builds the interned layer table from the LEF layers, recording routing widths.
fn build_layer_table(lef_layers: &[crate::lef::LefLayer], dbu_per_micron: i64) -> LayerTable {
    let mut layers = LayerTable::new();
    // Declaration order keeps colors and numbering stable across runs.
    for layer in lef_layers {
        let id = layers.intern(&layer.name);
        if matches!(layer.kind, LefLayerKind::Routing)
            && let Some(w) = layer.width_um
        {
            layers.widths.insert(id, um_to_dbu(w, dbu_per_micron));
        }
    }
    layers
}

/// Lowers placed DEF components to model instances, warning on a component that
/// names a macro the LEF never defined.
fn lower_components(
    components: &[crate::def::DefComponent],
    document: &Document,
    warnings: &mut Vec<LefDefWarning>,
) -> Vec<Instance> {
    let mut instances = Vec::new();
    for c in components {
        let Some((x, y, orient)) = &c.placed else {
            continue; // unplaced: nothing to draw
        };
        if document.cell(&c.macro_name).is_none() {
            warnings.push(LefDefWarning::new(
                WarningKind::UnresolvedReference,
                "component references an undefined macro",
                format!(
                    "instance `{}` places macro `{}`, which the LEF did not define; the placement was skipped",
                    c.inst, c.macro_name
                ),
            ));
            continue;
        }
        instances.push(Instance {
            cell: c.macro_name.clone(),
            transform: Transform {
                translation: Point::new(clamp_dbu(*x), clamp_dbu(*y)),
                orientation: crate::orient::from_def(orient),
                magnification: reticle_geometry::Magnification::UNITY,
            },
        });
    }
    instances
}

/// Lowers DEF nets to the run-level net list, drawing each wire into `top` so it
/// renders.
fn lower_nets(nets: &[crate::def::DefNet], layers: &mut LayerTable, top: &mut Cell) -> Vec<Net> {
    let mut out = Vec::new();
    for n in nets {
        let mut segments = Vec::new();
        for seg in &n.segments {
            match seg {
                DefSeg::Wire { layer, points } => {
                    let id = layers.intern(layer);
                    let width = layers.width_of(id).unwrap_or(FALLBACK_WIRE_WIDTH);
                    let pts: Vec<Point> = points
                        .iter()
                        .map(|(x, y)| Point::new(clamp_dbu(*x), clamp_dbu(*y)))
                        .collect();
                    if pts.len() >= 2 {
                        top.shapes.push(DrawShape::new(
                            id,
                            ShapeKind::Path(Path::new(
                                pts.clone(),
                                width,
                                reticle_geometry::Endcap::Flat,
                            )),
                        ));
                    }
                    segments.push(NetSegment::Wire {
                        layer: id,
                        points: pts,
                        width,
                    });
                }
                DefSeg::Via { at, via } => segments.push(NetSegment::Via {
                    at: Point::new(clamp_dbu(at.0), clamp_dbu(at.1)),
                    via: via.clone(),
                }),
            }
        }
        out.push(Net {
            name: n.name.clone(),
            use_kind: n.use_kind.clone(),
            segments,
        });
    }
    out
}

/// Lowers DEF pins to design pins, drawing each placed pin rectangle into `top` and
/// recording it as a model [`Pin`].
fn lower_pins(
    def_pins: &[crate::def::DefPin],
    layers: &mut LayerTable,
    top: &mut Cell,
) -> Vec<DesignPin> {
    let mut out = Vec::new();
    for p in def_pins {
        let origin = p.placed.as_ref().map_or((0, 0), |(x, y, _)| (*x, *y));
        let layer_id = p.layer.as_ref().map(|name| layers.intern(name));
        let region = p.rect.map(|(x1, y1, x2, y2)| {
            Rect::new(
                Point::new(clamp_dbu(origin.0 + x1), clamp_dbu(origin.1 + y1)),
                Point::new(clamp_dbu(origin.0 + x2), clamp_dbu(origin.1 + y2)),
            )
        });
        if let (Some(id), Some(rect)) = (layer_id, region)
            && !rect.is_empty()
        {
            top.shapes.push(DrawShape::new(id, ShapeKind::Rect(rect)));
            top.pins.push(Pin {
                name: p.name.clone(),
                region: rect,
                layer: id,
                direction: direction_of(&p.direction),
            });
        }
        out.push(DesignPin {
            name: p.name.clone(),
            direction: direction_of(&p.direction),
            net: p.net.clone(),
            layer: layer_id,
            region,
        });
    }
    out
}

/// Builds the [`Technology`] from the interned layer table, moving the layer infos
/// out of `layers`.
fn build_technology(top_name: &str, dbu_per_micron: i64, layers: &mut LayerTable) -> Technology {
    let name = if top_name == "top" {
        "lefdef".to_string()
    } else {
        format!("{top_name}_tech")
    };
    Technology {
        name,
        dbu_per_micron,
        layers: std::mem::take(&mut layers.infos),
        ..Technology::default()
    }
}

/// Lowers LEF sites to run-level [`Site`] records in DBU.
fn lower_sites(sites: &[crate::lef::LefSite], dbu_per_micron: i64) -> Vec<Site> {
    sites
        .iter()
        .map(|s| Site {
            name: s.name.clone(),
            class: s.class.clone(),
            width: um_to_dbu(s.width_um, dbu_per_micron),
            height: um_to_dbu(s.height_um, dbu_per_micron),
        })
        .collect()
}

/// Lowers DEF rows to run-level [`Row`] records with mapped orientations.
fn lower_rows(rows: &[crate::def::DefRow]) -> Vec<Row> {
    rows.iter()
        .map(|r| Row {
            name: r.name.clone(),
            site: r.site.clone(),
            origin: Point::new(clamp_dbu(r.orig_x), clamp_dbu(r.orig_y)),
            orientation: crate::orient::from_def(&r.orient),
            count_x: r.num_x,
            count_y: r.num_y,
            step_x: clamp_dbu(r.step_x),
            step_y: clamp_dbu(r.step_y),
        })
        .collect()
}

/// Lowers a DEF die-area rectangle to a model [`Rect`] in DBU.
fn lower_die_area(die: Option<(i64, i64, i64, i64)>) -> Option<Rect> {
    die.map(|(x1, y1, x2, y2)| {
        Rect::new(
            Point::new(clamp_dbu(x1), clamp_dbu(y1)),
            Point::new(clamp_dbu(x2), clamp_dbu(y2)),
        )
    })
}

/// Lowers one LEF macro to a [`Cell`]: pin ports and obstructions become drawn
/// rectangles on their mapped layers, and each pin also becomes a model [`Pin`].
fn lower_macro(m: &crate::lef::LefMacro, layers: &mut LayerTable, dbu_per_micron: i64) -> Cell {
    let mut cell = Cell::new(m.name.clone());

    for pin in &m.pins {
        let direction = direction_of(&pin.direction);
        let mut union: Option<(LayerId, Rect)> = None;
        for r in &pin.rects {
            let (id, rect) = lower_rect(r, layers, dbu_per_micron);
            if rect.is_empty() {
                continue;
            }
            cell.shapes.push(DrawShape::new(id, ShapeKind::Rect(rect)));
            union = Some(match union {
                Some((uid, urect)) => (uid, urect.union(&rect)),
                None => (id, rect),
            });
        }
        if let Some((id, region)) = union {
            cell.pins.push(Pin {
                name: pin.name.clone(),
                region,
                layer: id,
                direction,
            });
        }
    }

    for r in &m.obs {
        let (id, rect) = lower_rect(r, layers, dbu_per_micron);
        if !rect.is_empty() {
            cell.shapes.push(DrawShape::new(id, ShapeKind::Rect(rect)));
        }
    }

    cell
}

/// Lowers a LEF micron rectangle to a mapped `(LayerId, Rect)` in DBU.
fn lower_rect(r: &LefRect, layers: &mut LayerTable, dbu_per_micron: i64) -> (LayerId, Rect) {
    let id = layers.intern(&r.layer);
    let (x1, y1, x2, y2) = r.coords;
    let rect = Rect::new(
        Point::new(um_to_dbu(x1, dbu_per_micron), um_to_dbu(y1, dbu_per_micron)),
        Point::new(um_to_dbu(x2, dbu_per_micron), um_to_dbu(y2, dbu_per_micron)),
    );
    (id, rect)
}

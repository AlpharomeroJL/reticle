//! Device recognition: SKY130 MOSFETs from layer geometry.
//!
//! This module sits a level above [connectivity extraction](crate::Extractor). A
//! MOSFET is recognised where a poly shape crosses a diffusion shape: that
//! overlap is the transistor's channel (its *gate*). The diffusion on either side
//! of the channel is the source and the drain; whether it is an NMOS or a PMOS is
//! read from the surrounding implant (n+ `nsdm` / p+ `psdm`) and well (`nwell`).
//!
//! # Why device recognition needs more than connectivity
//!
//! Pure geometric connectivity ([`Extractor`]) sees a single
//! diffusion rectangle as *one* net: source and drain are the same piece of
//! silicon, shorted together. That is correct for a wire and wrong for a
//! transistor, whose channel is not a conductor at DC. Device recognition
//! therefore **splits the diffusion by the gate** before assigning terminal nets,
//! so a transistor's source and drain land on different nets exactly when the
//! layout wires them apart.
//!
//! # Scope (LVS-lite, honest about its limits)
//!
//! Recognised: NMOS/PMOS from poly-over-diff, terminal nets bound through the
//! connectivity extractor, and gate width/length from the channel geometry.
//! *Not* done: parameter matching beyond W/L, parasitic devices (diodes,
//! capacitors), series/parallel device folding, or bipolar/JFET/diode
//! recognition. See the device-extraction chapter and ADR for the full boundary.

use std::collections::HashMap;

use reticle_geometry::{BooleanOp, LayerId, Polygon, Rect, Shape as _, polygon_boolean};
use reticle_model::{Document, DrawShape, ShapeKind};

use crate::{ConnectionRules, Extractor, Netlist, sky130_connection_rules};

/// The kind of a recognised MOSFET.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// N-channel MOSFET: n+ diffusion (`nsdm`) in the p-substrate.
    Nmos,
    /// P-channel MOSFET: p+ diffusion (`psdm`) inside an `nwell`.
    Pmos,
}

impl DeviceKind {
    /// The SPICE-style short name (`"nmos"` / `"pmos"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Nmos => "nmos",
            DeviceKind::Pmos => "pmos",
        }
    }
}

/// The technology's device-recognition layers: which GDS `(layer, datatype)`
/// pairs are diffusion, poly, the implants, and the well.
///
/// [`sky130`](Self::sky130) fills these with the SKY130 numbers; a different
/// technology can supply its own without touching the recognition logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceTech {
    /// Diffusion (active) layer.
    pub diff: LayerId,
    /// Substrate/well tap layer (bulk contact diffusion).
    pub tap: LayerId,
    /// Polysilicon gate layer.
    pub poly: LayerId,
    /// N-well layer (present under PMOS).
    pub nwell: LayerId,
    /// N+ select/implant layer (marks NMOS source/drain and n-taps).
    pub nsdm: LayerId,
    /// P+ select/implant layer (marks PMOS source/drain and p-taps).
    pub psdm: LayerId,
    /// Local-interconnect contact layer (poly/diff/tap → `li1`).
    pub licon1: LayerId,
    /// Local-interconnect (metal-0) layer, used to tie bodies to their rails.
    pub li1: LayerId,
}

impl DeviceTech {
    /// The SKY130 device layers.
    #[must_use]
    pub fn sky130() -> Self {
        Self {
            diff: LayerId::new(65, 20),
            tap: LayerId::new(65, 44),
            poly: LayerId::new(66, 20),
            nwell: LayerId::new(64, 20),
            nsdm: LayerId::new(93, 44),
            psdm: LayerId::new(94, 20),
            licon1: LayerId::new(66, 44),
            li1: LayerId::new(67, 20),
        }
    }
}

/// A recognised MOSFET with its terminals bound to extracted nets.
///
/// Terminal fields are indices into [`DeviceNetlist::nets`]; a terminal that
/// could not be bound to a net is `None` (honest about an unresolved bulk or an
/// isolated diffusion lobe rather than inventing a net).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// NMOS or PMOS.
    pub kind: DeviceKind,
    /// Net index of the gate (the poly conductor).
    pub gate_net: Option<usize>,
    /// Net index of the source diffusion lobe.
    pub source_net: Option<usize>,
    /// Net index of the drain diffusion lobe.
    pub drain_net: Option<usize>,
    /// Net index of the bulk (well/tap), if determinable.
    pub bulk_net: Option<usize>,
    /// Channel width in database units (the diffusion extent under the gate).
    pub width: i64,
    /// Channel length in database units (the poly extent across the channel).
    pub length: i64,
}

/// The result of device extraction: the recognised devices plus the device-aware
/// netlist their terminals index into.
///
/// [`nets`](Self::nets) is the connectivity netlist computed *after* splitting
/// each diffusion by its gates, so a transistor's source and drain are distinct
/// nets whenever the layout wires them apart. It is a sibling of the frozen
/// connectivity [`Netlist`], not a replacement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceNetlist {
    /// The recognised devices, in a stable order.
    pub devices: Vec<Device>,
    /// The device-aware net partition the terminals index into.
    pub nets: crate::Netlist,
}

impl DeviceNetlist {
    /// The number of devices of `kind`.
    #[must_use]
    pub fn count_of(&self, kind: DeviceKind) -> usize {
        self.devices.iter().filter(|d| d.kind == kind).count()
    }

    /// The name of the net a terminal is bound to, or `None` for an unbound
    /// terminal or an out-of-range index.
    #[must_use]
    pub fn net_name(&self, terminal: Option<usize>) -> Option<&str> {
        terminal
            .and_then(|i| self.nets.nets.get(i))
            .map(|n| n.name.as_str())
    }
}

/// A device reduced to its kind and terminal net *names*, the unit the
/// device-level compare matches on.
///
/// Source and drain are held in canonical (sorted) order because they are
/// geometrically symmetric, so a device matches regardless of which lobe extraction
/// labelled the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    /// NMOS or PMOS.
    pub kind: DeviceKind,
    /// Gate net name (`None` if the net is unnamed or unbound).
    pub gate: Option<String>,
    /// Source net name.
    pub source: Option<String>,
    /// Drain net name.
    pub drain: Option<String>,
    /// Bulk net name.
    pub bulk: Option<String>,
}

impl DeviceSummary {
    /// The canonical match key: kind, gate, the *unordered* {source, drain}, and
    /// bulk. Two devices are the same device iff their keys are equal.
    fn key(
        &self,
    ) -> (
        DeviceKind,
        &Option<String>,
        [&Option<String>; 2],
        &Option<String>,
    ) {
        let mut sd = [&self.source, &self.drain];
        sd.sort();
        (self.kind, &self.gate, sd, &self.bulk)
    }
}

/// The result of a device-level compare: the devices each side has that the other
/// does not, matched by kind and terminal-net connectivity.
///
/// Empty means the two device netlists agree on every device (count, kind, and
/// terminal nets). This is the device-level analogue of [`NetlistDiff`](crate::NetlistDiff)
/// and is additive: it does not change the connectivity
/// [`compare_netlists`](crate::compare_netlists).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDiff {
    /// Devices in `expected` with no match in `extracted` (the layout is missing
    /// them, or wired them differently).
    pub missing: Vec<DeviceSummary>,
    /// Devices in `extracted` with no match in `expected` (spurious or miswired in
    /// the layout).
    pub extra: Vec<DeviceSummary>,
}

impl DeviceDiff {
    /// Returns `true` if the two device netlists agree on every device.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }

    /// The total number of unmatched devices across both sides.
    #[must_use]
    pub fn len(&self) -> usize {
        self.missing.len() + self.extra.len()
    }
}

/// Summarises `device` (from `dnl`) into a kind plus terminal net names.
fn summarise(dnl: &DeviceNetlist, device: &Device) -> DeviceSummary {
    let name = |t: Option<usize>| dnl.net_name(t).map(str::to_string);
    DeviceSummary {
        kind: device.kind,
        gate: name(device.gate_net),
        source: name(device.source_net),
        drain: name(device.drain_net),
        bulk: name(device.bulk_net),
    }
}

/// Compares an `extracted` device netlist against an `expected` one, device-level
/// LVS-lite.
///
/// Each device is reduced to its kind and terminal net *names* ([`DeviceSummary`]);
/// devices are then matched greedily by that signature (source and drain unordered,
/// since they are symmetric). Unmatched expected devices are
/// [`missing`](DeviceDiff::missing); unmatched extracted devices are
/// [`extra`](DeviceDiff::extra). This catches device-count and terminal-net
/// mismatches.
///
/// The match is name-based, so both sides must name their nets (the layout through
/// label geometry, the schematic directly). It does *not* match device parameters
/// (W/L or model) or recognise parasitic devices; see the device-extraction
/// chapter for the full scope.
#[must_use]
pub fn compare_devices(extracted: &DeviceNetlist, expected: &DeviceNetlist) -> DeviceDiff {
    let ext: Vec<DeviceSummary> = extracted
        .devices
        .iter()
        .map(|d| summarise(extracted, d))
        .collect();
    let exp: Vec<DeviceSummary> = expected
        .devices
        .iter()
        .map(|d| summarise(expected, d))
        .collect();

    // Greedily cancel each expected device against an unused extracted match.
    let mut used = vec![false; ext.len()];
    let mut missing = Vec::new();
    for want in &exp {
        let hit = ext
            .iter()
            .enumerate()
            .find(|(i, got)| !used[*i] && got.key() == want.key());
        match hit {
            Some((i, _)) => used[i] = true,
            None => missing.push(want.clone()),
        }
    }
    let extra = ext
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(_, s)| s)
        .collect();

    DeviceDiff { missing, extra }
}

/// A gate candidate: a poly shape crossing a diffusion shape.
struct Gate {
    /// Index (into the flattened shape list) of the crossing poly.
    poly: usize,
    /// Index (into the flattened shape list) of the crossed diffusion.
    diff: usize,
    /// The channel region (poly ∩ diff).
    region: Rect,
    /// NMOS or PMOS, from the surrounding implant/well.
    kind: DeviceKind,
    /// Channel width (diffusion extent under the gate).
    width: i64,
    /// Channel length (poly extent across the channel).
    length: i64,
}

/// Extracts the MOSFET-level netlist of `cell` in `doc` under `tech`.
///
/// Flattens the cell to leaf shapes, recognises every poly-over-diffusion gate,
/// classifies each as NMOS or PMOS from the implant/well, and binds the gate,
/// source, drain, and bulk terminals to nets from a device-aware connectivity
/// pass (diffusion split by its gates). An unknown cell yields an empty netlist.
#[must_use]
pub fn extract_devices(doc: &Document, cell: &str, tech: &DeviceTech) -> DeviceNetlist {
    extract_devices_labeled(doc, cell, tech, &[])
}

/// Like [`extract_devices`], but seeds the device-aware netlist with net-naming
/// `labels` (see [`NetLabel`](crate::NetLabel)), so terminal nets carry the pin
/// names a real flow attaches through label geometry. The three-argument
/// [`extract_devices`] is this with no labels.
#[must_use]
pub fn extract_devices_labeled(
    doc: &Document,
    cell: &str,
    tech: &DeviceTech,
    labels: &[crate::NetLabel],
) -> DeviceNetlist {
    let shapes = doc.flatten(cell);
    let gates = find_gates(&shapes, tech);

    // Split each gated diffusion into its source/drain lobes, then extract
    // connectivity over the split geometry so a channel separates source from
    // drain. `orig_to_new` maps a 1:1-carried shape to its index in the split
    // list; `diff_lobes` records, per original diffusion, the split lobe indices.
    let split = split_diffusions(&shapes, &gates);
    let nets = Extractor::new()
        .with_rules(device_connection_rules(tech))
        .with_labels(labels.to_vec())
        .extract_shapes(&split.shapes);

    let devices = gates
        .iter()
        .map(|g| bind_terminals(&shapes, tech, g, &split, &nets))
        .collect();

    DeviceNetlist { devices, nets }
}

/// The connectivity rules used to bind device terminals to nets.
///
/// This is the SKY130 via/contact stack ([`sky130_connection_rules`]) extended
/// with the body-tie path `tap → licon1 → li1`, so a tapped well or substrate
/// joins its power rail in the device netlist (the base stack ties only the
/// signal path, poly/diff → li1 → metal).
fn device_connection_rules(tech: &DeviceTech) -> ConnectionRules {
    sky130_connection_rules().connect(tech.tap, tech.licon1, tech.li1)
}

/// The device-aware geometry: the split shape list plus the index bookkeeping
/// that ties a gate back to its poly, source/drain lobes, and bulk shapes.
struct SplitGeometry {
    /// The shapes to extract connectivity over: every non-gated shape as-is, and
    /// each gated diffusion replaced by its lobe polygons.
    shapes: Vec<DrawShape>,
    /// For a shape carried through 1:1, its index in [`shapes`](Self::shapes).
    orig_to_new: Vec<Option<usize>>,
    /// For an original diffusion index, the `(new index, bounding box)` of each
    /// lobe it was split into.
    diff_lobes: HashMap<usize, Vec<(usize, Rect)>>,
}

/// Builds the [`SplitGeometry`]: subtracts each diffusion's gate regions from it
/// so the channel gap separates the source and drain lobes.
fn split_diffusions(shapes: &[DrawShape], gates: &[Gate]) -> SplitGeometry {
    // Gate channel regions grouped by the diffusion they cross.
    let mut gates_by_diff: HashMap<usize, Vec<Rect>> = HashMap::new();
    for g in gates {
        gates_by_diff.entry(g.diff).or_default().push(g.region);
    }

    let mut out = Vec::with_capacity(shapes.len());
    let mut orig_to_new = vec![None; shapes.len()];
    let mut diff_lobes: HashMap<usize, Vec<(usize, Rect)>> = HashMap::new();

    for (i, s) in shapes.iter().enumerate() {
        let Some(regions) = gates_by_diff.get(&i) else {
            orig_to_new[i] = Some(out.len());
            out.push(s.clone());
            continue;
        };
        // diffusion − gates → the source/drain lobes.
        let cut: Vec<Polygon> = regions.iter().map(|r| Polygon::from_rect(*r)).collect();
        let lobes =
            polygon_boolean(BooleanOp::Difference, &[footprint(s)], &cut).unwrap_or_default();
        let mut entry = Vec::new();
        for lobe in lobes {
            if lobe.len() < 3 {
                continue; // degenerate sliver, not a real diffusion region
            }
            let bbox = lobe.bounding_box();
            entry.push((out.len(), bbox));
            out.push(DrawShape::new(s.layer, ShapeKind::Polygon(lobe)));
        }
        diff_lobes.insert(i, entry);
    }

    SplitGeometry {
        shapes: out,
        orig_to_new,
        diff_lobes,
    }
}

/// Binds a gate's gate/source/drain/bulk terminals to nets from the device-aware
/// netlist.
fn bind_terminals(
    shapes: &[DrawShape],
    tech: &DeviceTech,
    gate: &Gate,
    split: &SplitGeometry,
    nets: &Netlist,
) -> Device {
    let gate_net = split.orig_to_new[gate.poly].and_then(|ni| net_index_of(nets, ni));

    // The two diffusion lobes flanking the channel along its length axis.
    let empty = Vec::new();
    let lobes = split.diff_lobes.get(&gate.diff).unwrap_or(&empty);
    let poly_vertical =
        shapes[gate.poly].bounding_box().height() >= shapes[gate.poly].bounding_box().width();
    let (source_lobe, drain_lobe) = flanking_lobes(lobes, &gate.region, poly_vertical);
    let source_net = source_lobe.and_then(|ni| net_index_of(nets, ni));
    let drain_net = drain_lobe.and_then(|ni| net_index_of(nets, ni));

    let bulk_net = bulk_net(shapes, tech, gate, split, nets);

    Device {
        kind: gate.kind,
        gate_net,
        source_net,
        drain_net,
        bulk_net,
        width: gate.width,
        length: gate.length,
    }
}

/// The lobe on the low side and the lobe on the high side of `region` along the
/// channel's length axis (x when the poly runs vertically, else y).
///
/// The source is the nearest lobe abutting the channel on the low side; the drain
/// the nearest on the high side. Source/drain are geometrically symmetric, so this
/// low/high convention is a stable labelling, not a claim about circuit function.
fn flanking_lobes(
    lobes: &[(usize, Rect)],
    region: &Rect,
    poly_vertical: bool,
) -> (Option<usize>, Option<usize>) {
    // Coordinate accessors for the length axis.
    let lo = |r: &Rect| if poly_vertical { r.min.x } else { r.min.y };
    let hi = |r: &Rect| if poly_vertical { r.max.x } else { r.max.y };
    let gate_lo = lo(region);
    let gate_hi = hi(region);

    let mut source: Option<(usize, i32)> = None; // (index, closeness key = hi(lobe))
    let mut drain: Option<(usize, i32)> = None; // (index, closeness key = lo(lobe))
    for &(idx, bbox) in lobes {
        if hi(&bbox) <= gate_lo {
            // Low side: keep the one whose high edge is nearest the channel.
            if source.is_none_or(|(_, k)| hi(&bbox) > k) {
                source = Some((idx, hi(&bbox)));
            }
        } else if lo(&bbox) >= gate_hi {
            // High side: keep the one whose low edge is nearest the channel.
            if drain.is_none_or(|(_, k)| lo(&bbox) < k) {
                drain = Some((idx, lo(&bbox)));
            }
        }
    }
    (source.map(|(i, _)| i), drain.map(|(i, _)| i))
}

/// The bulk (body) net: the tap tie of the matching polarity near the gate.
///
/// NMOS body is the p-substrate, tied through a p-tap (a `tap` shape under
/// `psdm`); PMOS body is the n-well, tied through an n-tap (a `tap` shape under
/// `nsdm`). This is best-effort: an untapped body yields `None` rather than a
/// guessed net.
fn bulk_net(
    shapes: &[DrawShape],
    tech: &DeviceTech,
    gate: &Gate,
    split: &SplitGeometry,
    nets: &Netlist,
) -> Option<usize> {
    let tie_implant = match gate.kind {
        DeviceKind::Nmos => tech.psdm, // p-tap ties the substrate
        DeviceKind::Pmos => tech.nsdm, // n-tap ties the n-well
    };
    // The nearest tap shape that carries the body-tie implant.
    let mut best: Option<(usize, i64)> = None;
    for (i, s) in shapes.iter().enumerate() {
        if s.layer != tech.tap {
            continue;
        }
        let tap_box = s.bounding_box();
        if !any_layer_overlaps(shapes, tie_implant, &tap_box) {
            continue;
        }
        let d = center_distance_sq(&tap_box, &gate.region);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.and_then(|(i, _)| split.orig_to_new[i])
        .and_then(|ni| net_index_of(nets, ni))
}

/// Squared distance between the centres of two rectangles.
fn center_distance_sq(a: &Rect, b: &Rect) -> i64 {
    let cx = |r: &Rect| i64::midpoint(i64::from(r.min.x), i64::from(r.max.x));
    let cy = |r: &Rect| i64::midpoint(i64::from(r.min.y), i64::from(r.max.y));
    let dx = cx(a) - cx(b);
    let dy = cy(a) - cy(b);
    dx * dx + dy * dy
}

/// The index of the net in `nets` that owns shape `idx`, if any.
fn net_index_of(nets: &Netlist, idx: usize) -> Option<usize> {
    nets.nets.iter().position(|n| n.contains(idx))
}

/// The polygonal footprint of a shape (rect/polygon exact, path via bounding box).
fn footprint(shape: &DrawShape) -> Polygon {
    match &shape.kind {
        ShapeKind::Rect(r) => Polygon::from_rect(*r),
        ShapeKind::Polygon(p) => p.clone(),
        ShapeKind::Path(p) => Polygon::from_rect(p.bounding_box()),
    }
}

/// Finds every poly-over-diffusion gate in `shapes`.
///
/// A gate is the positive-area overlap of a poly shape and a diffusion shape. The
/// overlap region gives the channel; its extent along the poly's crossing axis is
/// the length, and along the diffusion the width.
fn find_gates(shapes: &[DrawShape], tech: &DeviceTech) -> Vec<Gate> {
    let polys: Vec<usize> = layer_indices(shapes, tech.poly);
    let diffs: Vec<usize> = layer_indices(shapes, tech.diff);
    let mut gates = Vec::new();

    for &pi in &polys {
        let poly_box = shapes[pi].bounding_box();
        for &di in &diffs {
            let diff_box = shapes[di].bounding_box();
            let Some(region) = poly_box.intersection(&diff_box) else {
                continue; // no positive-area overlap → not a gate
            };
            if !crosses(&poly_box, &region, &diff_box) {
                continue; // poly grazes the diffusion but does not cross it
            }
            let (length, width) = channel_dimensions(&poly_box, &region);
            let kind = classify(shapes, tech, &region, diff_box);
            gates.push(Gate {
                poly: pi,
                diff: di,
                region,
                kind,
                width,
                length,
            });
        }
    }
    gates
}

/// Returns `true` if the poly fully crosses the diffusion, i.e. the channel
/// region spans the diffusion's whole extent along the width axis.
///
/// A poly that only overlaps the corner or end of a diffusion (its region is
/// shorter than the diffusion in the width direction) leaves diffusion on just one
/// side and is not a transistor channel; only a full crossing separates a distinct
/// source from a distinct drain.
fn crosses(poly_box: &Rect, region: &Rect, diff_box: &Rect) -> bool {
    let poly_vertical = poly_box.height() >= poly_box.width();
    if poly_vertical {
        // Width axis is y: the channel must span the diffusion's full height.
        region.height() >= diff_box.height()
    } else {
        // Width axis is x: the channel must span the diffusion's full width.
        region.width() >= diff_box.width()
    }
}

/// Channel length and width from the gate `region` and the crossing poly's box.
///
/// Current flows across the poly, so the length is the gate extent along the
/// poly's *minor* axis (the direction the poly is narrow in) and the width is the
/// extent along the poly's major axis. A square poly falls back to width = x.
fn channel_dimensions(poly_box: &Rect, region: &Rect) -> (i64, i64) {
    let poly_vertical = poly_box.height() >= poly_box.width();
    if poly_vertical {
        // Vertical poly stripe: current flows in x, so length is x, width is y.
        (region.width(), region.height())
    } else {
        // Horizontal poly stripe: current flows in y, so length is y, width is x.
        (region.height(), region.width())
    }
}

/// Classifies a gate as NMOS or PMOS from the implant/well around it.
///
/// PMOS when a p+ select (`psdm`) covers the diffusion, or the channel sits in an
/// `nwell`; otherwise NMOS (n+ select or bare substrate). Implant is preferred
/// over well because it is what the foundry uses to distinguish the source/drain
/// doping.
fn classify(shapes: &[DrawShape], tech: &DeviceTech, region: &Rect, diff_box: Rect) -> DeviceKind {
    let p_select = any_layer_overlaps(shapes, tech.psdm, &diff_box);
    let n_select = any_layer_overlaps(shapes, tech.nsdm, &diff_box);
    let in_nwell = any_layer_overlaps(shapes, tech.nwell, region);

    if p_select && !n_select {
        DeviceKind::Pmos
    } else if n_select && !p_select {
        DeviceKind::Nmos
    } else if in_nwell {
        // Ambiguous or missing implant: the well decides.
        DeviceKind::Pmos
    } else {
        DeviceKind::Nmos
    }
}

/// The indices of every shape on `layer`.
fn layer_indices(shapes: &[DrawShape], layer: LayerId) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer == layer)
        .map(|(i, _)| i)
        .collect()
}

/// Returns `true` if any shape on `layer` has a bounding box overlapping `region`
/// with positive area.
fn any_layer_overlaps(shapes: &[DrawShape], layer: LayerId, region: &Rect) -> bool {
    shapes
        .iter()
        .any(|s| s.layer == layer && s.bounding_box().intersects(region))
}

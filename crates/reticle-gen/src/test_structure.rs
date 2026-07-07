//! The probe-able test-structure generator: the classic measurable silicon test-tile
//! content, selected by a parameter.
//!
//! # What it emits
//!
//! One of four standard structures a probe station measures, chosen by
//! [`StructureKind`]:
//!
//! * **Van der Pauw cross** ([`StructureKind::VanDerPauw`]): a symmetric plus-shaped
//!   region on one conductor layer, built as a horizontal and a vertical bar that
//!   overlap in the centre. Sheet resistance is extracted by forcing current through
//!   one pair of arms and measuring the voltage across the other.
//! * **Contact chain** ([`StructureKind::ContactChain`]): a series of `mcon` contacts
//!   alternating between `li1` and `met1` bridges, so current threads
//!   metal-contact-metal down the chain. Every contact is enclosed by a `met1` bridge
//!   (or a `met1` end pad) by at least the `m1.4` enclosure. Measures per-contact
//!   resistance.
//! * **Comb** ([`StructureKind::Comb`]): two interdigitated combs on one layer whose
//!   fingers interleave at exactly the layer minimum spacing. Measures inter-comb
//!   leakage/shorts.
//! * **Serpentine** ([`StructureKind::Serpentine`]): a single continuous boustrophedon
//!   trace (parallel bars joined end to end by alternating links) on one layer.
//!   Measures line continuity and resistance.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! Every structure is emitted as axis-aligned [`Rect`]s (plus `mcon` cuts for the
//! contact chain), which the bounding-box DRC engine checks exactly. Each is
//! dimensioned so the subset rules for the layers it touches are met:
//!
//! * **Width.** Every drawn rectangle is at least the layer minimum width in its
//!   thinnest dimension (bars, arms, fingers, spines, and links all validate their
//!   width against the layer floor).
//! * **Spacing.** Same-layer features that must not touch are placed at least the
//!   layer minimum spacing apart: the two combs' fingers, the serpentine's adjacent
//!   bars, and the contact chain's same-layer bridges. Features that *do* connect
//!   (a finger to its spine, a serpentine bar to its link, a bridge to its shared
//!   contact) touch or overlap, which the engine does not flag.
//! * **Area.** Every rectangle's bounding-box area clears the layer minimum area where
//!   the subset carries one (`li1`, `met1`); lengths are validated so even the
//!   smallest feature is large enough.
//! * **Enclosure.** In the contact chain each `mcon` cut is enclosed by a `met1`
//!   feature by at least the `m1.4` margin (30 DBU); the `li1` bridges add a generous
//!   enclosure too, though the subset carries no `li1`-to-`mcon` enclosure rule.
//!
//! # Coverage limits (honest)
//!
//! The subset is a min-width/spacing/area/enclosure deck, so these are clean *on that
//! deck*, which is not tape-out clean. The contact chain uses the one cut the subset
//! gives an interconnect enclosure for at both ends (`mcon`, enclosed by `met1`); it
//! is not a poly or diffusion contact chain, because the subset carries no
//! contact-to-active enclosure rules. The van der Pauw cross, comb, and serpentine
//! draw on the interconnect layers only. Cut-to-cut spacing has no rule in the subset,
//! so the contact pitch is a conservative choice rather than a checked constraint.

use reticle_geometry::{Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::gentech::{Conductor, GenTech};
use crate::schema::{FieldSchema, ParamSchema};

/// The conductor layer the single-layer structures (cross, comb, serpentine) are
/// drawn on.
///
/// Restricted to the interconnect layers the SKY130 subset carries width, spacing, and
/// (where present) area rules for. The contact chain ignores this choice: it is fixed
/// to `li1`/`met1` bridges through `mcon`, the layers the subset gives the contact
/// enclosure for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureLayer {
    /// Local interconnect `li1` (67/20): width 170, spacing 170, area 56100.
    #[default]
    Li1,
    /// Metal 1 `met1` (68/20): width 140, spacing 140, area 83000.
    Met1,
    /// Metal 2 `met2` (69/20): width 140, spacing 140, no area rule.
    Met2,
    /// Metal 3 `met3` (70/20): width 300, spacing 300, no area rule.
    Met3,
}

impl StructureLayer {
    /// The conductor data (layer, width, spacing, area) for this choice in the given
    /// technology. The variants name interconnect *levels* (0 = base): `li1..met3` on
    /// SKY130, `Metal1..Metal4` on SG13G2.
    fn conductor(self, gt: &GenTech) -> Conductor {
        match self {
            Self::Li1 => gt.conductor(0),
            Self::Met1 => gt.conductor(1),
            Self::Met2 => gt.conductor(2),
            Self::Met3 => gt.conductor(3),
        }
    }

    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 4] = ["li1", "met1", "met2", "met3"];
}

/// Which probe-able structure to emit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    /// A van der Pauw cross for sheet-resistance measurement.
    #[default]
    VanDerPauw,
    /// A series contact chain of `mcon` contacts between `li1` and `met1`.
    ContactChain,
    /// Two interdigitated combs for leakage/short measurement.
    Comb,
    /// A single continuous serpentine trace for continuity/resistance measurement.
    Serpentine,
}

impl StructureKind {
    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 4] = ["van_der_pauw", "contact_chain", "comb", "serpentine"];
}

/// Parameters for the [`TestStructure`] generator. All lengths are in DBU
/// (1 dbu = 1 nm).
///
/// A `kind` selects the structure; `layer` picks the conductor for the single-layer
/// structures (the contact chain is fixed to `li1`/`met1`). `feature_width` sets the
/// drawn line/arm/finger width, `feature_length` sets how long each such feature is,
/// and `count` sets how many repeats the structure has (arms are fixed at four for the
/// cross, so `count` is the contact count, the finger-pairs, or the serpentine bars).
/// See each field for its range and default.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestStructureParams {
    /// Which structure to emit.
    pub kind: StructureKind,
    /// The conductor layer for the single-layer structures. Ignored by the contact
    /// chain (which is fixed to `li1`/`met1`).
    pub layer: StructureLayer,
    /// Drawn width of each line/arm/finger, in DBU. At least the layer minimum width.
    pub feature_width: i32,
    /// Length of each line/arm/finger, in DBU. Long enough that every drawn rectangle
    /// clears the layer minimum area.
    pub feature_length: i32,
    /// Repeat count: contacts in the chain, finger-pairs in the comb, or bars in the
    /// serpentine. Ignored by the van der Pauw cross (always four arms).
    pub count: u32,
}

impl Default for TestStructureParams {
    fn default() -> Self {
        // A van der Pauw cross on li1 with comfortably-above-rule dimensions: a
        // working example a form can generate unchanged.
        Self {
            kind: StructureKind::VanDerPauw,
            layer: StructureLayer::Li1,
            feature_width: 400,
            feature_length: 2_000,
            count: 8,
        }
    }
}

impl TestStructureParams {
    /// Inclusive lower bound for `feature_width` across every layer: the largest layer
    /// minimum width in the subset (`met3`, 300). Per-layer `validate` enforces the
    /// exact width floor.
    const WIDTH_MIN: i64 = 300;
    /// Inclusive upper bound for `feature_width`: a broad line, bounded so the emitted
    /// geometry stays well inside the coordinate range.
    const WIDTH_MAX: i64 = 50_000;
    /// Inclusive lower bound for `feature_length` across every layer: enough for a
    /// min-width line to clear the largest area rule (`met1`, 83000 / 300 = 277, so
    /// 300 is safe). Per-layer `validate` enforces the exact area-derived floor.
    const LEN_MIN: i64 = 300;
    /// Inclusive upper bound for `feature_length`.
    const LEN_MAX: i64 = 500_000;
    /// Inclusive lower bound on the repeat count.
    const COUNT_MIN: i64 = 2;
    /// Inclusive upper bound on the repeat count: enough for a real chain/comb/snake,
    /// bounded so the emitted geometry and coordinate range stay sane.
    const COUNT_MAX: i64 = 128;

    /// The contact chain's fixed enclosure of each `mcon` by its `li1`/`met1` bridges,
    /// in DBU. At least the `m1.4` requirement (30); chosen larger for margin, and
    /// applied to the `li1` bridges too even though the subset has no `li1`-to-`mcon`
    /// rule.
    const CHAIN_ENC: i32 = 90;

    /// The true per-layer, per-kind floor for `feature_length` given the chosen
    /// `feature_width`: the larger of the area-derived floor and any geometric floor the
    /// structure imposes.
    ///
    /// * **Area floor.** A `feature_width` by `len` rectangle needs
    ///   `len >= ceil(min_area / width)` to clear the layer area rule (where one
    ///   exists); the width is already at least the layer minimum.
    /// * **Serpentine geometric floor.** A serpentine's turn links sit at opposite ends
    ///   of each bar, so a bar must be at least `2 * width + min_spacing` long for the
    ///   left-end and right-end links to keep the layer minimum spacing between them.
    fn length_floor(kind: StructureKind, cond: Conductor, feature_width: i32) -> i64 {
        let area_floor = match cond.min_area {
            Some(min_area) => {
                let w = i64::from(feature_width.max(1));
                div_ceil_i64(min_area, w).max(Self::LEN_MIN)
            }
            None => Self::LEN_MIN,
        };
        let geometric_floor = match kind {
            StructureKind::Serpentine => 2 * i64::from(feature_width) + i64::from(cond.min_spacing),
            _ => 0,
        };
        area_floor.max(geometric_floor)
    }
}

impl GenParams for TestStructureParams {
    fn schema() -> ParamSchema {
        // Placeholders for id/title/description; `Generator::schema` stamps the real
        // values.
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::enumerated(
                    "kind",
                    "Which probe-able structure to emit.",
                    &StructureKind::VARIANTS,
                    "van_der_pauw",
                ),
                FieldSchema::enumerated(
                    "layer",
                    "Conductor layer for single-layer structures (chain is fixed li1/met1).",
                    &StructureLayer::VARIANTS,
                    "li1",
                ),
                FieldSchema::int(
                    "feature_width",
                    "Drawn width of each line/arm/finger; at least the layer min width.",
                    400,
                    Self::WIDTH_MIN,
                    Self::WIDTH_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "feature_length",
                    "Length of each line/arm/finger; long enough to clear the min area.",
                    2_000,
                    Self::LEN_MIN,
                    Self::LEN_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "count",
                    "Repeats: chain contacts, comb finger-pairs, or serpentine bars.",
                    8,
                    Self::COUNT_MIN,
                    Self::COUNT_MAX,
                    "count",
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        // Validation bounds are the reference (SKY130) technology; `generate` uses the
        // active technology's own floors. The contact chain is fixed to the base and
        // next interconnect (conductor 0/1); single-layer structures use `layer`.
        let gt = GenTech::sky130();
        let cond = match self.kind {
            StructureKind::ContactChain => gt.conductor(1),
            _ => self.layer.conductor(&gt),
        };

        check_range(
            "feature_width",
            i64::from(self.feature_width),
            i64::from(cond.min_width),
            Self::WIDTH_MAX,
        )?;

        let length_floor = Self::length_floor(self.kind, cond, self.feature_width);
        check_range(
            "feature_length",
            i64::from(self.feature_length),
            length_floor,
            Self::LEN_MAX,
        )?;

        check_range(
            "count",
            i64::from(self.count),
            Self::COUNT_MIN,
            Self::COUNT_MAX,
        )?;

        Ok(())
    }
}

/// The probe-able test-structure generator.
///
/// Emits one of the four classic measurable structures selected by
/// [`StructureKind`]; see [`TestStructureParams`] for the parameters and the
/// [crate overview](crate) for the DRC-clean-by-construction argument and coverage
/// limits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TestStructure;

impl Generator for TestStructure {
    type Params = TestStructureParams;

    fn id(&self) -> &'static str {
        "test_structure"
    }

    fn title(&self) -> &'static str {
        "Test structure"
    }

    fn description(&self) -> &'static str {
        "A probe-able silicon test structure: van der Pauw cross, contact chain, comb, \
         or serpentine, selected by a parameter. DRC-clean by construction against the \
         SKY130 subset."
    }

    fn generate(
        &self,
        params: &Self::Params,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let start = cell.shapes.len();
        let gt = GenTech::for_technology(tech);
        match params.kind {
            StructureKind::VanDerPauw => emit_van_der_pauw(params, cell, &gt),
            StructureKind::ContactChain => emit_contact_chain(params, cell, &gt),
            StructureKind::Comb => emit_comb(params, cell, &gt),
            StructureKind::Serpentine => emit_serpentine(params, cell, &gt),
        }
        let added = cell.shapes.len() - start;
        let bbox = cell
            .shapes
            .iter()
            .skip(start)
            .map(|s| match &s.kind {
                ShapeKind::Rect(r) => *r,
                // Every structure emits only rects; this arm is unreachable but keeps
                // the match total without an allocation.
                other => bbox_of(other),
            })
            .reduce(|a, b| a.union(&b));
        Ok(GenOutput {
            shapes_added: added,
            bbox,
        })
    }
}

/// Pushes an axis-aligned rectangle on `layer` into the cell.
fn push_rect(cell: &mut Cell, layer: reticle_geometry::LayerId, r: Rect) {
    cell.shapes.push(DrawShape::new(layer, ShapeKind::Rect(r)));
}

/// A symmetric van der Pauw cross: a horizontal and a vertical bar of width
/// `feature_width`, each `feature_length` long, overlapping in the centre. The two
/// bars share the centre square (overlap, not a spacing violation) and each clears the
/// layer width and area rules on its own bounding box.
fn emit_van_der_pauw(params: &TestStructureParams, cell: &mut Cell, gt: &GenTech) {
    let layer = params.layer.conductor(gt).layer;
    let w = params.feature_width;
    let span = params.feature_length.max(w); // the cross fits in a span x span box
    let off = (span - w) / 2; // centre each bar across the span

    let horizontal = Rect::new(Point::new(0, off), Point::new(span, off + w));
    let vertical = Rect::new(Point::new(off, 0), Point::new(off + w, span));
    push_rect(cell, layer, horizontal);
    push_rect(cell, layer, vertical);
}

/// A series contact chain: `count` base cuts in a row, alternating `next` and `base`
/// interconnect bridges so current threads metal-contact-metal along the chain. Each
/// contact is enclosed by **both** bridging conductor levels, so whichever level the
/// process asks to enclose the cut is satisfied (`met1` encloses `mcon` on SKY130,
/// `Metal1` encloses `Via1` on SG13G2). Interior contacts get both frames from the two
/// bridges that meet at them; the two end contacts get the missing frame as an end pad.
fn emit_contact_chain(params: &TestStructureParams, cell: &mut Cell, gt: &GenTech) {
    // The chain threads the base cut between the base and next interconnect levels
    // (mcon between li1/met1 on SKY130; Via1 between Metal1/Metal2 on SG13G2).
    let cut = gt.cut(0);
    let base = gt.conductor(0);
    let next = gt.conductor(1);
    let s = cut.size;
    let enc = TestStructureParams::CHAIN_ENC;
    let n = params.count;

    // Pitch: contact size, an enclosure on each side, and a gap comfortably above the
    // largest same-layer min spacing so consecutive same-layer bridges clear each
    // other, and above any cut-to-cut spacing the deck carries.
    let max_spacing = base
        .min_spacing
        .max(next.min_spacing)
        .max(gt.safe_cut_margin());
    let pitch = s + 2 * enc + 2 * max_spacing;

    // Vertical band: contacts centred in a band tall enough to enclose them by `enc`
    // and to clear the met1/li1 minimum areas comfortably given the wide bridges.
    let band_h = s + 2 * enc;
    let band_y0 = 0;

    // Contact i sits at x = i * pitch (left edge), all in the same band.
    let contact_x = |i: u32| (i as i32) * pitch;

    // Emit the contacts.
    for i in 0..n {
        let x = contact_x(i);
        let contact = Rect::new(
            Point::new(x, band_y0 + enc),
            Point::new(x + s, band_y0 + enc + s),
        );
        push_rect(cell, cut.layer, contact);
    }

    // Emit the alternating bridges between consecutive contacts. Bridge j spans
    // contacts j and j+1; it is met1 when j is even, li1 when odd. A bridge is the
    // rectangle from the left contact's left edge minus `enc` to the right contact's
    // right edge plus `enc`, over the full band.
    for j in 0..n.saturating_sub(1) {
        let layer = if j % 2 == 0 { next.layer } else { base.layer };
        let xl = contact_x(j) - enc;
        let xr = contact_x(j + 1) + s + enc;
        let bridge = Rect::new(Point::new(xl, band_y0), Point::new(xr, band_y0 + band_h));
        push_rect(cell, layer, bridge);
    }

    // Enclose every contact by both bridging conductor levels. Interior contacts
    // already sit under one `next` and one `base` bridge (adjacent bridges alternate
    // parity), so they are framed on both. The two end contacts sit under a single
    // bridge, so add the frame each one lacks:
    //   * contact 0 is always under bridge 0 (`next`), so it lacks a `base` frame;
    //   * contact n-1 is under bridge n-2, which is `next` when its index is even and
    //     `base` otherwise, so it lacks the opposite level.
    let last = n - 1;
    emit_chain_pad(cell, gt, base, contact_x(0), band_y0, s, enc);
    let last_bridge_is_next = n < 2 || (n - 2).is_multiple_of(2);
    let missing = if last_bridge_is_next { base } else { next };
    emit_chain_pad(cell, gt, missing, contact_x(last), band_y0, s, enc);
}

/// Emits a square frame on `cond`'s layer centered on the contact whose left edge is
/// at `x`, large enough to enclose the base cut by the chain enclosure on all sides and
/// to clear `cond`'s minimum area. Used to give an end contact the conductor-level frame
/// its single bridge does not provide.
fn emit_chain_pad(
    cell: &mut Cell,
    gt: &GenTech,
    cond: Conductor,
    x: i32,
    band_y0: i32,
    s: i32,
    enc: i32,
) {
    let side = enclosing_pad_side(gt, cond);
    let cx = x + s / 2;
    let cy = band_y0 + enc + s / 2;
    let pad = Rect::new(
        Point::new(cx - side / 2, cy - side / 2),
        Point::new(cx - side / 2 + side, cy - side / 2 + side),
    );
    push_rect(cell, cond.layer, pad);
}

/// The side of a square pad on `cond` that both encloses the base cut (by at least the
/// chain enclosure) and clears `cond`'s minimum area where the process carries one: the
/// larger of the enclosure-derived side and the area-derived side.
fn enclosing_pad_side(gt: &GenTech, cond: Conductor) -> i32 {
    let enclosure_side = gt.cut(0).size + 2 * TestStructureParams::CHAIN_ENC;
    let area_side = cond.min_area.map_or(0, isqrt_ceil_i32);
    enclosure_side.max(area_side)
}

/// Two interdigitated combs on one layer: comb A (spine at the bottom, fingers up) and
/// comb B (spine at the top, fingers down), with A's and B's fingers interleaved at
/// exactly the layer minimum spacing. A finger connects to its own spine (touch) and
/// stays a min-spacing gap from the other comb's fingers and spine.
fn emit_comb(params: &TestStructureParams, cell: &mut Cell, gt: &GenTech) {
    let cond = params.layer.conductor(gt);
    let layer = cond.layer;
    let w = params.feature_width;
    let finger_len = params.feature_length.max(w);
    let gap = cond.min_spacing;

    let pairs = params.count; // finger-pairs (one A finger + one B finger each)

    // Finger x layout: A finger p at x = p * pitch; B finger p at x = p*pitch + (w+gap).
    // Adjacent A and B fingers are exactly `gap` apart; pitch spaces successive A
    // fingers by 2*(w+gap).
    let pair_pitch = 2 * (w + gap);

    let spine_h = w;
    let overlap = w; // fingers overlap their spine by a full width so the joint is solid

    // Bottom spine (comb A) at y in [0, spine_h]; A fingers rise from it.
    // Top spine (comb B) at y in [spine_h + gap + finger_len, ...]; B fingers descend.
    // The channel between the spines is `finger_len` tall; each finger spans the
    // channel plus its spine overlap, stopping a `gap` short of the far spine.
    let a_spine_y0 = 0;
    let a_spine_y1 = spine_h;
    let channel_y0 = a_spine_y1; // fingers start at the top of A's spine
    let channel_y1 = channel_y0 + finger_len + gap; // top of channel (bottom of B spine region)
    let b_spine_y0 = channel_y1;
    let b_spine_y1 = channel_y1 + spine_h;

    // Overall x extent covers every finger of both combs.
    let last_b_x = (pairs.saturating_sub(1) as i32) * pair_pitch + (w + gap) + w;
    let x_extent = last_b_x.max(w);

    // Spines span the full x extent.
    push_rect(
        cell,
        layer,
        Rect::new(Point::new(0, a_spine_y0), Point::new(x_extent, a_spine_y1)),
    );
    push_rect(
        cell,
        layer,
        Rect::new(Point::new(0, b_spine_y0), Point::new(x_extent, b_spine_y1)),
    );

    for p in 0..pairs {
        let ax = (p as i32) * pair_pitch;
        let bx = ax + (w + gap);
        // A finger: rises from inside A's spine up into the channel, stopping `gap`
        // below B's spine.
        let a_finger = Rect::new(
            Point::new(ax, a_spine_y1 - overlap),
            Point::new(ax + w, b_spine_y0 - gap),
        );
        // B finger: descends from inside B's spine down into the channel, stopping
        // `gap` above A's spine.
        let b_finger = Rect::new(
            Point::new(bx, a_spine_y1 + gap),
            Point::new(bx + w, b_spine_y0 + overlap),
        );
        push_rect(cell, layer, a_finger);
        push_rect(cell, layer, b_finger);
    }
}

/// A single continuous serpentine: `count` horizontal bars stacked on a pitch, joined
/// end to end by alternating vertical links so the whole thing is one connected trace.
/// Adjacent bars are a min-spacing gap apart where they are not joined; a bar and its
/// link touch.
fn emit_serpentine(params: &TestStructureParams, cell: &mut Cell, gt: &GenTech) {
    let cond = params.layer.conductor(gt);
    let layer = cond.layer;
    let w = params.feature_width;
    let length = params.feature_length.max(w);
    let gap = cond.min_spacing;
    let rows = params.count;

    // Vertical pitch: a bar plus a min-spacing gap, so two adjacent bars clear each
    // other exactly at the layer minimum spacing.
    let pitch = w + gap;

    // Bar k occupies y in [k*pitch, k*pitch + w], spanning x in [0, length].
    for k in 0..rows {
        let y0 = (k as i32) * pitch;
        push_rect(
            cell,
            layer,
            Rect::new(Point::new(0, y0), Point::new(length, y0 + w)),
        );
    }

    // Links join bar k to bar k+1: at the right end for even k, the left end for odd
    // k. A link spans both bars' bands (touching each) so the trace is continuous.
    for k in 0..rows.saturating_sub(1) {
        let y0 = (k as i32) * pitch;
        let y1 = ((k + 1) as i32) * pitch + w; // covers bar k and bar k+1
        let (xl, xr) = if k % 2 == 0 {
            (length - w, length) // right end
        } else {
            (0, w) // left end
        };
        push_rect(
            cell,
            layer,
            Rect::new(Point::new(xl, y0), Point::new(xr, y1)),
        );
    }
}

/// The bounding box of a non-rect shape kind (unreachable for this generator, which
/// emits only rects; present so the `bbox` fold is total).
fn bbox_of(kind: &ShapeKind) -> Rect {
    match kind {
        ShapeKind::Rect(r) => *r,
        ShapeKind::Polygon(p) => p.bounding_box(),
        ShapeKind::Path(p) => p.bounding_box(),
    }
}

/// The smallest integer `s` with `s * s >= n` for `n >= 0`, clamped into `i32`.
fn isqrt_ceil_i32(n: i64) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut s = (n as f64).sqrt() as i64;
    while s * s < n {
        s += 1;
    }
    while s > 0 && (s - 1) * (s - 1) >= n {
        s -= 1;
    }
    s.clamp(0, i64::from(i32::MAX)) as i32
}

/// Ceiling of `a / b` for positive `b`, in [`i64`].
fn div_ceil_i64(a: i64, b: i64) -> i64 {
    debug_assert!(b > 0, "divisor must be positive");
    (a + b - 1) / b
}

/// Range check that yields a [`GenError::OutOfRange`] naming the field on failure.
fn check_range(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), GenError> {
    if value < min || value > max {
        Err(GenError::OutOfRange {
            field,
            value,
            min,
            max,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky130;

    fn build(params: &TestStructureParams) -> Cell {
        let mut cell = Cell::new("top");
        TestStructure
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_cross_validates_and_generates() {
        let p = TestStructureParams::default();
        p.validate().expect("default valid");
        let cell = build(&p);
        assert_eq!(cell.shapes.len(), 2, "a cross is two bars");
        assert!(cell.shapes.iter().all(|s| s.layer == sky130::LI1.layer));
    }

    #[test]
    fn contact_chain_encloses_every_contact_in_met1() {
        // Every mcon must sit inside some met1 rectangle by at least the m1.4 margin.
        let p = TestStructureParams {
            kind: StructureKind::ContactChain,
            count: 7, // odd count: last bridge is li1, exercising the met1 end pad
            ..TestStructureParams::default()
        };
        p.validate().expect("valid");
        let cell = build(&p);
        let met1: Vec<Rect> = cell
            .shapes
            .iter()
            .filter(|s| s.layer == sky130::MET1.layer)
            .map(|s| match s.kind {
                ShapeKind::Rect(r) => r,
                _ => unreachable!(),
            })
            .collect();
        let contacts: Vec<Rect> = cell
            .shapes
            .iter()
            .filter(|s| s.layer == sky130::MCON.layer)
            .map(|s| match s.kind {
                ShapeKind::Rect(r) => r,
                _ => unreachable!(),
            })
            .collect();
        let enc = i64::from(sky130::MCON.enclosure.expect("mcon enclosure").1);
        for c in &contacts {
            let enclosed = met1.iter().any(|m| {
                m.min.x <= c.min.x
                    && m.min.y <= c.min.y
                    && c.max.x <= m.max.x
                    && c.max.y <= m.max.y
                    && i64::from(c.min.x - m.min.x) >= enc
                    && i64::from(c.min.y - m.min.y) >= enc
                    && i64::from(m.max.x - c.max.x) >= enc
                    && i64::from(m.max.y - c.max.y) >= enc
            });
            assert!(enclosed, "contact {c:?} not enclosed by met1 by {enc}");
        }
    }

    #[test]
    fn comb_and_serpentine_generate_on_layer() {
        for kind in [StructureKind::Comb, StructureKind::Serpentine] {
            let p = TestStructureParams {
                kind,
                layer: StructureLayer::Met2,
                feature_width: 300,
                feature_length: 3_000,
                count: 5,
            };
            p.validate().expect("valid");
            let cell = build(&p);
            assert!(!cell.shapes.is_empty());
            assert!(
                cell.shapes.iter().all(|s| s.layer == sky130::MET2.layer),
                "{kind:?} all on met2"
            );
        }
    }

    #[test]
    fn thin_feature_rejected() {
        let p = TestStructureParams {
            layer: StructureLayer::Met1,
            feature_width: 100, // below met1 min width 140
            ..TestStructureParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange {
                field: "feature_width",
                ..
            })
        ));
    }

    #[test]
    fn short_feature_rejected_by_length_floor() {
        // A met1 serpentine with a 300-wide line has a length floor of
        // max(ceil(83000/300)=277, 2*300+140=740) = 740; a length of 200 is rejected
        // naming `feature_length`.
        let p = TestStructureParams {
            kind: StructureKind::Serpentine,
            layer: StructureLayer::Met1,
            feature_width: 300,
            feature_length: 200,
            count: 4,
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange {
                field: "feature_length",
                ..
            })
        ));
    }

    #[test]
    fn serpentine_geometric_floor_enforced() {
        // A serpentine bar must be at least 2*width + min_spacing long so the two
        // end-links clear each other. On li1 (spacing 170) at width 800 that is
        // 1770; a length between the area floor (tiny here) and 1770 must be rejected,
        // while 1770 and up validate.
        let base = TestStructureParams {
            kind: StructureKind::Serpentine,
            layer: StructureLayer::Li1,
            feature_width: 800,
            count: 3,
            ..TestStructureParams::default()
        };
        let too_short = TestStructureParams {
            feature_length: 1_700,
            ..base.clone()
        };
        assert!(matches!(
            too_short.validate(),
            Err(GenError::OutOfRange {
                field: "feature_length",
                ..
            })
        ));
        let ok = TestStructureParams {
            feature_length: 1_770,
            ..base
        };
        ok.validate().expect("2*width + spacing is long enough");
    }

    #[test]
    fn count_out_of_range_rejected() {
        let p = TestStructureParams {
            kind: StructureKind::Comb,
            count: 200, // above 128
            ..TestStructureParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange { field: "count", .. })
        ));
    }

    #[test]
    fn div_ceil_and_isqrt_are_exact() {
        assert_eq!(div_ceil_i64(83_000, 300), 277); // 276*300=82800 < 83000
        assert_eq!(div_ceil_i64(56_100, 170), 330); // 170*330 = 56100 exactly
        assert_eq!(div_ceil_i64(56_101, 170), 331); // one dbu more rounds up
        assert_eq!(isqrt_ceil_i32(83_000), 289);
    }
}

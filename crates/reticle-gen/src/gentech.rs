//! `GenTech`: the technology numbers the generators draw against, as **data**.
//!
//! Every generator in this crate needs the same small set of per-process numbers:
//! the conductor layers it may route on (with their minimum width, spacing, and
//! optional minimum area), the cut layers that bridge adjacent conductors (with
//! their exact drawn size and the enclosure a covering plate owes them), the
//! substrate-tap contact under the base conductor, and a conservative cut pitch for
//! layers the rule deck gives no cut-to-cut spacing. [`GenTech`] gathers exactly
//! those numbers into one value.
//!
//! # Why this exists
//!
//! Earlier revisions baked these numbers into the generators as SKY130 constants.
//! That made every generator SKY130-only and scattered the process data through the
//! topology code. `GenTech` turns the numbers into a value the generators read: the
//! ring/serpentine/fill/array *topology* stays code, the *numbers* are data, and the
//! same generator runs against any process that can supply a `GenTech`. The crate
//! ships two: [`GenTech::sky130`] and [`GenTech::sg13g2`], selected from a
//! [`Technology`] by name via [`GenTech::for_technology`].
//!
//! # Roles, not names
//!
//! A `GenTech` is four stacked interconnect [`Conductor`]s (index 0 is the base
//! interconnect, index 3 the top), three [`Cut`]s where `cut[i]` bridges
//! `conductor[i]` and `conductor[i+1]`, and one substrate-tap cut enclosed by the
//! base conductor. The generators address these by *role* (level index), so
//! `RingLayer::Li1` means "the base interconnect" on SKY130 and "Metal1" on SG13G2:
//! the same structural role, different physical layer. See the second-PDK chapter of
//! the book for the full cross-process role table.
//!
//! # Derivation and the committed decks
//!
//! [`GenTech::sky130`]/[`GenTech::sg13g2`] are authored constants, but each is tied
//! to its committed DRC subset: [`derive`] reconstructs a `GenTech` from a parsed
//! [`Technology`]'s rules (and cross-checks the stack ordering), and a test asserts
//! `derive(committed deck) == the constant`. So a drift between these numbers and the
//! committed rule deck fails a test rather than shipping. The runtime path stays a
//! plain constant (no file I/O, no allocation, `wasm32`-clean); the derivation proves
//! the constant is faithful to the data.

use reticle_geometry::LayerId;
use reticle_model::{Rule, RuleKind, Technology};

/// A conductor layer the generators may route on, with the width, spacing, and
/// (optional) minimum-area numbers its process attaches to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Conductor {
    /// The GDSII layer address.
    pub layer: LayerId,
    /// Minimum drawn width in DBU.
    pub min_width: i32,
    /// Minimum same-layer spacing in DBU.
    pub min_spacing: i32,
    /// Minimum shape area in DBU², or `None` if the deck carries no area rule for
    /// this layer.
    pub min_area: Option<i64>,
}

/// A cut layer (a contact or via), with its exact drawn size and, when a covering
/// conductor owes it one, the enclosure margin (DBU) and the nominal enclosing
/// layer.
///
/// The generators grow *both* covering plates by the enclosure margin, so the stored
/// enclosing [`LayerId`] is nominal (the level above the cut); the margin is the load
/// bearing number and is chosen as the largest enclosure the deck asks of the cut, so
/// growing both plates by it satisfies every enclosure rule on the cut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cut {
    /// The GDSII layer address of the cut.
    pub layer: LayerId,
    /// The exact drawn cut size in DBU.
    pub size: i32,
    /// The margin (DBU) a covering conductor keeps around the cut, tagged with the
    /// nominal enclosing layer, or `None` when neither the deck nor a residue supplies
    /// one.
    pub enclosure: Option<(LayerId, i32)>,
}

/// The generator-facing technology: four stacked interconnect conductors, the three
/// cuts bridging them, the substrate-tap cut, and a conservative cut pitch margin.
///
/// Construct one of the built-ins with [`GenTech::sky130`] or [`GenTech::sg13g2`], or
/// pick from a [`Technology`] by name with [`GenTech::for_technology`]. See the
/// [module docs](self) for the role model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GenTech {
    /// The process name (`"sky130"`, `"sg13g2"`).
    pub name: &'static str,
    /// Interconnect conductors, bottom-to-top (index 0 is the base interconnect).
    conductors: [Conductor; 4],
    /// Cuts, bottom-to-top: `cuts[i]` bridges `conductors[i]` and `conductors[i + 1]`.
    cuts: [Cut; 3],
    /// The substrate-tap contact enclosed by the base conductor (`conductors[0]`).
    tap_cut: Cut,
    /// A conservative cut-to-cut pitch margin for layers the deck gives no cut
    /// spacing: cuts are pitched at their size plus this, so no two cuts crowd.
    safe_cut_margin: i32,
}

impl GenTech {
    /// Assembles a `GenTech` from its parts. `const` so the built-ins are plain
    /// constants; the derivation in [`derive`] proves each built-in matches its deck.
    #[must_use]
    pub const fn new(
        name: &'static str,
        conductors: [Conductor; 4],
        cuts: [Cut; 3],
        tap_cut: Cut,
        safe_cut_margin: i32,
    ) -> Self {
        Self {
            name,
            conductors,
            cuts,
            tap_cut,
            safe_cut_margin,
        }
    }

    /// The interconnect conductor at stack `level` (0 = base interconnect, up to 3 =
    /// top). Panics if `level >= 4`; the generator enums only ever pass 0..=3.
    #[must_use]
    pub fn conductor(&self, level: usize) -> Conductor {
        self.conductors[level]
    }

    /// The cut at stack `level` (0..=2), which bridges [`conductor(level)`](Self::conductor)
    /// and [`conductor(level + 1)`](Self::conductor).
    #[must_use]
    pub fn cut(&self, level: usize) -> Cut {
        self.cuts[level]
    }

    /// The lower conductor `cut(level)` bridges (`conductor(level)`).
    #[must_use]
    pub fn cut_lower(&self, level: usize) -> Conductor {
        self.conductors[level]
    }

    /// The upper conductor `cut(level)` bridges (`conductor(level + 1)`).
    #[must_use]
    pub fn cut_upper(&self, level: usize) -> Conductor {
        self.conductors[level + 1]
    }

    /// The substrate-tap contact enclosed by the base conductor.
    #[must_use]
    pub fn tap_cut(&self) -> Cut {
        self.tap_cut
    }

    /// The conservative cut-to-cut pitch margin (see the field docs).
    #[must_use]
    pub fn safe_cut_margin(&self) -> i32 {
        self.safe_cut_margin
    }

    /// The top interconnect conductor (`conductor(3)`), where the pad ring draws.
    #[must_use]
    pub fn top(&self) -> Conductor {
        self.conductors[self.conductors.len() - 1]
    }

    /// The interconnect conductors, bottom-to-top.
    #[must_use]
    pub fn conductors(&self) -> &[Conductor] {
        &self.conductors
    }

    /// The cuts, bottom-to-top (`cuts[i]` bridges `conductors[i]`/`conductors[i+1]`).
    #[must_use]
    pub fn cuts(&self) -> &[Cut] {
        &self.cuts
    }

    /// The built-in SKY130 generator technology.
    #[must_use]
    pub const fn sky130() -> GenTech {
        crate::sky130::GENTECH
    }

    /// The built-in IHP SG13G2 generator technology.
    #[must_use]
    pub const fn sg13g2() -> GenTech {
        crate::sg13g2::GENTECH
    }

    /// SKY130's [`Residue`] (the role binding plus the numbers the deck omits), for
    /// reconstructing [`sky130`](Self::sky130) from a parsed [`Technology`] via
    /// [`derive_gentech`].
    pub const SKY130_RESIDUE: Residue = crate::sky130::RESIDUE;

    /// SG13G2's [`Residue`], for reconstructing [`sg13g2`](Self::sg13g2) via
    /// [`derive_gentech`].
    pub const SG13G2_RESIDUE: Residue = crate::sg13g2::RESIDUE;

    /// Picks the built-in `GenTech` for a [`Technology`] by its name, defaulting to
    /// [`sky130`](Self::sky130) for an unrecognized or empty name.
    ///
    /// This is how a generator turns the [`Technology`] argument threaded into
    /// [`generate`](crate::Generator::generate) into the numbers it draws against. The
    /// SKY130 default keeps every caller that passes [`Technology::default`] (the app,
    /// the tests, the agent) on exactly the SKY130 numbers the generators shipped with.
    #[must_use]
    pub fn for_technology(tech: &Technology) -> GenTech {
        match tech.name.as_str() {
            "sg13g2" | "ihp-sg13g2" | "ihp_sg13g2" => Self::sg13g2(),
            _ => Self::sky130(),
        }
    }
}

/// The per-process "residue": the role assignment and the conservative numbers the
/// DRC subset genuinely lacks, which a rule deck alone cannot supply.
///
/// A rule deck says "layer 68/44 has width 150 and this enclosure"; it does not say
/// "68/44 is the via that bridges the first and second interconnect levels", nor does
/// it carry a cut-to-cut pitch where the deck omits one. The residue supplies exactly
/// that structural binding plus those missing numbers, so [`derive`] can turn a
/// parsed [`Technology`] into a [`GenTech`]. It is intentionally tiny.
#[derive(Clone, Copy, Debug)]
pub struct Residue {
    /// The process name.
    pub name: &'static str,
    /// The four interconnect conductor layers, bottom-to-top.
    pub conductors: [LayerId; 4],
    /// The three cut layers, bottom-to-top (`cuts[i]` bridges `conductors[i]` and
    /// `conductors[i + 1]`).
    pub cuts: [LayerId; 3],
    /// The substrate-tap contact layer, enclosed by `conductors[0]`.
    pub tap_cut: LayerId,
    /// A conservative enclosure margin (DBU) for each cut whose deck carries no
    /// enclosure rule, or `None` to leave the cut unenclosed.
    pub cut_fallback_enclosure: [Option<i32>; 3],
    /// A conservative enclosure margin (DBU) for the tap cut when the deck carries no
    /// base-conductor enclosure rule for it.
    pub tap_fallback_enclosure: Option<i32>,
    /// The conservative cut-to-cut pitch margin.
    pub safe_cut_margin: i32,
}

/// Reconstructs a [`GenTech`] from a parsed [`Technology`]'s rules and stack plus a
/// [`Residue`], proving the built-in constants are faithful to their committed deck.
///
/// The width, spacing, area, cut size, and enclosure numbers all come from the
/// technology's [`rules`](Technology::rules); the residue supplies only the role
/// assignment and the numbers the deck lacks (see [`Residue`]). When the technology
/// declares a physical [`stack`](Technology::stack), the conductors are cross-checked
/// to sit in strictly increasing z-order, so a residue that lists the levels out of
/// order fails loudly.
///
/// # Errors
///
/// Returns a message naming the layer if a required width or spacing rule is missing,
/// or if the stack ordering disagrees with the residue's conductor order.
pub fn derive_gentech(tech: &Technology, residue: &Residue) -> Result<GenTech, String> {
    let rules = &tech.rules;

    let mut conductors = [conductor_from(rules, residue.conductors[0])?; 4];
    for (slot, &layer) in conductors.iter_mut().zip(residue.conductors.iter()) {
        *slot = conductor_from(rules, layer)?;
    }

    let mut cuts = [cut_from(
        rules,
        residue.cuts[0],
        residue.conductors[1],
        residue.cut_fallback_enclosure[0],
    )?; 3];
    for (i, slot) in cuts.iter_mut().enumerate() {
        *slot = cut_from(
            rules,
            residue.cuts[i],
            residue.conductors[i + 1],
            residue.cut_fallback_enclosure[i],
        )?;
    }

    let tap_cut = tap_from(
        rules,
        residue.tap_cut,
        residue.conductors[0],
        residue.tap_fallback_enclosure,
    )?;

    if !tech.stack.is_empty() {
        verify_stack_order(tech, &residue.conductors)?;
    }

    Ok(GenTech::new(
        residue.name,
        conductors,
        cuts,
        tap_cut,
        residue.safe_cut_margin,
    ))
}

/// The single value of the first rule matching `kind`/`layer`/`other_layer`, if any.
fn rule_value(
    rules: &[Rule],
    kind: RuleKind,
    layer: LayerId,
    other: Option<LayerId>,
) -> Option<i64> {
    rules
        .iter()
        .find(|r| r.kind == kind && r.layer == layer && r.other_layer == other)
        .map(|r| r.value)
}

/// The largest enclosure margin any rule asks of `cut_layer` (by any conductor), if
/// the deck carries an enclosure rule for it. Growing both covering plates by this
/// satisfies every enclosure rule on the cut.
fn max_enclosure(rules: &[Rule], cut_layer: LayerId) -> Option<i64> {
    rules
        .iter()
        .filter(|r| r.kind == RuleKind::Enclosure && r.layer == cut_layer)
        .map(|r| r.value)
        .max()
}

/// Builds a [`Conductor`] from the deck's width/spacing/area rules for `layer`.
fn conductor_from(rules: &[Rule], layer: LayerId) -> Result<Conductor, String> {
    let min_width = rule_value(rules, RuleKind::Width, layer, None)
        .ok_or_else(|| format!("no width rule for conductor {layer:?}"))?;
    let min_spacing = rule_value(rules, RuleKind::Spacing, layer, None)
        .ok_or_else(|| format!("no spacing rule for conductor {layer:?}"))?;
    let min_area = rule_value(rules, RuleKind::Area, layer, None);
    Ok(Conductor {
        layer,
        min_width: clamp_i32(min_width),
        min_spacing: clamp_i32(min_spacing),
        min_area,
    })
}

/// Builds a [`Cut`] from the deck's width and enclosure rules for `cut_layer`, tagging
/// the enclosure with the nominal `upper` conductor and falling back to `fallback`
/// where the deck carries no enclosure.
fn cut_from(
    rules: &[Rule],
    cut_layer: LayerId,
    upper: LayerId,
    fallback: Option<i32>,
) -> Result<Cut, String> {
    let size = rule_value(rules, RuleKind::Width, cut_layer, None)
        .ok_or_else(|| format!("no width rule for cut {cut_layer:?}"))?;
    let margin = max_enclosure(rules, cut_layer).map(clamp_i32).or(fallback);
    Ok(Cut {
        layer: cut_layer,
        size: clamp_i32(size),
        enclosure: margin.map(|m| (upper, m)),
    })
}

/// Builds the substrate-tap [`Cut`] from the deck's width and its enclosure by the
/// `base` conductor, falling back to `fallback` where the deck carries no such
/// enclosure.
fn tap_from(
    rules: &[Rule],
    tap_layer: LayerId,
    base: LayerId,
    fallback: Option<i32>,
) -> Result<Cut, String> {
    let size = rule_value(rules, RuleKind::Width, tap_layer, None)
        .ok_or_else(|| format!("no width rule for tap {tap_layer:?}"))?;
    let margin = rule_value(rules, RuleKind::Enclosure, tap_layer, Some(base))
        .map(clamp_i32)
        .or(fallback);
    Ok(Cut {
        layer: tap_layer,
        size: clamp_i32(size),
        enclosure: margin.map(|m| (base, m)),
    })
}

/// Checks the residue's conductor order against the technology's physical stack:
/// every conductor with a stack entry must sit strictly above the previous one.
fn verify_stack_order(tech: &Technology, conductors: &[LayerId; 4]) -> Result<(), String> {
    let mut prev_bottom: Option<i64> = None;
    for &layer in conductors {
        let Some(entry) = tech.stack_for(layer) else {
            continue; // a conductor with no stack entry is not ordered here
        };
        if let Some(prev) = prev_bottom
            && entry.z_bottom_nm <= prev
        {
            return Err(format!(
                "conductor {layer:?} sits at z {} which is not above the previous {prev}",
                entry.z_bottom_nm
            ));
        }
        prev_bottom = Some(entry.z_bottom_nm);
    }
    Ok(())
}

/// Narrows a widened rule value into the DBU (`i32`) coordinate range, saturating.
fn clamp_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

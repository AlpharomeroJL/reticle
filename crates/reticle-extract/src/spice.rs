//! SPICE netlist export: an extracted [`DeviceNetlist`] written out as a
//! SPICE subcircuit.
//!
//! This module sits a level above [device recognition](crate::device): it takes
//! the devices and terminal nets [`extract_devices`](crate::extract_devices)
//! already recovered and formats them as `.subckt` / `X` device-instance cards /
//! `.ends` / `.end`, the exchange subset committed at
//! `tests/fixtures/contracts/spice_exchange_inverter.spice` (its structural
//! twin, `.json`, is what the cross-test in `tests/spice_writer.rs` checks
//! against). External simulators and schematic-capture tools (the `xschem`
//! lane) read this subset back.
//!
//! # What is written, and from where
//!
//! - **`.subckt NAME <ports...>`**: `NAME` and the DBU-to-micron resolution are
//!   supplied by the caller (a [`DeviceNetlist`] has neither -- see
//!   [`to_spice_subckt`]); ports are the nets any device terminal references,
//!   in the extracted netlist's own stable order.
//! - **One `X` card per device**: `Xn <drain> <gate> <source> <bulk> <model>
//!   w=<W> l=<L>`, in [`DeviceNetlist::devices`] order (`n` is that order's
//!   index). `W`/`L` convert from the [`Device`] DBU fields to decimal microns
//!   by exact integer long division, never a float, so the text is byte-stable.
//! - **the model name**: from [`SpiceTech`], a small caller-supplied table
//!   keyed only on [`DeviceKind`] (NMOS/PMOS) -- see its doc comment for why.
//!
//! # What is honestly absent
//!
//! - Area/perimeter parameters (`ad`/`pd`/`as`/`ps`): [`Device`] carries no
//!   diffusion-area data, so these are never invented (see device.rs's own
//!   scope note).
//! - A terminal `extract_devices` could not bind to a net ([`Device::gate_net`]
//!   and friends are `Option<usize>`) is written as the documented placeholder
//!   [`UNBOUND_NODE`], never a guessed net name.
//! - A general SPICE *importer*: [`parse_spice`] reads back only the subset
//!   [`format_spice`] writes (enough to round-trip this writer's own output and
//!   the committed contract fixture); it is not hardened for arbitrary decks.
//!
//! ```
//! use reticle_extract::device::{DeviceKind, DeviceTech, extract_devices};
//! use reticle_extract::spice::{SpiceTech, write_spice};
//! use reticle_geometry::{Point, Rect};
//! use reticle_model::{Cell, Document, DrawShape, ShapeKind};
//!
//! // A trivial NMOS: poly crossing an n+ diffusion (no PMOS, no labels).
//! let tech = DeviceTech::sky130();
//! let mut cell = Cell::new("top");
//! let rect = |layer, x0, y0, x1, y1| {
//!     DrawShape::new(layer, ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))))
//! };
//! cell.shapes.push(rect(tech.diff, 0, 0, 100, 40));
//! cell.shapes.push(rect(tech.poly, 40, -10, 60, 50));
//! cell.shapes.push(rect(tech.nsdm, -5, -5, 105, 45));
//! let mut doc = Document::new();
//! doc.insert_cell(cell);
//!
//! let dnl = extract_devices(&doc, "top", &tech);
//! assert_eq!(dnl.count_of(DeviceKind::Nmos), 1);
//! let text = write_spice(&dnl, "trivial_nmos", 1000, &SpiceTech::sky130());
//! assert!(text.contains(".subckt trivial_nmos"));
//! assert!(text.contains("sky130_fd_pr__nfet_01v8"));
//! ```

use std::collections::HashSet;
use std::fmt;
use std::fmt::Write as _;

use crate::device::{Device, DeviceKind, DeviceNetlist};

/// The node name written for a device terminal [`extract_devices`](crate::extract_devices)
/// could not bind to a net (a [`Device`] terminal field of `None`).
///
/// Written literally as the placeholder text `"NC"`, documented, never a
/// guessed or invented net name. See device.rs's [`Device`] doc comment for why
/// a terminal can be unbound (an untapped body, an isolated diffusion lobe).
pub const UNBOUND_NODE: &str = "NC";

/// The PDK SPICE model name to write for each recognised [`DeviceKind`].
///
/// [`DeviceKind`] distinguishes only NMOS/PMOS (the channel type); it carries
/// no threshold-voltage flavour (standard/high/low-Vt), body-bias variant, or
/// any other model-selection detail, because device recognition reads none of
/// that from geometry (see device.rs's scope note). `SpiceTech` is therefore a
/// single model name per kind, supplied by the caller, mirroring how
/// [`DeviceTech`](crate::DeviceTech) supplies fixed technology layer numbers
/// device recognition cannot derive either. [`sky130`](Self::sky130) is a
/// reasonable plain default; a caller that knows a specific extracted device
/// is, say, a high-Vt part builds its own `SpiceTech` with that model name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiceTech {
    /// The model name written for [`DeviceKind::Nmos`] devices.
    pub nmos_model: String,
    /// The model name written for [`DeviceKind::Pmos`] devices.
    pub pmos_model: String,
}

impl SpiceTech {
    /// The plain SKY130 primitive device models (standard-Vt):
    /// `sky130_fd_pr__nfet_01v8` / `sky130_fd_pr__pfet_01v8`.
    #[must_use]
    pub fn sky130() -> Self {
        Self {
            nmos_model: "sky130_fd_pr__nfet_01v8".to_owned(),
            pmos_model: "sky130_fd_pr__pfet_01v8".to_owned(),
        }
    }

    /// The model name for `kind`.
    fn model_for(&self, kind: DeviceKind) -> &str {
        match kind {
            DeviceKind::Nmos => &self.nmos_model,
            DeviceKind::Pmos => &self.pmos_model,
        }
    }
}

/// One `X` device-instance card: a recognised device's terminals, model, and
/// channel dimensions, already resolved to the text SPICE writes (net names
/// and exact decimal-micron `w`/`l` strings, not the [`Device`]'s indices and
/// DBU).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiceDevice {
    /// The card's instance name (`X0`, `X1`, ... in [`DeviceNetlist::devices`]
    /// order).
    pub name: String,
    /// The drain node name.
    pub drain: String,
    /// The gate node name.
    pub gate: String,
    /// The source node name.
    pub source: String,
    /// The bulk (body) node name.
    pub bulk: String,
    /// The PDK model name (from [`SpiceTech`]).
    pub model: String,
    /// Channel width in decimal microns, exact (see [`format_spice`]'s doc for
    /// why this is a string, not a float).
    pub w: String,
    /// Channel length in decimal microns, exact.
    pub l: String,
}

/// A SPICE subcircuit: the structured form [`to_spice_subckt`] builds from a
/// [`DeviceNetlist`], [`format_spice`] renders to text, and [`parse_spice`]
/// reads back. This is the unit a cross-test compares against the exchange
/// contract (see this module's doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiceSubckt {
    /// The subcircuit name (the `.subckt` line's first token).
    pub name: String,
    /// The subcircuit's ports: the nets any device terminal references, in the
    /// source netlist's own stable order.
    pub ports: Vec<String>,
    /// The device-instance cards, in [`DeviceNetlist::devices`] order.
    pub devices: Vec<SpiceDevice>,
}

/// Builds the structured SPICE subcircuit for `dnl`, named `name`.
///
/// `dbu_per_micron` is the document's technology resolution (see
/// `Technology::dbu_per_micron` in `reticle_model`); a non-positive value is
/// treated as `1`, matching the rest of Reticle's DBU-to-micron conversions.
/// `tech` supplies the PDK model name per device kind ([`SpiceTech`]).
/// Deterministic: `dnl`'s devices and nets are already stably ordered
/// ([`DeviceNetlist`]'s own contract), so the same inputs always build the
/// same structure.
#[must_use]
pub fn to_spice_subckt(
    dnl: &DeviceNetlist,
    name: &str,
    dbu_per_micron: i64,
    tech: &SpiceTech,
) -> SpiceSubckt {
    let devices = dnl
        .devices
        .iter()
        .enumerate()
        .map(|(index, device)| spice_device(dnl, index, device, dbu_per_micron, tech))
        .collect();
    SpiceSubckt {
        name: name.to_owned(),
        ports: ports_of(dnl),
        devices,
    }
}

/// Builds one device's `X`-card fields.
fn spice_device(
    dnl: &DeviceNetlist,
    index: usize,
    device: &Device,
    dbu_per_micron: i64,
    tech: &SpiceTech,
) -> SpiceDevice {
    SpiceDevice {
        name: format!("X{index}"),
        drain: node_name(dnl, device.drain_net),
        gate: node_name(dnl, device.gate_net),
        source: node_name(dnl, device.source_net),
        bulk: node_name(dnl, device.bulk_net),
        model: tech.model_for(device.kind).to_owned(),
        w: format_microns(device.width, dbu_per_micron),
        l: format_microns(device.length, dbu_per_micron),
    }
}

/// The node name for a terminal: the bound net's name, or the documented
/// [`UNBOUND_NODE`] placeholder when `terminal` is `None` (or, defensively, an
/// index [`DeviceNetlist::net_name`] cannot resolve).
fn node_name(dnl: &DeviceNetlist, terminal: Option<usize>) -> String {
    dnl.net_name(terminal)
        .map_or_else(|| UNBOUND_NODE.to_owned(), str::to_owned)
}

/// The subcircuit's ports: the distinct nets any device terminal references,
/// kept in `dnl.nets`'s own index order (its documented stable,
/// lowest-member-index order) rather than re-derived from device-scan order,
/// so this writer invents no ordering of its own.
fn ports_of(dnl: &DeviceNetlist) -> Vec<String> {
    let mut referenced: HashSet<usize> = HashSet::new();
    for device in &dnl.devices {
        for net_index in [
            device.gate_net,
            device.drain_net,
            device.source_net,
            device.bulk_net,
        ]
        .into_iter()
        .flatten()
        {
            referenced.insert(net_index);
        }
    }
    dnl.nets
        .nets
        .iter()
        .enumerate()
        .filter(|(index, _)| referenced.contains(index))
        .map(|(_, net)| net.name.clone())
        .collect()
}

/// Converts a DBU span to an exact decimal-micron string.
///
/// Plain integer long division, not a float: `dbu / dbu_per_micron` is always
/// rational, and for every `dbu_per_micron` Reticle actually uses (a power of
/// ten -- `1000` for SKY130) it terminates in at most a handful of digits, so
/// the text this produces is exact and byte-stable (no `0.6500000001`-style
/// float-formatting drift). Trailing zeros are trimmed (`1000` DBU at 1000
/// DBU/micron is `"1"`, not `"1.0"`) and a whole value drops the decimal point
/// entirely. Capped at `MAX_FRACTION_DIGITS` digits so a `dbu_per_micron` with
/// prime factors other than 2 and 5 (not a real Reticle technology today)
/// still terminates, rounded down at the cap, rather than looping. A
/// non-positive `dbu_per_micron` is treated as `1`.
fn format_microns(dbu: i64, dbu_per_micron: i64) -> String {
    const MAX_FRACTION_DIGITS: u32 = 9;

    let denom = dbu_per_micron.max(1).unsigned_abs();
    let negative = dbu < 0;
    let magnitude = dbu.unsigned_abs();
    let whole = magnitude / denom;
    let mut remainder = magnitude % denom;

    let mut fraction = String::new();
    for _ in 0..MAX_FRACTION_DIGITS {
        if remainder == 0 {
            break;
        }
        remainder *= 10;
        let digit = remainder / denom;
        fraction.push(char::from(b'0' + digit as u8));
        remainder %= denom;
    }
    while fraction.ends_with('0') {
        fraction.pop();
    }

    let sign = if negative && (whole != 0 || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// Renders `subckt` as SPICE text in the Reticle exchange subset: a leading
/// identifying comment, `.subckt NAME <ports...>`, one `X` card per device
/// (`Xname drain gate source bulk model w=W l=L`), `.ends`, `.end`. The
/// inverse of [`parse_spice`].
#[must_use]
pub fn format_spice(subckt: &SpiceSubckt) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "* Reticle SPICE export: {}", subckt.name);
    let _ = write!(out, ".subckt {}", subckt.name);
    for port in &subckt.ports {
        let _ = write!(out, " {port}");
    }
    out.push('\n');
    for device in &subckt.devices {
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} w={} l={}",
            device.name,
            device.drain,
            device.gate,
            device.source,
            device.bulk,
            device.model,
            device.w,
            device.l
        );
    }
    out.push_str(".ends\n.end\n");
    out
}

/// Builds `dnl`'s [`SpiceSubckt`] and renders it to text in one call
/// ([`to_spice_subckt`] then [`format_spice`]).
#[must_use]
pub fn write_spice(
    dnl: &DeviceNetlist,
    name: &str,
    dbu_per_micron: i64,
    tech: &SpiceTech,
) -> String {
    format_spice(&to_spice_subckt(dnl, name, dbu_per_micron, tech))
}

/// An error [`parse_spice`] returns for text outside the Reticle exchange
/// subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpiceParseError {
    /// No `.subckt` line was found (the first non-blank, non-comment line is
    /// not `.subckt`, or the text has no such line at all).
    MissingSubckt,
    /// The `.subckt` line has no name token after the keyword.
    SubcktMissingName,
    /// A device card does not have the required name plus four terminal nodes
    /// plus a model name (the malformed line, verbatim).
    MalformedDevice(String),
    /// A device card is missing its `w=` or `l=` parameter.
    MissingParam {
        /// The device's own card name (e.g. `"X0"`).
        device: String,
        /// Which parameter is missing: `"w"` or `"l"`.
        param: &'static str,
    },
    /// The `.subckt` block was never closed with a matching `.ends`.
    MissingEnds,
    /// The subcircuit is not terminated with a trailing `.end`.
    MissingEnd,
}

impl fmt::Display for SpiceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpiceParseError::MissingSubckt => write!(f, "no .subckt line found"),
            SpiceParseError::SubcktMissingName => {
                write!(f, ".subckt line has no subcircuit name")
            }
            SpiceParseError::MalformedDevice(line) => {
                write!(f, "malformed device card: {line:?}")
            }
            SpiceParseError::MissingParam { device, param } => {
                write!(f, "device {device} is missing {param}=")
            }
            SpiceParseError::MissingEnds => write!(f, "no .ends closing the .subckt block"),
            SpiceParseError::MissingEnd => write!(f, "no trailing .end"),
        }
    }
}

impl std::error::Error for SpiceParseError {}

/// Parses SPICE text back into a [`SpiceSubckt`], the round-trip inverse of
/// [`format_spice`].
///
/// Accepts exactly the Reticle exchange subset: blank lines and `*` full-line
/// comments are skipped anywhere; one `.subckt` / `X`-card* / `.ends` / `.end`
/// block; unrecognised `key=value` device params (e.g. a foreign deck's
/// `ad=`/`pd=`/`as=`/`ps=`) are skipped rather than rejected, but `w=` and
/// `l=` must both be present. This exists to round-trip this writer's own
/// output (and to validate the committed contract fixture) for the cross-test
/// in `tests/spice_writer.rs`; it is not a hardened importer for arbitrary,
/// untrusted SPICE decks. It never panics on malformed input (every branch
/// returns [`SpiceParseError`] instead), but has not been fuzzed the way
/// Reticle's binary-format readers (GDS, OASIS) have.
pub fn parse_spice(text: &str) -> Result<SpiceSubckt, SpiceParseError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('*'));

    let header = lines.next().ok_or(SpiceParseError::MissingSubckt)?;
    let mut header_tokens = header.split_whitespace();
    let keyword = header_tokens.next().unwrap_or_default();
    if !keyword.eq_ignore_ascii_case(".subckt") {
        return Err(SpiceParseError::MissingSubckt);
    }
    let name = header_tokens
        .next()
        .ok_or(SpiceParseError::SubcktMissingName)?
        .to_owned();
    let ports: Vec<String> = header_tokens.map(str::to_owned).collect();

    let mut devices = Vec::new();
    let mut saw_ends = false;
    for line in lines.by_ref() {
        if line.eq_ignore_ascii_case(".ends") {
            saw_ends = true;
            break;
        }
        devices.push(parse_device_card(line)?);
    }
    if !saw_ends {
        return Err(SpiceParseError::MissingEnds);
    }

    let saw_end = lines
        .next()
        .is_some_and(|line| line.eq_ignore_ascii_case(".end"));
    if !saw_end {
        return Err(SpiceParseError::MissingEnd);
    }

    Ok(SpiceSubckt {
        name,
        ports,
        devices,
    })
}

/// Parses one `X` device-instance card.
fn parse_device_card(line: &str) -> Result<SpiceDevice, SpiceParseError> {
    let malformed = || SpiceParseError::MalformedDevice(line.to_owned());
    let mut tokens = line.split_whitespace();

    let name = tokens.next().ok_or_else(malformed)?;
    if !(name.starts_with('X') || name.starts_with('x')) {
        return Err(malformed());
    }
    let name = name.to_owned();
    let drain = tokens.next().ok_or_else(malformed)?.to_owned();
    let gate = tokens.next().ok_or_else(malformed)?.to_owned();
    let source = tokens.next().ok_or_else(malformed)?.to_owned();
    let bulk = tokens.next().ok_or_else(malformed)?.to_owned();
    let model = tokens.next().ok_or_else(malformed)?.to_owned();

    let mut w = None;
    let mut l = None;
    for token in tokens {
        let (key, value) = token.split_once('=').ok_or_else(malformed)?;
        match key {
            "w" => w = Some(value.to_owned()),
            "l" => l = Some(value.to_owned()),
            _ => {} // Out of this writer's scope (e.g. ad=/pd=/as=/ps=): ignored, not rejected.
        }
    }
    let w = w.ok_or_else(|| SpiceParseError::MissingParam {
        device: name.clone(),
        param: "w",
    })?;
    let l = l.ok_or_else(|| SpiceParseError::MissingParam {
        device: name.clone(),
        param: "l",
    })?;

    Ok(SpiceDevice {
        name,
        drain,
        gate,
        source,
        bulk,
        model,
        w,
        l,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlist::{Net, Netlist};

    /// A tiny synthetic device netlist (no geometry/extraction): one NMOS with
    /// an unbound bulk, so unit tests can exercise the writer/parser without
    /// building layout.
    fn tiny_dnl() -> DeviceNetlist {
        let nets = Netlist::new(vec![
            Net::new("A", vec![0]),
            Net::new("Y", vec![1]),
            Net::new("VGND", vec![2]),
        ]);
        let device = Device {
            kind: DeviceKind::Nmos,
            gate_net: Some(0),
            drain_net: Some(1),
            source_net: Some(2),
            bulk_net: None,
            width: 650,
            length: 150,
        };
        DeviceNetlist {
            devices: vec![device],
            nets,
        }
    }

    #[test]
    fn unbound_terminal_emits_the_documented_placeholder() {
        let subckt = to_spice_subckt(&tiny_dnl(), "test", 1000, &SpiceTech::sky130());
        assert_eq!(subckt.devices[0].bulk, UNBOUND_NODE);
        assert_eq!(subckt.devices[0].bulk, "NC");
    }

    #[test]
    fn ports_are_only_nets_a_device_terminal_references() {
        let mut dnl = tiny_dnl();
        dnl.nets.nets.push(Net::new("unused", vec![3])); // not any terminal's net
        let subckt = to_spice_subckt(&dnl, "test", 1000, &SpiceTech::sky130());
        assert_eq!(
            subckt.ports,
            vec!["A".to_owned(), "Y".to_owned(), "VGND".to_owned()]
        );
    }

    #[test]
    fn format_microns_is_exact_not_rounded() {
        assert_eq!(format_microns(650, 1000), "0.65");
        assert_eq!(format_microns(150, 1000), "0.15");
        assert_eq!(format_microns(1000, 1000), "1");
        assert_eq!(format_microns(0, 1000), "0");
        assert_eq!(format_microns(1, 1000), "0.001");
        assert_eq!(format_microns(1500, 1000), "1.5");
    }

    #[test]
    fn sky130_default_models_have_no_vt_flavour() {
        let tech = SpiceTech::sky130();
        assert_eq!(tech.nmos_model, "sky130_fd_pr__nfet_01v8");
        assert_eq!(tech.pmos_model, "sky130_fd_pr__pfet_01v8");
    }

    #[test]
    fn write_then_parse_round_trips() {
        let dnl = tiny_dnl();
        let tech = SpiceTech::sky130();
        let text = write_spice(&dnl, "test", 1000, &tech);
        let parsed = parse_spice(&text).expect("the writer's own output parses");
        assert_eq!(parsed, to_spice_subckt(&dnl, "test", 1000, &tech));
    }

    #[test]
    fn parse_skips_comments_and_blank_lines() {
        let text = "* header comment\n\n.subckt test a b\n* inline\nX0 a b a b model w=1 l=1\n\n.ends\n.end\n";
        let parsed = parse_spice(text).expect("comments and blanks are skipped");
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.ports, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(parsed.devices.len(), 1);
    }

    #[test]
    fn parse_rejects_missing_subckt() {
        let text = "X0 a b c d model w=1 l=1\n.ends\n.end\n";
        assert_eq!(parse_spice(text), Err(SpiceParseError::MissingSubckt));
    }

    #[test]
    fn parse_rejects_missing_ends() {
        assert_eq!(
            parse_spice(".subckt test a b\n"),
            Err(SpiceParseError::MissingEnds)
        );
    }

    #[test]
    fn parse_rejects_missing_end() {
        let text = ".subckt test a b\nX0 a b a b model w=1 l=1\n.ends\n";
        assert_eq!(parse_spice(text), Err(SpiceParseError::MissingEnd));
    }

    #[test]
    fn parse_rejects_device_missing_a_param() {
        let text = ".subckt test a b\nX0 a b a b model w=1\n.ends\n.end\n";
        assert_eq!(
            parse_spice(text),
            Err(SpiceParseError::MissingParam {
                device: "X0".to_owned(),
                param: "l"
            })
        );
    }

    #[test]
    fn parse_rejects_a_malformed_device_line() {
        let text = ".subckt test a b\nX0 a b\n.ends\n.end\n";
        assert!(matches!(
            parse_spice(text),
            Err(SpiceParseError::MalformedDevice(_))
        ));
    }

    #[test]
    fn parse_ignores_out_of_scope_params() {
        // ad=/pd=/as=/ps= (area/perimeter) are out of this writer's scope but a
        // foreign deck may carry them; the parser skips rather than rejects.
        let text = ".subckt test a b\nX0 a b a b model ad=1 w=1 pd=2 l=1 ps=3\n.ends\n.end\n";
        let parsed = parse_spice(text).expect("unrecognised params are skipped, not rejected");
        assert_eq!(parsed.devices[0].w, "1");
        assert_eq!(parsed.devices[0].l, "1");
    }
}

//! SPICE export and xschem probe-list import (v8.2 Phase 3, `xschem` lane).
//!
//! Two pieces, both fixture-first against the committed SPICE exchange contract
//! (`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.{spice,json}`)
//! so this lane does not block on the `netlist` lane's writer:
//!
//! - [`SpiceNetlist`] and [`write_spice`]: a Rust mirror of the contract's structural
//!   JSON shape and a pure writer that emits its `.subckt`/`X`/`.ends`/`.end` textual
//!   form. The tests read the committed fixture JSON directly into this shape, so a
//!   future change to the fixture is caught rather than silently drifting from a
//!   hand-copied duplicate.
//! - [`ProbeQuantity`], [`Probe`], and [`parse_probe_list`]: a small, capped parser
//!   for an xschem-style probe list (`id node quantity` per line), reading untrusted
//!   input. `ProbeQuantity` mirrors `reticle_sim::Quantity`'s three variants and its
//!   `snake_case` wire strings (`"voltage"`, `"current"`, `"charge"`) without adding a
//!   dependency on `reticle-sim`: wiring that workspace dependency into
//!   `reticle-app`'s `Cargo.toml` is the `waveform-ui` lane's owned edit (see its
//!   brief), outside this lane's owned paths.
//!
//! [`spice_netlist_from_devices`] bridges the real, already-merged device
//! recognition (`reticle_extract::extract_devices`) into the exchange contract shape,
//! so `file.export_spice` exports the open design today rather than waiting on the
//! `netlist` lane's writer to merge. It is a deliberately small, temporary bridge;
//! see its doc comment and `docs/decisions/0112-xschem-interop.md` for the ledger.
//!
//! Untrusted input: [`parse_probe_list`] caps its accepted size
//! ([`MAX_PROBE_LIST_BYTES`]) and probe count ([`MAX_PROBES`]) before doing any
//! per-line work, and every malformed line is a structured [`XschemError`], never a
//! panic (a wasm panic kills the tab).

use std::collections::HashSet;
use std::fmt;

use reticle_extract::{DeviceKind, DeviceNetlist};

// ---------------------------------------------------------------------------
// F4-compatible probe descriptors
// ---------------------------------------------------------------------------

/// The physical quantity a probe measures.
///
/// Structurally and textually compatible with `reticle_sim::Quantity`: the same
/// three variants, the same `snake_case` wire strings ([`as_str`](Self::as_str) /
/// [`parse`](Self::parse)). Defined locally rather than imported (see the module
/// doc) so this lane stays inside its owned paths; a later wiring lane can promote
/// an [`XschemState`] probe into a real `reticle_sim::Probe` once a `WaveformSet`
/// exists, since the wire representation already matches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeQuantity {
    /// A node voltage.
    Voltage,
    /// A branch current.
    Current,
    /// A node charge.
    Charge,
}

impl ProbeQuantity {
    /// The wire string (`"voltage"` / `"current"` / `"charge"`), matching
    /// `reticle_sim::Quantity`'s `serde(rename_all = "snake_case")` representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProbeQuantity::Voltage => "voltage",
            ProbeQuantity::Current => "current",
            ProbeQuantity::Charge => "charge",
        }
    }

    /// Parses a wire string back into a quantity, or `None` for anything else
    /// (never panics on an unrecognised value).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "voltage" => Some(Self::Voltage),
            "current" => Some(Self::Current),
            "charge" => Some(Self::Charge),
            _ => None,
        }
    }
}

/// An imported probe descriptor: a node to watch and what to measure there, before
/// any simulation has run.
///
/// The import-side counterpart of `reticle_sim::Probe`, minus the recorded
/// `samples_nano` series (nothing has been simulated yet). `id` is the stable key a
/// later `WaveformSet` probe would carry; `node` is the extracted netlist node name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Probe {
    /// Stable id for this probe (unique within an imported list).
    pub id: String,
    /// The extracted netlist node this probe follows.
    pub node: String,
    /// What to measure at `node`.
    pub quantity: ProbeQuantity,
}

// ---------------------------------------------------------------------------
// The SPICE exchange contract (structural mirror of
// spice_exchange_inverter.json)
// ---------------------------------------------------------------------------

/// A device instance's four terminal net names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SpiceDeviceTerminals {
    /// The drain terminal's net name.
    pub drain: String,
    /// The gate terminal's net name.
    pub gate: String,
    /// The source terminal's net name.
    pub source: String,
    /// The bulk (body) terminal's net name.
    pub bulk: String,
}

/// One `X` subcircuit-device card: an instance name, its four terminals, the PDK
/// model, and its width/length in decimal microns (pre-formatted strings, matching
/// the contract fixture's byte-stable convention; see [`write_spice`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SpiceDevice {
    /// NMOS or PMOS.
    pub kind: DeviceKind,
    /// The instance name (`"X0"`, `"X1"`, ...).
    pub name: String,
    /// The four terminal net names.
    pub terminals: SpiceDeviceTerminals,
    /// The PDK model name.
    pub model: String,
    /// Channel width, a decimal-micron string (for example `"0.65"`).
    pub w: String,
    /// Channel length, a decimal-micron string (for example `"0.15"`).
    pub l: String,
}

/// A SPICE subcircuit in the exchange contract's structural shape: mirrors
/// `spice_exchange_inverter.json`'s `subckt`/`ports`/`nodes`/`devices` fields.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SpiceNetlist {
    /// The subcircuit name (`.subckt NAME ...`).
    pub subckt: String,
    /// The `.subckt` port list, in declaration order.
    pub ports: Vec<String>,
    /// Every net name referenced anywhere in the subcircuit (a structural JSON
    /// field; [`write_spice`] does not render it directly).
    pub nodes: Vec<String>,
    /// The device instances, in card order.
    pub devices: Vec<SpiceDevice>,
}

/// Writes `netlist` as the exchange contract's SPICE text: a `.subckt` header, one
/// `X` card per device, `.ends`, then `.end`.
///
/// Pure and deterministic: given the same `netlist`, always the same text. `w`/`l`
/// are written verbatim (already decimal-micron strings), so this writer never
/// touches a float and cannot drift the way a `f64` formatter can (see the contract
/// fixture's own note on float-formatting drift). Matches the netlist lane's own
/// success-bar wording: asserted STRUCTURE, not necessarily byte-identical to a
/// human-authored fixture that also carries a `*` comment header this writer does
/// not reproduce.
#[must_use]
pub fn write_spice(netlist: &SpiceNetlist) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if netlist.ports.is_empty() {
        let _ = writeln!(out, ".subckt {}", netlist.subckt);
    } else {
        let _ = writeln!(
            out,
            ".subckt {} {}",
            netlist.subckt,
            netlist.ports.join(" ")
        );
    }
    for d in &netlist.devices {
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} w={} l={}",
            d.name,
            d.terminals.drain,
            d.terminals.gate,
            d.terminals.source,
            d.terminals.bulk,
            d.model,
            d.w,
            d.l
        );
    }
    out.push_str(".ends\n.end\n");
    out
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A parse failure from [`parse_probe_list`]: an oversized or malformed probe
/// list. Every variant is a structured, honest description; nothing in this
/// module panics on untrusted input.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum XschemError {
    /// The probe-list input exceeds [`MAX_PROBE_LIST_BYTES`].
    TooLarge {
        /// The input's actual size in bytes.
        bytes: usize,
        /// The accepted maximum.
        max: usize,
    },
    /// The probe-list input is not valid UTF-8 text.
    InvalidUtf8,
    /// A non-comment, non-blank line does not have exactly three
    /// whitespace-separated fields (`id node quantity`).
    MalformedLine {
        /// The 1-based line number.
        line: usize,
        /// The offending line's trimmed text.
        text: String,
    },
    /// A line's quantity field is not `voltage`, `current`, or `charge`.
    UnknownQuantity {
        /// The 1-based line number.
        line: usize,
        /// The unrecognised quantity text.
        value: String,
    },
    /// Two lines import the same probe id.
    DuplicateId {
        /// The 1-based line number of the second occurrence.
        line: usize,
        /// The repeated id.
        id: String,
    },
    /// The probe list has more entries than [`MAX_PROBES`] accepts.
    TooManyProbes {
        /// The accepted maximum.
        max: usize,
    },
}

impl fmt::Display for XschemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, max } => {
                write!(f, "probe list is {bytes} bytes, over the {max}-byte cap")
            }
            Self::InvalidUtf8 => write!(f, "probe list is not valid UTF-8 text"),
            Self::MalformedLine { line, text } => {
                write!(f, "line {line}: expected `id node quantity`, got {text:?}")
            }
            Self::UnknownQuantity { line, value } => write!(
                f,
                "line {line}: unknown quantity {value:?} (expected voltage, current, or charge)"
            ),
            Self::DuplicateId { line, id } => {
                write!(f, "line {line}: duplicate probe id {id:?}")
            }
            Self::TooManyProbes { max } => {
                write!(f, "probe list has more than {max} probes")
            }
        }
    }
}

impl std::error::Error for XschemError {}

// ---------------------------------------------------------------------------
// Untrusted-input probe-list parser
// ---------------------------------------------------------------------------

/// The accepted maximum size of a probe-list input, checked before any per-line
/// work. A probe list is a small, human-authored text file (the committed contract
/// fixture's "probes" array is under 200 bytes); 1 MiB is generous for that use
/// while refusing a pathological input up front, matching the workspace's
/// untrusted-input convention (every parser caps count-driven allocation against
/// remaining bytes).
pub const MAX_PROBE_LIST_BYTES: usize = 1 << 20;

/// The accepted maximum number of probes in one list, independent of byte size (so
/// many short lines are capped the same way as few long ones).
pub const MAX_PROBES: usize = 10_000;

/// Parses `bytes` as an xschem-style probe list: one probe per line, `<id> <node>
/// <quantity>` separated by whitespace, `#` starts a full-line comment, blank lines
/// are skipped.
///
/// This is Reticle's own minimal probe-list interchange subset (xschem's native
/// schematic file format beyond the probe list is out of scope; see
/// `docs/src/xschem-interop.md`), not a byte-for-byte port of any xschem file.
///
/// Untrusted-input hardening: `bytes.len()` is checked against
/// [`MAX_PROBE_LIST_BYTES`] before any UTF-8 decoding or line scanning, and the
/// probe count is checked against [`MAX_PROBES`] before each push, so a
/// pathological input is rejected with a structured [`XschemError`] rather than
/// growing memory unboundedly. Invalid UTF-8 and every malformed line is likewise a
/// structured error; nothing here panics.
pub fn parse_probe_list(bytes: &[u8]) -> Result<Vec<Probe>, XschemError> {
    if bytes.len() > MAX_PROBE_LIST_BYTES {
        return Err(XschemError::TooLarge {
            bytes: bytes.len(),
            max: MAX_PROBE_LIST_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| XschemError::InvalidUtf8)?;

    let mut probes = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(XschemError::MalformedLine {
                line: line_no,
                text: line.to_owned(),
            });
        }
        let (id, node, quantity_str) = (fields[0], fields[1], fields[2]);
        let quantity =
            ProbeQuantity::parse(quantity_str).ok_or_else(|| XschemError::UnknownQuantity {
                line: line_no,
                value: quantity_str.to_owned(),
            })?;
        if !seen_ids.insert(id.to_owned()) {
            return Err(XschemError::DuplicateId {
                line: line_no,
                id: id.to_owned(),
            });
        }
        if probes.len() >= MAX_PROBES {
            return Err(XschemError::TooManyProbes { max: MAX_PROBES });
        }
        probes.push(Probe {
            id: id.to_owned(),
            node: node.to_owned(),
            quantity,
        });
    }
    Ok(probes)
}

// ---------------------------------------------------------------------------
// Live bridge: DeviceNetlist -> SpiceNetlist
// ---------------------------------------------------------------------------

/// The two-entry `kind -> model` table this bridge emits, matching the model names
/// the committed contract fixture names for its recognised kinds
/// (`spice_exchange_inverter.json`). A stand-in for the `netlist` lane's real
/// `DeviceKind` -> tech model table (its brief: "small tech table (nfet/pfet PDK
/// model)"); see [`spice_netlist_from_devices`] for why this bridge exists.
fn bridge_model_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Nmos => "sky130_fd_pr__nfet_01v8",
        DeviceKind::Pmos => "sky130_fd_pr__pfet_01v8_hvt",
    }
}

/// Formats `dbu` database units as an exact decimal-micron string at
/// `dbu_per_micron` resolution.
///
/// Pure integer long division, never a `f64`: `650` at `1000` dbu/micron is exactly
/// `"0.65"`, matching the contract fixture's convention of pre-formatted,
/// byte-stable decimal strings (its own comment: "so the fixture is byte-stable and
/// free of float-formatting drift"). The fractional part is truncated (not
/// rounded) at `MAX_FRAC_DIGITS` digits for a `dbu_per_micron` whose decimal
/// expansion does not terminate (a resolution that is not a product of only 2s and
/// 5s, for example thirds); every resolution this codebase actually uses (1000, and
/// similar powers of ten) terminates well inside that cap.
fn dbu_to_micron_string(dbu: i64, dbu_per_micron: i64) -> String {
    const MAX_FRAC_DIGITS: u32 = 9;
    let dpm = dbu_per_micron.unsigned_abs().max(1);
    let negative = dbu < 0;
    let magnitude = dbu.unsigned_abs();
    let whole = magnitude / dpm;
    let mut remainder = magnitude % dpm;

    let mut frac = String::new();
    for _ in 0..MAX_FRAC_DIGITS {
        if remainder == 0 {
            break;
        }
        remainder *= 10;
        let digit = remainder / dpm;
        frac.push(char::from(b'0' + digit as u8));
        remainder %= dpm;
    }
    while frac.ends_with('0') {
        frac.pop();
    }

    let mut out = whole.to_string();
    if !frac.is_empty() {
        out.push('.');
        out.push_str(&frac);
    }
    if negative && (whole != 0 || !frac.is_empty()) {
        out.insert(0, '-');
    }
    out
}

/// Resolves one device terminal to a net name, honestly: a bound terminal uses
/// [`DeviceNetlist::net_name`] (already carrying extraction's own `net_<n>`
/// fallback for an unnamed net); a genuinely unbound terminal (`None`, most often
/// an unresolved bulk) gets a synthetic `unbound_<device>_<terminal>` placeholder
/// unique to that terminal, so an honestly-absent connection is visible rather than
/// silently merged with another device's unbound terminal of the same name.
fn resolve_terminal(
    dn: &DeviceNetlist,
    device_idx: usize,
    terminal: Option<usize>,
    label: &str,
    nodes: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> String {
    let name = dn
        .net_name(terminal)
        .map_or_else(|| format!("unbound_{device_idx}_{label}"), str::to_owned);
    if seen.insert(name.clone()) {
        nodes.push(name.clone());
    }
    name
}

/// Bridges extracted device recognition into the SPICE exchange contract shape, so
/// `file.export_spice` exports the open design today rather than waiting on the
/// `netlist` lane's writer to merge (the brief's ledger-if-squeezed allowance: wire
/// export against the contract structure and gate the live extract-to-emit call
/// behind the merged API).
///
/// This is a deliberately small, temporary bridge, not the `netlist` lane's real
/// writer:
/// - [`dbu_to_micron_string`] is exact (pure integer long division), so it does not
///   carry float-formatting drift, but [`bridge_model_name`]'s two-entry table only
///   covers the technology the committed contract fixture names. The `netlist`
///   lane's brief owns the real `DeviceKind` -> tech model table and the DBU ->
///   decimal-micron conversion in `reticle_extract::spice`; once it merges, prefer
///   calling it directly over this bridge (see `docs/decisions/0112-xschem-interop.md`).
/// - `ports` and `nodes` are not distinguished: both are every named net a device
///   terminal resolves to, in first-appearance order over the devices' stable
///   order (gate, source, drain, bulk per device). Determining true I/O-boundary
///   pins needs pin/label information this bridge does not have.
#[must_use]
pub fn spice_netlist_from_devices(
    dn: &DeviceNetlist,
    subckt: &str,
    dbu_per_micron: i64,
) -> SpiceNetlist {
    let mut nodes: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut devices = Vec::with_capacity(dn.devices.len());

    for (i, d) in dn.devices.iter().enumerate() {
        let terminals = SpiceDeviceTerminals {
            drain: resolve_terminal(dn, i, d.drain_net, "d", &mut nodes, &mut seen),
            gate: resolve_terminal(dn, i, d.gate_net, "g", &mut nodes, &mut seen),
            source: resolve_terminal(dn, i, d.source_net, "s", &mut nodes, &mut seen),
            bulk: resolve_terminal(dn, i, d.bulk_net, "b", &mut nodes, &mut seen),
        };
        devices.push(SpiceDevice {
            kind: d.kind,
            name: format!("X{i}"),
            terminals,
            model: bridge_model_name(d.kind).to_owned(),
            w: dbu_to_micron_string(d.width, dbu_per_micron),
            l: dbu_to_micron_string(d.length, dbu_per_micron),
        });
    }

    SpiceNetlist {
        subckt: subckt.to_owned(),
        ports: nodes.clone(),
        nodes,
        devices,
    }
}

// ---------------------------------------------------------------------------
// App-facing state
// ---------------------------------------------------------------------------

/// The xschem-interop app state: the probes most recently imported from an
/// xschem-style probe list (see [`parse_probe_list`]).
///
/// Ready for a later lane to visualize or feed into a `WaveformSet` once the
/// sim-engine lands (`docs/decisions/0112-xschem-interop.md`); this lane owns
/// import and parsing only, not a rendering surface (egui-free by design).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct XschemState {
    /// The most recently imported probe list. An import replaces this outright
    /// (a fresh selection, not an accumulation across imports).
    pub probes: Vec<Probe>,
}

impl XschemState {
    /// An empty state: nothing imported yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `bytes` as an xschem-style probe list ([`parse_probe_list`]) and, on
    /// success, replaces [`probes`](Self::probes) with the result, returning the
    /// new count. Leaves the previous probe list untouched on a parse error.
    pub fn import(&mut self, bytes: &[u8]) -> Result<usize, XschemError> {
        let probes = parse_probe_list(bytes)?;
        let n = probes.len();
        self.probes = probes;
        Ok(n)
    }

    /// A one-line, comma-joined summary of the current probe list (`"in (A,
    /// voltage), out (Y, voltage)"`), for a status line or toast; empty when
    /// nothing has been imported.
    #[must_use]
    pub fn summary(&self) -> String {
        self.probes
            .iter()
            .map(|p| format!("{} ({}, {})", p.id, p.node, p.quantity.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_extract::Device;

    const CONTRACT_JSON: &str =
        include_str!("../../reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.json");
    const CONTRACT_SPICE: &str = include_str!(
        "../../reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.spice"
    );

    /// Reticle's own xschem-probe-list sample: whitespace-separated `id node
    /// quantity`, a full-line `#` comment, and a blank line, content-equivalent to
    /// the contract JSON's `probes` array.
    const SAMPLE_PROBE_LIST: &str = "\
# Reticle xschem-probe-list interchange format sample (Phase 3, xschem lane).
in A voltage

out Y voltage
";

    // --- ProbeQuantity ---

    #[test]
    fn probe_quantity_round_trips_through_its_wire_strings() {
        for (q, s) in [
            (ProbeQuantity::Voltage, "voltage"),
            (ProbeQuantity::Current, "current"),
            (ProbeQuantity::Charge, "charge"),
        ] {
            assert_eq!(q.as_str(), s);
            assert_eq!(ProbeQuantity::parse(s), Some(q));
        }
        assert_eq!(ProbeQuantity::parse("bogus"), None);
        assert_eq!(ProbeQuantity::parse(""), None);
    }

    // --- contract JSON reader (test-only: reads the committed, trusted fixture
    // directly with `serde_json::Value` + `.expect()` rather than a
    // production-facing parser, so a future edit to the fixture is caught here
    // instead of drifting from a hand-copied duplicate) ---

    /// Reads the committed SPICE exchange contract fixture's JSON structural form
    /// into [`SpiceNetlist`] and its probe list. Test-only: the fixture is trusted
    /// and committed, so this uses plain indexing and `.expect()` rather than the
    /// structured-error discipline [`parse_probe_list`] (the real untrusted-input
    /// entry point) uses.
    fn read_contract_fixture() -> (SpiceNetlist, Vec<Probe>) {
        let value: serde_json::Value =
            serde_json::from_str(CONTRACT_JSON).expect("fixture is valid JSON");
        let str_field = |v: &serde_json::Value, field: &str| -> String {
            v.get(field)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("fixture field `{field}` missing or not a string"))
                .to_owned()
        };
        let str_array = |v: &serde_json::Value, field: &str| -> Vec<String> {
            v.get(field)
                .and_then(serde_json::Value::as_array)
                .unwrap_or_else(|| panic!("fixture field `{field}` missing or not an array"))
                .iter()
                .map(|e| e.as_str().expect("array entry is a string").to_owned())
                .collect()
        };

        let devices = value["devices"]
            .as_array()
            .expect("fixture has a devices array")
            .iter()
            .map(|d| {
                let kind = match str_field(d, "kind").as_str() {
                    "nmos" => DeviceKind::Nmos,
                    "pmos" => DeviceKind::Pmos,
                    other => panic!("unrecognised device kind {other:?} in fixture"),
                };
                let terminals = &d["terminals"];
                let params = &d["params"];
                SpiceDevice {
                    kind,
                    name: str_field(d, "name"),
                    terminals: SpiceDeviceTerminals {
                        drain: str_field(terminals, "drain"),
                        gate: str_field(terminals, "gate"),
                        source: str_field(terminals, "source"),
                        bulk: str_field(terminals, "bulk"),
                    },
                    model: str_field(d, "model"),
                    w: str_field(params, "w"),
                    l: str_field(params, "l"),
                }
            })
            .collect();

        let probes = value["probes"]
            .as_array()
            .expect("fixture has a probes array")
            .iter()
            .map(|p| Probe {
                id: str_field(p, "id"),
                node: str_field(p, "node"),
                quantity: ProbeQuantity::parse(&str_field(p, "quantity")).expect("known quantity"),
            })
            .collect();

        (
            SpiceNetlist {
                subckt: str_field(&value, "subckt"),
                ports: str_array(&value, "ports"),
                nodes: str_array(&value, "nodes"),
                devices,
            },
            probes,
        )
    }

    #[test]
    fn contract_json_parses_the_inverter_fixture() {
        let (netlist, probes) = read_contract_fixture();
        assert_eq!(netlist.subckt, "sky130_fd_sc_hd__inv_1");
        assert_eq!(netlist.ports, vec!["VPB", "VNB", "VGND", "VPWR", "A", "Y"]);
        assert_eq!(
            netlist.nodes, netlist.ports,
            "fixture nodes equal ports for this inverter"
        );
        assert_eq!(netlist.devices.len(), 2);

        let nmos = &netlist.devices[0];
        assert_eq!(nmos.kind, DeviceKind::Nmos);
        assert_eq!(nmos.name, "X0");
        assert_eq!(nmos.model, "sky130_fd_pr__nfet_01v8");
        assert_eq!(nmos.w, "0.65");
        assert_eq!(nmos.l, "0.15");
        assert_eq!(nmos.terminals.drain, "Y");
        assert_eq!(nmos.terminals.gate, "A");
        assert_eq!(nmos.terminals.source, "VGND");
        assert_eq!(nmos.terminals.bulk, "VNB");

        let pmos = &netlist.devices[1];
        assert_eq!(pmos.kind, DeviceKind::Pmos);
        assert_eq!(pmos.model, "sky130_fd_pr__pfet_01v8_hvt");
        assert_eq!(pmos.w, "1");

        assert_eq!(
            probes,
            vec![
                Probe {
                    id: "in".to_owned(),
                    node: "A".to_owned(),
                    quantity: ProbeQuantity::Voltage,
                },
                Probe {
                    id: "out".to_owned(),
                    node: "Y".to_owned(),
                    quantity: ProbeQuantity::Voltage,
                },
            ]
        );
    }

    // --- write_spice: the export path emits the exchange-contract structure ---

    #[test]
    fn write_spice_emits_the_committed_contract_structure() {
        let (netlist, _probes) = read_contract_fixture();
        let text = write_spice(&netlist);

        for line in [
            ".subckt sky130_fd_sc_hd__inv_1 VPB VNB VGND VPWR A Y",
            "X0 Y A VGND VNB sky130_fd_pr__nfet_01v8 w=0.65 l=0.15",
            "X1 Y A VPWR VPB sky130_fd_pr__pfet_01v8_hvt w=1 l=0.15",
            ".ends",
            ".end",
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "missing line {line:?} in:\n{text}"
            );
        }

        // Structural, not byte-identical (the committed fixture also carries a `*`
        // comment header this writer does not reproduce; matches the netlist lane's
        // own success-bar wording): every non-comment, non-blank fixture line
        // appears, in order, exactly as this writer emits it.
        let fixture_body: Vec<&str> = CONTRACT_SPICE
            .lines()
            .filter(|l| !l.trim_start().starts_with('*') && !l.trim().is_empty())
            .collect();
        let written: Vec<&str> = text.lines().collect();
        assert_eq!(written, fixture_body);
    }

    #[test]
    fn write_spice_of_an_empty_netlist_is_still_valid_skeleton() {
        let text = write_spice(&SpiceNetlist {
            subckt: "EMPTY".to_owned(),
            ..SpiceNetlist::default()
        });
        assert_eq!(text, ".subckt EMPTY\n.ends\n.end\n");
    }

    // --- parse_probe_list: the untrusted-input entry point ---

    #[test]
    fn probe_fixture_parses_into_the_contract_probes() {
        let (_netlist, contract_probes) = read_contract_fixture();
        let parsed = parse_probe_list(SAMPLE_PROBE_LIST.as_bytes()).expect("sample parses");
        assert_eq!(parsed, contract_probes);
    }

    #[test]
    fn probe_list_empty_input_is_zero_probes_not_an_error() {
        assert_eq!(parse_probe_list(b"").unwrap(), Vec::new());
        assert_eq!(
            parse_probe_list(b"\n\n# just a comment\n").unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn probe_list_rejects_malformed_line_without_panicking() {
        assert_eq!(
            parse_probe_list(b"in A\n"),
            Err(XschemError::MalformedLine {
                line: 1,
                text: "in A".to_owned(),
            })
        );
        assert_eq!(
            parse_probe_list(b"in A voltage extra\n"),
            Err(XschemError::MalformedLine {
                line: 1,
                text: "in A voltage extra".to_owned(),
            })
        );
    }

    #[test]
    fn probe_list_rejects_unknown_quantity() {
        assert_eq!(
            parse_probe_list(b"in A resistance\n"),
            Err(XschemError::UnknownQuantity {
                line: 1,
                value: "resistance".to_owned(),
            })
        );
    }

    #[test]
    fn probe_list_rejects_duplicate_id() {
        assert_eq!(
            parse_probe_list(b"in A voltage\nin B current\n"),
            Err(XschemError::DuplicateId {
                line: 2,
                id: "in".to_owned(),
            })
        );
    }

    #[test]
    fn probe_list_rejects_invalid_utf8_without_panicking() {
        assert_eq!(
            parse_probe_list(&[0xff, 0xfe, 0x00]),
            Err(XschemError::InvalidUtf8)
        );
    }

    #[test]
    fn probe_list_rejects_oversized_input_before_scanning_it() {
        let huge = vec![b'a'; MAX_PROBE_LIST_BYTES + 1];
        assert_eq!(
            parse_probe_list(&huge),
            Err(XschemError::TooLarge {
                bytes: MAX_PROBE_LIST_BYTES + 1,
                max: MAX_PROBE_LIST_BYTES,
            })
        );
    }

    #[test]
    fn probe_list_caps_probe_count_rather_than_growing_unbounded() {
        use std::fmt::Write as _;
        let mut text = String::new();
        for i in 0..=MAX_PROBES {
            let _ = writeln!(text, "p{i} n{i} voltage");
        }
        assert!(
            text.len() < MAX_PROBE_LIST_BYTES,
            "test input must stay under the byte cap"
        );
        assert_eq!(
            parse_probe_list(text.as_bytes()),
            Err(XschemError::TooManyProbes { max: MAX_PROBES })
        );
    }

    #[test]
    fn probe_list_trims_whitespace_and_skips_comments_and_blank_lines() {
        let probes =
            parse_probe_list(b"  in   A   voltage  \n\n# a comment\n\tout\tY\tvoltage\t\n")
                .expect("parses");
        assert_eq!(probes.len(), 2);
        assert_eq!(probes[0].id, "in");
        assert_eq!(probes[1].id, "out");
    }

    // --- XschemState ---

    #[test]
    fn xschem_state_import_replaces_rather_than_accumulates() {
        let mut state = XschemState::new();
        assert!(state.probes.is_empty());
        assert_eq!(state.import(SAMPLE_PROBE_LIST.as_bytes()).unwrap(), 2);
        assert_eq!(state.probes.len(), 2);

        assert_eq!(state.import(b"solo N voltage\n").unwrap(), 1);
        assert_eq!(
            state.probes.len(),
            1,
            "import replaces, does not accumulate"
        );
        assert_eq!(state.probes[0].id, "solo");
    }

    #[test]
    fn xschem_state_import_error_leaves_the_previous_list_untouched() {
        let mut state = XschemState::new();
        state.import(SAMPLE_PROBE_LIST.as_bytes()).unwrap();
        assert_eq!(state.probes.len(), 2);

        assert!(state.import(b"malformed\n").is_err());
        assert_eq!(
            state.probes.len(),
            2,
            "a failed import must not clear or corrupt state"
        );
    }

    // --- dbu_to_micron_string ---

    #[test]
    fn dbu_to_micron_string_matches_the_contract_fixture_values() {
        assert_eq!(dbu_to_micron_string(650, 1000), "0.65");
        assert_eq!(dbu_to_micron_string(1000, 1000), "1");
        assert_eq!(dbu_to_micron_string(150, 1000), "0.15");
    }

    #[test]
    fn dbu_to_micron_string_handles_zero_and_negative_values() {
        assert_eq!(dbu_to_micron_string(0, 1000), "0");
        assert_eq!(dbu_to_micron_string(-650, 1000), "-0.65");
        assert_eq!(dbu_to_micron_string(-1000, 1000), "-1");
    }

    #[test]
    fn dbu_to_micron_string_truncates_a_nonterminating_expansion_without_panicking() {
        // 1/3000 does not terminate in decimal; truncated (not rounded) at the
        // fractional-digit cap rather than looping unboundedly.
        let s = dbu_to_micron_string(1, 3000);
        assert!(s.starts_with("0.000333"), "got {s:?}");
    }

    // --- spice_netlist_from_devices: the live bridge ---

    fn hand_built_inverter() -> DeviceNetlist {
        // Mirrors spice_exchange_inverter.json's two devices without needing real
        // extracted geometry: constructed directly, matching device.rs's own
        // public `Device`/`DeviceNetlist` shape.
        let nets = reticle_extract::Netlist::new(vec![
            reticle_extract::Net::new("VPB", vec![0]),
            reticle_extract::Net::new("VNB", vec![1]),
            reticle_extract::Net::new("VGND", vec![2]),
            reticle_extract::Net::new("VPWR", vec![3]),
            reticle_extract::Net::new("A", vec![4]),
            reticle_extract::Net::new("Y", vec![5]),
        ]);
        DeviceNetlist {
            devices: vec![
                Device {
                    kind: DeviceKind::Nmos,
                    gate_net: Some(4),
                    source_net: Some(2),
                    drain_net: Some(5),
                    bulk_net: Some(1),
                    width: 650,
                    length: 150,
                },
                Device {
                    kind: DeviceKind::Pmos,
                    gate_net: Some(4),
                    source_net: Some(3),
                    drain_net: Some(5),
                    bulk_net: Some(0),
                    width: 1000,
                    length: 150,
                },
            ],
            nets,
        }
    }

    #[test]
    fn spice_netlist_from_devices_matches_the_contract_for_a_hand_built_inverter() {
        let dn = hand_built_inverter();
        let sn = spice_netlist_from_devices(&dn, "sky130_fd_sc_hd__inv_1", 1000);
        let text = write_spice(&sn);
        assert!(text.contains(".subckt sky130_fd_sc_hd__inv_1"));
        assert!(text.contains("X0 Y A VGND VNB sky130_fd_pr__nfet_01v8 w=0.65 l=0.15"));
        assert!(text.contains("X1 Y A VPWR VPB sky130_fd_pr__pfet_01v8_hvt w=1 l=0.15"));
    }

    #[test]
    fn spice_netlist_from_devices_names_an_unbound_terminal_honestly() {
        let dn = DeviceNetlist {
            devices: vec![Device {
                kind: DeviceKind::Nmos,
                gate_net: Some(0),
                source_net: None,
                drain_net: Some(0),
                bulk_net: None,
                width: 100,
                length: 100,
            }],
            nets: reticle_extract::Netlist::new(vec![reticle_extract::Net::new("A", vec![0])]),
        };
        let sn = spice_netlist_from_devices(&dn, "TOP", 1000);
        assert_eq!(sn.devices[0].terminals.gate, "A");
        assert_eq!(sn.devices[0].terminals.drain, "A");
        assert_eq!(sn.devices[0].terminals.source, "unbound_0_s");
        assert_eq!(sn.devices[0].terminals.bulk, "unbound_0_b");
        // "A" appears once in nodes/ports despite being referenced by two
        // terminals (deduplicated, not double-listed).
        assert_eq!(sn.nodes.iter().filter(|n| n.as_str() == "A").count(), 1);
    }

    #[test]
    fn spice_netlist_from_devices_of_no_devices_is_an_empty_but_valid_subckt() {
        let dn = DeviceNetlist::default();
        let sn = spice_netlist_from_devices(&dn, "EMPTY", 1000);
        assert!(sn.devices.is_empty());
        assert!(sn.ports.is_empty());
        assert_eq!(write_spice(&sn), ".subckt EMPTY\n.ends\n.end\n");
    }
}

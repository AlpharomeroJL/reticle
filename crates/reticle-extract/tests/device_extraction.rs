//! Device-recognition tests: MOSFETs from SKY130 layer geometry.
//!
//! A gate is the intersection of a poly shape over a diffusion shape; NMOS vs
//! PMOS is decided by the surrounding implant/well. These tests build small
//! SKY130 layouts by hand (the same style as `extraction.rs`) so the expected
//! device list is hand-verifiable, and check both directions: a real gate is
//! found with the right kind and terminal nets, and a near-miss (poly not
//! crossing diff) yields no spurious device.

mod common;

use common::{DIFF, NSDM, POLY, doc_with, inverter, rect};
use reticle_extract::device::{
    Device, DeviceKind, DeviceNetlist, DeviceTech, extract_devices, extract_devices_labeled,
};

/// The device of the given kind (the fixtures have one of each).
fn device_of(dnl: &DeviceNetlist, kind: DeviceKind) -> &Device {
    dnl.devices
        .iter()
        .find(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("a {kind:?} device is present"))
}

#[test]
fn poly_crossing_ndiff_forms_one_nmos() {
    // A horizontal n+ diffusion with a vertical poly stripe crossing it: one NMOS
    // whose channel (poly over diff) separates the two source/drain lobes.
    let doc = doc_with(vec![
        rect(DIFF, 0, 0, 100, 40),   // active
        rect(POLY, 40, -10, 60, 50), // gate stripe, extends past diff (poly endcap)
        rect(NSDM, -5, -5, 105, 45), // n+ select over the whole diff → NMOS
    ]);
    let dnl = extract_devices(&doc, "top", &DeviceTech::sky130());
    assert_eq!(dnl.devices.len(), 1, "one transistor");
    assert_eq!(dnl.devices[0].kind, DeviceKind::Nmos);
}

#[test]
fn gate_splits_diffusion_into_distinct_source_and_drain() {
    // The channel makes source and drain distinct nets: the single diffusion
    // rectangle, split by the gate, is two separate lobes. Pure connectivity would
    // short them; device recognition keeps them apart, on a third net from the gate.
    let doc = doc_with(vec![
        rect(DIFF, 0, 0, 100, 40),
        rect(POLY, 40, -10, 60, 50),
        rect(NSDM, -5, -5, 105, 45),
    ]);
    let dnl = extract_devices(&doc, "top", &DeviceTech::sky130());
    let dev = &dnl.devices[0];

    assert!(dev.source_net.is_some(), "source bound to a net");
    assert!(dev.drain_net.is_some(), "drain bound to a net");
    assert!(dev.gate_net.is_some(), "gate bound to a net");
    assert_ne!(
        dev.source_net, dev.drain_net,
        "the gate separates source from drain"
    );
    assert_ne!(
        dev.gate_net, dev.source_net,
        "gate is not the source diffusion"
    );
    assert_ne!(
        dev.gate_net, dev.drain_net,
        "gate is not the drain diffusion"
    );

    // Channel geometry: L = poly width across the channel (20), W = diff height (40).
    assert_eq!(dev.length, 20, "channel length is the poly width");
    assert_eq!(dev.width, 40, "channel width is the diffusion height");
}

#[test]
fn inverter_extracts_one_nmos_and_one_pmos() {
    // The success bar: the inverter fixture extracts exactly one NMOS and one PMOS.
    let (doc, labels) = inverter();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);

    assert_eq!(dnl.devices.len(), 2, "an inverter is two transistors");
    assert_eq!(dnl.count_of(DeviceKind::Nmos), 1, "one NMOS");
    assert_eq!(dnl.count_of(DeviceKind::Pmos), 1, "one PMOS");
}

#[test]
fn inverter_terminal_nets_are_correct() {
    // The success bar: correct terminal nets. Both gates are the input A; both
    // drains are the output Y; each source and bulk is its own power rail.
    let (doc, labels) = inverter();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);

    let nmos = device_of(&dnl, DeviceKind::Nmos);
    assert_eq!(dnl.net_name(nmos.gate_net), Some("A"), "NMOS gate is A");
    assert_eq!(
        dnl.net_name(nmos.source_net),
        Some("VGND"),
        "NMOS source is VGND"
    );
    assert_eq!(dnl.net_name(nmos.drain_net), Some("Y"), "NMOS drain is Y");
    assert_eq!(
        dnl.net_name(nmos.bulk_net),
        Some("VGND"),
        "NMOS body ties to VGND"
    );

    let pmos = device_of(&dnl, DeviceKind::Pmos);
    assert_eq!(dnl.net_name(pmos.gate_net), Some("A"), "PMOS gate is A");
    assert_eq!(
        dnl.net_name(pmos.source_net),
        Some("VPWR"),
        "PMOS source is VPWR"
    );
    assert_eq!(dnl.net_name(pmos.drain_net), Some("Y"), "PMOS drain is Y");
    assert_eq!(
        dnl.net_name(pmos.bulk_net),
        Some("VPWR"),
        "PMOS body ties to VPWR"
    );

    // Structural invariants, independent of the names: shared input and output.
    assert_eq!(
        nmos.gate_net, pmos.gate_net,
        "the two gates are the same net"
    );
    assert_eq!(
        nmos.drain_net, pmos.drain_net,
        "the two drains are the same net"
    );
    assert_ne!(
        nmos.source_net, pmos.source_net,
        "the sources are separate rails"
    );
}

#[test]
fn poly_beside_diff_forms_no_device() {
    // The seeded-bad case: a poly that runs alongside the diffusion without
    // crossing it is not a transistor, so nothing is recognised.
    let doc = doc_with(vec![
        rect(DIFF, 0, 0, 100, 40),
        rect(POLY, 0, 60, 100, 80), // above the diff, never overlapping it
        rect(NSDM, -5, -5, 105, 45),
    ]);
    let dnl = extract_devices(&doc, "top", &DeviceTech::sky130());
    assert!(dnl.devices.is_empty(), "no gate, no device");
}

#[test]
fn poly_grazing_diff_edge_forms_no_device() {
    // A poly that clips the corner of the diffusion (overlaps but does not cross
    // the full width) leaves diffusion on only one side: not a channel.
    let doc = doc_with(vec![
        rect(DIFF, 0, 0, 100, 40),
        rect(POLY, 40, 30, 60, 90), // dips into the top of the diff, does not cross
        rect(NSDM, -5, -5, 105, 45),
    ]);
    let dnl = extract_devices(&doc, "top", &DeviceTech::sky130());
    assert!(
        dnl.devices.is_empty(),
        "a partial overlap is not a full-width channel"
    );
}

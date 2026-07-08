//! Device-level LVS-lite: compare a schematic's expected device netlist against
//! the extracted layout, by device kind and terminal-net connectivity.
//!
//! The comparison is name-based (a device matches when its kind, gate, unordered
//! source/drain, and bulk nets carry the same names), so it needs both sides to
//! name their nets. The layout side gets names from label geometry; the schematic
//! side is built with named nets here.

mod common;

use common::inverter;
use reticle_extract::device::{
    Device, DeviceKind, DeviceNetlist, DeviceTech, compare_devices, extract_devices_labeled,
};
use reticle_extract::{Net, Netlist};

/// The expected inverter schematic: named nets A/VGND/Y/VPWR and two transistors
/// wired the way the layout should be.
fn expected_inverter() -> DeviceNetlist {
    // Net indices: 0=A, 1=VGND, 2=Y, 3=VPWR.
    let nets = Netlist::new(vec![
        Net::new("A", vec![]),
        Net::new("VGND", vec![]),
        Net::new("Y", vec![]),
        Net::new("VPWR", vec![]),
    ]);
    let nmos = Device {
        kind: DeviceKind::Nmos,
        gate_net: Some(0),
        source_net: Some(1),
        drain_net: Some(2),
        bulk_net: Some(1),
        width: 40,
        length: 20,
    };
    let pmos = Device {
        kind: DeviceKind::Pmos,
        gate_net: Some(0),
        source_net: Some(3),
        drain_net: Some(2),
        bulk_net: Some(3),
        width: 40,
        length: 20,
    };
    DeviceNetlist {
        devices: vec![nmos, pmos],
        nets,
    }
}

#[test]
fn extraction_matches_the_expected_inverter() {
    let (doc, labels) = inverter();
    let extracted = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);
    let diff = compare_devices(&extracted, &expected_inverter());
    assert!(
        diff.is_empty(),
        "the extracted inverter matches the schematic: {diff:?}"
    );
}

#[test]
fn a_missing_device_is_reported() {
    let (doc, labels) = inverter();
    let extracted = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);

    // Expected has an extra (third) NMOS the layout does not: one missing device.
    let mut expected = expected_inverter();
    expected.devices.push(Device {
        kind: DeviceKind::Nmos,
        gate_net: Some(0),
        source_net: Some(1),
        drain_net: Some(2),
        bulk_net: Some(1),
        width: 40,
        length: 20,
    });

    let diff = compare_devices(&extracted, &expected);
    assert!(!diff.is_empty(), "the device counts disagree");
    assert_eq!(diff.missing.len(), 1, "one expected device is unmatched");
    assert_eq!(diff.extra.len(), 0);
    assert_eq!(diff.missing[0].kind, DeviceKind::Nmos);
}

#[test]
fn a_miswired_terminal_is_reported() {
    let (doc, labels) = inverter();
    let extracted = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);

    // Schematic where the NMOS drain is (wrongly) on VPWR instead of Y: the real
    // NMOS no longer matches, so it is extra (in layout) and the wrong one missing.
    let mut expected = expected_inverter();
    expected.devices[0].drain_net = Some(3); // VPWR

    let diff = compare_devices(&extracted, &expected);
    assert_eq!(
        diff.missing.len(),
        1,
        "the miswired schematic NMOS is unmatched"
    );
    assert_eq!(
        diff.extra.len(),
        1,
        "the correctly-wired layout NMOS is unmatched"
    );
    assert_eq!(diff.missing[0].kind, DeviceKind::Nmos);
    assert_eq!(diff.extra[0].kind, DeviceKind::Nmos);
}

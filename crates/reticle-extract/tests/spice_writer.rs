//! SPICE-writer cross-test: the netlist lane's producer contract with the
//! xschem lane's consumer.
//!
//! Both lanes build against the committed
//! `tests/fixtures/contracts/spice_exchange_inverter.{spice,json}` fixture
//! first (see the JSON's own `_comment` field), so this test asserts the
//! writer's structural output -- built from a real `extract_devices_labeled`
//! pass, not a hand-typed stand-in -- matches the fixture. It does not require
//! the xschem lane to have run in this session.

mod common;

use std::collections::HashSet;

use common::{DIFF, LI1, LICON1, NSDM, NWELL, POLY, PSDM, TAP, doc_with, label, rect};
use reticle_extract::NetLabel;
use reticle_extract::device::{DeviceTech, extract_devices_labeled};
use reticle_extract::spice::{SpiceTech, parse_spice, to_spice_subckt, write_spice};
use reticle_model::Document;
use serde_json::Value;

const CONTRACT_JSON: &str = include_str!("fixtures/contracts/spice_exchange_inverter.json");
const CONTRACT_SPICE: &str = include_str!("fixtures/contracts/spice_exchange_inverter.spice");
const DBU_PER_MICRON: i64 = 1000;
const SUBCKT_NAME: &str = "sky130_fd_sc_hd__inv_1";

/// A hand-built SKY130 inverter whose body ties are separate straps (`VNB` /
/// `VPB`), not shorted into the power rails like `common::inverter()` (the
/// smaller fixture the device-recognition unit tests use, where the tap ties
/// directly into the rail strap). This is the six-node structure the SPICE
/// exchange contract models: `VPB`, `VNB`, `VGND`, `VPWR`, `A`, `Y`, matching a
/// real standard-cell inverter's separate well/substrate taps.
///
/// W/L are chosen so the extracted channel geometry lands exactly on the
/// contract's decimal-micron values at 1000 DBU/micron: the NMOS channel is
/// 650 x 150 DBU (0.65 x 0.15 um), the PMOS channel 1000 x 150 DBU (1 x 0.15
/// um) -- see the inline comments below for which rectangle sets each.
fn inverter_with_separate_taps() -> (Document, Vec<NetLabel>) {
    let shapes = vec![
        // --- NMOS (bottom, n+ in substrate); diff height 650 = W, poly width 150 = L ---
        rect(DIFF, 0, 0, 400, 650),     // 0: NMOS active
        rect(NSDM, -5, -5, 405, 655),   // 1: n+ select over NMOS active
        rect(TAP, 0, -300, 400, -100),  // 2: p-tap, its own strap (VNB)
        rect(PSDM, -5, -305, 405, -95), // 3: p+ select over the p-tap
        // --- PMOS (top, p+ in n-well); diff height 1000 = W, poly width 150 = L ---
        rect(NWELL, -10, 950, 410, 2050), // 4: n-well
        rect(DIFF, 0, 1000, 400, 2000),   // 5: PMOS active
        rect(PSDM, -5, 995, 405, 2005),   // 6: p+ select over PMOS active
        rect(TAP, 0, 2300, 400, 2500),    // 7: n-tap, its own strap (VPB)
        rect(NSDM, -5, 2295, 405, 2505),  // 8: n+ select over the n-tap
        // --- Shared poly gate (input A), width 150 -> L=0.15um for both devices ---
        rect(POLY, 125, -50, 275, 2050), // 9: vertical gate stripe crossing both diffs
        // --- VGND: NMOS source lobe only (the tap is a separate strap below) ---
        rect(LICON1, 40, 275, 90, 375), // 10: contact on the NMOS source lobe
        rect(LI1, 20, 250, 110, 400),   // 11: li1 VGND strap
        // --- VNB: p-tap only (not tied to the source rail) ---
        rect(LICON1, 40, -230, 90, -170), // 12: contact on the p-tap
        rect(LI1, 20, -250, 110, -150),   // 13: li1 VNB strap
        // --- Y: NMOS drain lobe + PMOS drain lobe, tied by one strap ---
        rect(LICON1, 300, 275, 350, 375), // 14: contact on the NMOS drain lobe
        rect(LICON1, 300, 1275, 350, 1375), // 15: contact on the PMOS drain lobe
        rect(LI1, 290, 250, 360, 1400),   // 16: li1 Y strap spanning both
        // --- VPWR: PMOS source lobe only (the tap is a separate strap below) ---
        rect(LICON1, 40, 1275, 90, 1375), // 17: contact on the PMOS source lobe
        rect(LI1, 20, 1250, 110, 1400),   // 18: li1 VPWR strap
        // --- VPB: n-tap only (not tied to the source rail) ---
        rect(LICON1, 40, 2370, 90, 2430), // 19: contact on the n-tap
        rect(LI1, 20, 2350, 110, 2450),   // 20: li1 VPB strap
    ];
    let labels = vec![
        label("A", POLY, 200, 1000),
        label("VGND", LI1, 65, 325),
        label("VNB", LI1, 65, -200),
        label("Y", LI1, 325, 800),
        label("VPWR", LI1, 65, 1325),
        label("VPB", LI1, 65, 2400),
    ];
    (doc_with(shapes), labels)
}

/// The real `sky130_fd_sc_hd__inv_1` cell's models: its PMOS is the high-Vt
/// primitive (`_hvt`). `DeviceKind` carries no Vt-flavour information (see
/// `SpiceTech`'s doc comment in `src/spice.rs`), so this is supplied here as
/// the tech table, not derived from the extracted geometry.
fn sky130_inv_1_tech() -> SpiceTech {
    SpiceTech {
        nmos_model: "sky130_fd_pr__nfet_01v8".to_owned(),
        pmos_model: "sky130_fd_pr__pfet_01v8_hvt".to_owned(),
    }
}

#[test]
fn inverter_extracts_the_six_node_structure() {
    let (doc, labels) = inverter_with_separate_taps();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);
    assert_eq!(dnl.devices.len(), 2, "one NMOS + one PMOS");

    // Every terminal on every device resolves: unlike common::inverter(), the
    // taps here are their own nets (not shorted into a rail) and still bind,
    // which is the point of this fixture (it exercises VNB/VPB as distinct
    // ports, matching the exchange contract).
    for device in &dnl.devices {
        assert!(device.gate_net.is_some(), "gate should resolve");
        assert!(device.drain_net.is_some(), "drain should resolve");
        assert!(device.source_net.is_some(), "source should resolve");
        assert!(
            device.bulk_net.is_some(),
            "bulk should resolve to its own tap net"
        );
    }
}

#[test]
fn writer_matches_the_spice_exchange_contract_structure() {
    let (doc, labels) = inverter_with_separate_taps();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);
    let tech = sky130_inv_1_tech();
    let subckt = to_spice_subckt(&dnl, SUBCKT_NAME, DBU_PER_MICRON, &tech);

    let want: Value = serde_json::from_str(CONTRACT_JSON).expect("contract JSON parses");
    assert_eq!(subckt.name, want["subckt"].as_str().unwrap());

    // Ports: the set of distinct nets referenced by any device terminal. Order
    // is the netlist's own stable (lowest-member-index) order (see spice.rs's
    // `ports_of` doc comment) and need not match the contract's chosen order;
    // structure here is the set, not the sequence.
    let want_ports: HashSet<String> = want["ports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let got_ports: HashSet<String> = subckt.ports.iter().cloned().collect();
    assert_eq!(got_ports, want_ports, "ports are the same set of nets");

    // Devices: kind + terminals + model + W/L, matched by kind (one of each in
    // this fixture).
    let want_devices = want["devices"].as_array().unwrap();
    assert_eq!(dnl.devices.len(), subckt.devices.len());
    for (device, spice_device) in dnl.devices.iter().zip(&subckt.devices) {
        let want_device = want_devices
            .iter()
            .find(|d| d["kind"].as_str() == Some(device.kind.as_str()))
            .unwrap_or_else(|| panic!("contract has a {} device", device.kind.as_str()));

        let want_terminals = &want_device["terminals"];
        assert_eq!(
            spice_device.drain,
            want_terminals["drain"].as_str().unwrap()
        );
        assert_eq!(spice_device.gate, want_terminals["gate"].as_str().unwrap());
        assert_eq!(
            spice_device.source,
            want_terminals["source"].as_str().unwrap()
        );
        assert_eq!(spice_device.bulk, want_terminals["bulk"].as_str().unwrap());
        assert_eq!(spice_device.model, want_device["model"].as_str().unwrap());
        assert_eq!(spice_device.w, want_device["params"]["w"].as_str().unwrap());
        assert_eq!(spice_device.l, want_device["params"]["l"].as_str().unwrap());
    }
}

#[test]
fn emitted_spice_parses_back_to_the_same_structure() {
    let (doc, labels) = inverter_with_separate_taps();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);
    let tech = sky130_inv_1_tech();

    let expected = to_spice_subckt(&dnl, SUBCKT_NAME, DBU_PER_MICRON, &tech);
    let text = write_spice(&dnl, SUBCKT_NAME, DBU_PER_MICRON, &tech);
    let parsed = parse_spice(&text).expect("the writer's own output parses");
    assert_eq!(parsed, expected);
}

#[test]
fn committed_spice_fixture_parses_and_agrees_with_the_writer() {
    let (doc, labels) = inverter_with_separate_taps();
    let dnl = extract_devices_labeled(&doc, "top", &DeviceTech::sky130(), &labels);
    let tech = sky130_inv_1_tech();
    let built = to_spice_subckt(&dnl, SUBCKT_NAME, DBU_PER_MICRON, &tech);

    let parsed = parse_spice(CONTRACT_SPICE).expect("the committed contract fixture parses");
    assert_eq!(parsed.name, built.name);

    let want_ports: HashSet<String> = parsed.ports.iter().cloned().collect();
    let got_ports: HashSet<String> = built.ports.iter().cloned().collect();
    assert_eq!(got_ports, want_ports);

    assert_eq!(parsed.devices.len(), built.devices.len());
    for built_device in &built.devices {
        let matching = parsed
            .devices
            .iter()
            .find(|d| {
                d.drain == built_device.drain
                    && d.gate == built_device.gate
                    && d.source == built_device.source
                    && d.bulk == built_device.bulk
            })
            .unwrap_or_else(|| panic!("committed fixture has a device matching {built_device:?}"));
        assert_eq!(matching.model, built_device.model);
        assert_eq!(matching.w, built_device.w);
        assert_eq!(matching.l, built_device.l);
    }
}

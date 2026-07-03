//! Technology-file serializer round-trip tests.
//!
//! [`write_technology`](reticle_io::write_technology) is the inverse of
//! [`parse_technology`](reticle_io::parse_technology) over the format's semantic
//! content. These tests pin the two round-trip guarantees the editor relies on:
//!
//! * `parse(write(t)) == t` (a serialized technology reloads unchanged), and
//! * `write(parse(write(t))) == write(t)` (the serializer is an idempotent
//!   canonical form, so re-saving an unedited file is byte-stable).

use reticle_geometry::LayerId;
use reticle_io::{parse_technology, write_technology};
use reticle_model::{LayerInfo, Rule, RuleKind, StackEntry, Technology};

/// Builds a technology exercising every directive and both rule forms.
fn sample_technology() -> Technology {
    Technology {
        name: "demo_process".to_owned(),
        dbu_per_micron: 1000,
        layers: vec![
            LayerInfo {
                id: LayerId::new(1, 0),
                name: "metal1".to_owned(),
                color_rgba: 0x4488_FFFF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(2, 0),
                name: "via1".to_owned(),
                color_rgba: 0x8888_88FF,
                visible: true,
            },
        ],
        rules: vec![
            Rule {
                name: "width_1_0".to_owned(),
                kind: RuleKind::Width,
                layer: LayerId::new(1, 0),
                other_layer: None,
                value: 100,
            },
            Rule {
                name: "enclosure_2_0".to_owned(),
                kind: RuleKind::Enclosure,
                layer: LayerId::new(2, 0),
                other_layer: Some(LayerId::new(1, 0)),
                value: 20,
            },
        ],
        stack: vec![
            StackEntry {
                layer: LayerId::new(1, 0),
                z_bottom_nm: 500,
                thickness_nm: 200,
            },
            StackEntry {
                layer: LayerId::new(2, 0),
                z_bottom_nm: 700,
                thickness_nm: 150,
            },
        ],
    }
}

#[test]
fn write_then_parse_is_identity() {
    let tech = sample_technology();
    let text = write_technology(&tech);
    let reparsed = parse_technology(&text).expect("serialized technology parses");

    // The `name` set by `parse_rule` is derived, not stored in the file, so compare
    // the fields that the file format actually carries.
    assert_eq!(reparsed.name, tech.name);
    assert_eq!(reparsed.dbu_per_micron, tech.dbu_per_micron);
    assert_eq!(reparsed.layers, tech.layers);
    assert_eq!(reparsed.stack, tech.stack);
    assert_eq!(reparsed.rules.len(), tech.rules.len());
    for (got, want) in reparsed.rules.iter().zip(&tech.rules) {
        assert_eq!(got.kind, want.kind);
        assert_eq!(got.layer, want.layer);
        assert_eq!(got.other_layer, want.other_layer);
        assert_eq!(got.value, want.value);
    }
}

#[test]
fn serializer_is_idempotent_fixpoint() {
    let tech = sample_technology();
    let once = write_technology(&tech);
    let twice = write_technology(&parse_technology(&once).expect("first pass parses"));
    assert_eq!(
        once, twice,
        "re-serializing an unedited file is byte-stable"
    );
}

#[test]
fn colors_are_uppercase_eight_digits() {
    let tech = Technology {
        dbu_per_micron: 1,
        layers: vec![LayerInfo {
            id: LayerId::new(1, 0),
            name: "m1".to_owned(),
            color_rgba: 0x00AB_CDEF,
            visible: true,
        }],
        ..Technology::default()
    };
    let text = write_technology(&tech);
    assert!(
        text.contains("layer 1 0 m1 00ABCDEF"),
        "color must be eight uppercase hex digits, got:\n{text}"
    );
}

#[test]
fn empty_name_omits_technology_line() {
    let tech = Technology {
        dbu_per_micron: 1000,
        ..Technology::default()
    };
    let text = write_technology(&tech);
    assert!(
        !text.contains("technology "),
        "no name means no header line"
    );
    assert!(text.starts_with("dbu_per_micron 1000"));
}

#[test]
fn single_and_two_layer_rules_use_the_right_form() {
    let tech = Technology {
        dbu_per_micron: 1,
        rules: vec![
            Rule {
                name: "width".to_owned(),
                kind: RuleKind::Width,
                layer: LayerId::new(3, 0),
                other_layer: None,
                value: 55,
            },
            Rule {
                name: "spacing".to_owned(),
                kind: RuleKind::Spacing,
                layer: LayerId::new(3, 0),
                other_layer: Some(LayerId::new(4, 1)),
                value: 66,
            },
        ],
        ..Technology::default()
    };
    let text = write_technology(&tech);
    assert!(
        text.contains("rule width 3 0 55\n"),
        "single-layer form:\n{text}"
    );
    assert!(
        text.contains("rule spacing 3 0 4 1 66\n"),
        "two-layer form:\n{text}"
    );
}

#[test]
fn real_sky130_file_round_trips_to_a_fixpoint() {
    // The committed SKY130 technology file must survive parse -> serialize ->
    // parse with equal semantics, and serialize to a byte-stable canonical form.
    const SKY130: &str = include_str!("../../../tech/sky130.tech");
    let original = parse_technology(SKY130).expect("committed sky130.tech parses");

    let serialized = write_technology(&original);
    let reparsed = parse_technology(&serialized).expect("serialized sky130 parses");

    assert_eq!(reparsed.name, original.name);
    assert_eq!(reparsed.dbu_per_micron, original.dbu_per_micron);
    assert_eq!(reparsed.layers, original.layers);
    assert_eq!(reparsed.stack, original.stack);
    assert_eq!(reparsed.rules, original.rules);

    // And the canonical form is a fixpoint.
    assert_eq!(serialized, write_technology(&reparsed));
}

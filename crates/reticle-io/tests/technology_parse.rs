//! Technology-file parser tests: a well-formed file parses to the expected
//! [`Technology`], and malformed lines are rejected.

use reticle_geometry::LayerId;
use reticle_io::parse_technology;
use reticle_model::RuleKind;

/// A representative, well-formed technology file exercising every directive,
/// comments, inline comments, blank lines, and mixed-case keywords.
const SAMPLE: &str = "\
# Demo technology file.
technology demo_process

dbu_per_micron 1000

# Layers: <layer> <datatype> <name> <rgba_hex>
layer 1 0 metal1 4488FFFF
LAYER 2 0 via1   0x888888FF   # inline comment, 0x-prefixed color
layer 3 7 metal2 #AA00FFCC

# Rules.
rule width     1 0 100
rule spacing   1 0 140
rule enclosure 2 0 1 0 20
";

#[test]
fn parses_full_technology_file() {
    let tech = parse_technology(SAMPLE).expect("sample should parse");

    assert_eq!(tech.name, "demo_process");
    assert_eq!(tech.dbu_per_micron, 1000);

    // Three layers, in declaration order.
    assert_eq!(tech.layers.len(), 3);
    assert_eq!(tech.layers[0].id, LayerId::new(1, 0));
    assert_eq!(tech.layers[0].name, "metal1");
    assert_eq!(tech.layers[0].color_rgba, 0x4488_FFFF);
    assert!(tech.layers[0].visible);

    assert_eq!(tech.layers[1].id, LayerId::new(2, 0));
    assert_eq!(tech.layers[1].name, "via1");
    assert_eq!(tech.layers[1].color_rgba, 0x8888_88FF);

    assert_eq!(tech.layers[2].id, LayerId::new(3, 7));
    assert_eq!(tech.layers[2].name, "metal2");
    assert_eq!(tech.layers[2].color_rgba, 0xAA00_FFCC);

    // Three rules, in declaration order.
    assert_eq!(tech.rules.len(), 3);

    assert_eq!(tech.rules[0].kind, RuleKind::Width);
    assert_eq!(tech.rules[0].layer, LayerId::new(1, 0));
    assert_eq!(tech.rules[0].other_layer, None);
    assert_eq!(tech.rules[0].value, 100);

    assert_eq!(tech.rules[1].kind, RuleKind::Spacing);
    assert_eq!(tech.rules[1].value, 140);

    // Two-layer rule captures the second layer.
    assert_eq!(tech.rules[2].kind, RuleKind::Enclosure);
    assert_eq!(tech.rules[2].layer, LayerId::new(2, 0));
    assert_eq!(tech.rules[2].other_layer, Some(LayerId::new(1, 0)));
    assert_eq!(tech.rules[2].value, 20);
}

#[test]
fn empty_and_comment_only_input_is_valid() {
    let tech = parse_technology("# just a comment\n\n   \n").expect("comments-only parses");
    assert_eq!(tech.name, "");
    assert_eq!(tech.dbu_per_micron, 0);
    assert!(tech.layers.is_empty());
    assert!(tech.rules.is_empty());
}

#[test]
fn rejects_unknown_directive() {
    assert!(parse_technology("frobnicate 3\n").is_err());
}

#[test]
fn rejects_non_positive_resolution() {
    assert!(parse_technology("dbu_per_micron 0\n").is_err());
    assert!(parse_technology("dbu_per_micron -5\n").is_err());
    assert!(parse_technology("dbu_per_micron abc\n").is_err());
}

#[test]
fn rejects_malformed_layer() {
    // Too few tokens.
    assert!(parse_technology("layer 1 0 metal1\n").is_err());
    // Bad color length.
    assert!(parse_technology("layer 1 0 metal1 FF00\n").is_err());
    // Non-numeric layer number.
    assert!(parse_technology("layer x 0 metal1 FF0000FF\n").is_err());
}

#[test]
fn rejects_malformed_rule() {
    // Unknown kind.
    assert!(parse_technology("rule bogus 1 0 100\n").is_err());
    // Wrong token count (5 tokens is neither the 4- nor 6-token form).
    assert!(parse_technology("rule spacing 1 0 1 0\n").is_err());
    // Non-numeric value.
    assert!(parse_technology("rule width 1 0 wide\n").is_err());
}

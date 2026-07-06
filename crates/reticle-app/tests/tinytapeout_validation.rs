//! External validation of the "New Tiny Tapeout tile" template against Tiny Tapeout's
//! own published files.
//!
//! The unit tests in `crate::tinytapeout` prove the template is internally
//! consistent. This integration test goes further: it reads coordinates
//! *extracted from Tiny Tapeout's own published files* (committed under
//! `tests/fixtures/tinytapeout/`, each with a source URL in that directory's
//! `NOTICE.md`) and asserts the template matches them. The fixture numbers are the
//! independent oracle, so a drift in the template that still passed its own unit
//! tests would fail here against the real geometry.
//!
//! Two sources are used:
//!
//! * Tiny Tapeout's **canonical analog tile template** (`tt_analog_1x2.def` and
//!   `magic_init_project.tcl` from `tt-support-tools`): the die area, the six
//!   `ua[*]` met4 pin rectangles, and the power-strap geometry the template must
//!   reproduce exactly.
//! * A **real published GDS-mode submission**, `tt_um_analog_mux`: a cross-check
//!   that the template's footprint family (1x2, 225.76 um tall) and its met4-top /
//!   no-met5 rule agree with a design that actually taped out.

use std::collections::HashMap;
use std::path::Path;

use reticle_app::tinytapeout::{TT_TILE_TOP, tile_document};
use reticle_geometry::{LayerId, Rect, Shape};
use reticle_model::{Document, Pin};

/// SKY130 met4 pin purpose (`71/16`): the layer the template's pins are on.
const MET4_PIN: LayerId = LayerId::new(71, 16);

/// Parses a committed `key value` fixture file into a map. Blank lines and lines
/// whose first non-space character is `#` are ignored; each remaining line is a key
/// and a single-token value.
fn load_fixture(name: &str) -> HashMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tinytapeout")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            map.insert(k.to_owned(), v.to_owned());
        }
    }
    map
}

/// A required integer value from a fixture map.
fn int(map: &HashMap<String, String>, key: &str) -> i64 {
    map.get(key)
        .unwrap_or_else(|| panic!("fixture is missing key `{key}`"))
        .parse()
        .unwrap_or_else(|_| panic!("fixture key `{key}` is not an integer"))
}

/// The template's boundary rectangle (its single `tt_boundary`, `81/4`, shape).
fn template_die_area(doc: &Document) -> Rect {
    let cell = doc.cell(TT_TILE_TOP).expect("tile top cell");
    cell.shapes
        .iter()
        .find(|s| s.layer == LayerId::new(81, 4))
        .expect("the template has a boundary shape")
        .bounding_box()
}

/// The template's `ua[n]` pin, by index.
fn template_ua(doc: &Document, n: usize) -> &Pin {
    let cell = doc.cell(TT_TILE_TOP).unwrap();
    cell.pins
        .iter()
        .find(|p| p.name == format!("ua[{n}]"))
        .unwrap_or_else(|| panic!("template has ua[{n}]"))
}

/// The template's power strap pin, by net name.
fn template_strap<'a>(doc: &'a Document, name: &'_ str) -> &'a Pin {
    let cell = doc.cell(TT_TILE_TOP).unwrap();
    cell.pins
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("template has the {name} strap"))
}

#[test]
fn die_area_matches_the_canonical_def_template_exactly() {
    let f = load_fixture("analog_1x2_template.txt");
    let die = template_die_area(&tile_document());
    // Exact match to tt_analog_1x2.def DIEAREA (zero tolerance; these are the same
    // integers Tiny Tapeout published).
    assert_eq!(i64::from(die.min.x), int(&f, "die_min_x"));
    assert_eq!(i64::from(die.min.y), int(&f, "die_min_y"));
    assert_eq!(i64::from(die.max.x), int(&f, "die_max_x"));
    assert_eq!(i64::from(die.max.y), int(&f, "die_max_y"));
}

#[test]
fn six_ua_pins_match_the_canonical_def_rectangles_exactly() {
    let f = load_fixture("analog_1x2_template.txt");
    let doc = tile_document();

    let half_w = int(&f, "ua_port_half_w");
    let placed_y = int(&f, "ua_placed_y");
    let half_h = int(&f, "ua_port_half_h");
    // Reconstruct each expected absolute rectangle from the DEF PORT + PLACED, the
    // same way Tiny Tapeout's tools place the port, and compare to the template pin.
    for n in 0..6usize {
        let cx = int(&f, &format!("ua{n}_x"));
        let expected = Rect::new(
            reticle_geometry::Point::new(
                i32::try_from(cx - half_w).unwrap(),
                i32::try_from(placed_y - half_h).unwrap(),
            ),
            reticle_geometry::Point::new(
                i32::try_from(cx + half_w).unwrap(),
                i32::try_from(placed_y + half_h).unwrap(),
            ),
        );
        let pin = template_ua(&doc, n);
        assert_eq!(
            pin.region, expected,
            "ua[{n}] rectangle must match the DEF PORT+PLACED"
        );
        assert_eq!(pin.layer, MET4_PIN, "ua[{n}] must be on met4");
        // The layer name in the fixture is met4; our LayerId 71/16 is met4's pin
        // purpose, so assert the fixture agrees on the metal.
        assert_eq!(f.get("ua_layer").map(String::as_str), Some("met4"));
    }
}

#[test]
fn power_straps_match_the_init_script_geometry_exactly() {
    let f = load_fixture("analog_power_straps.txt");
    let doc = tile_document();

    let bottom = int(&f, "strap_bottom_y");
    let top = int(&f, "strap_top_y");
    let width = int(&f, "strap_width");
    let min_width = int(&f, "strap_min_width");
    let half_w = width / 2;

    for (net, key) in [
        ("VDPWR", "vdpwr_x"),
        ("VGND", "vgnd_x"),
        ("VAPWR", "vapwr_x"),
    ] {
        let cx = int(&f, key);
        let expected = Rect::new(
            reticle_geometry::Point::new(
                i32::try_from(cx - half_w).unwrap(),
                i32::try_from(bottom).unwrap(),
            ),
            reticle_geometry::Point::new(
                i32::try_from(cx + half_w).unwrap(),
                i32::try_from(top).unwrap(),
            ),
        );
        let pin = template_strap(&doc, net);
        assert_eq!(
            pin.region, expected,
            "{net} strap must match the init script"
        );
        assert_eq!(pin.layer, MET4_PIN, "{net} strap must be on met4");
        // And it clears Tiny Tapeout's stated minimum width.
        assert!(
            pin.region.width() >= min_width,
            "{net} width {} is under the fixture's stated minimum {min_width}",
            pin.region.width()
        );
    }
    assert_eq!(f.get("strap_layer").map(String::as_str), Some("met4"));
}

#[test]
fn footprint_agrees_with_the_published_analog_mux_submission() {
    // Cross-check against a real taped-out GDS-mode design. Its die is a touch wider
    // than the bare template (it is a full mux with its own floorplan), but it is
    // the same 1x2 family: same height, and a width within Tiny Tapeout's stated
    // "about 160 um, 3.3V slightly narrower" band. The template must sit in that
    // band and share the exact height.
    let f = load_fixture("published_example_tt_um_analog_mux.txt");
    let die = template_die_area(&tile_document());

    // Same height as the real submission (zero tolerance: 225.76 um is fixed for the
    // 1x2 row).
    assert_eq!(
        i64::from(die.max.y),
        int(&f, "config_die_max_y"),
        "the 1x2 tile height must match the published submission"
    );
    assert_eq!(int(&f, "def_die_max_y"), int(&f, "config_die_max_y"));

    // Width within a 10 um tolerance of the published example (the example is 168.36
    // um; the bare template is 161 um; both are legitimate 1x2 widths).
    let tol = 10_000_i64;
    let example_w = int(&f, "config_die_max_x");
    assert!(
        (i64::from(die.max.x) - example_w).abs() <= tol,
        "template width {} should be within {tol} of the published {example_w}",
        die.max.x
    );

    // The published design forbids met5 and tops out at met4; assert the fixture
    // captured that and that our template draws nothing above met4.
    assert_eq!(f.get("top_routing_layer").map(String::as_str), Some("met4"));
    assert_eq!(f.get("met5_forbidden").map(String::as_str), Some("true"));
    let cell = tile_document().cell(TT_TILE_TOP).unwrap().clone();
    assert!(
        !cell.shapes.iter().any(|s| s.layer == LayerId::new(72, 20)),
        "the template must draw nothing on the forbidden met5 layer"
    );
}

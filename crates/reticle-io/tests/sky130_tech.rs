//! Guards the committed SKY130 technology file: it must always parse, and carry
//! the digital metal stack layers, pin and label purposes, and physical stack
//! data with the documented values.

use reticle_geometry::LayerId;
use reticle_io::parse_technology;

const SKY130: &str = include_str!("../../../tech/sky130.tech");

#[test]
fn sky130_technology_parses() {
    let tech = parse_technology(SKY130).expect("sky130.tech should parse");
    assert_eq!(tech.name, "sky130");
    assert_eq!(
        tech.dbu_per_micron, 1000,
        "1 dbu = 1 nm, so 1000 dbu per micron"
    );

    // Drawing, pin, and label layers for the digital stack are present.
    let names: Vec<&str> = tech.layers.iter().map(|l| l.name.as_str()).collect();
    for expected in [
        "nwell",
        "diff",
        "poly",
        "li1",
        "met1",
        "met5",
        "li1_pin",
        "met1_label",
    ] {
        assert!(names.contains(&expected), "missing layer `{expected}`");
    }

    // Key GDS layer/datatype numbers match the SkyWater layer map.
    let li1 = tech
        .layers
        .iter()
        .find(|l| l.name == "li1")
        .expect("li1 layer");
    assert_eq!(li1.id, LayerId::new(67, 20));
    let met1 = tech
        .layers
        .iter()
        .find(|l| l.name == "met1")
        .expect("met1 layer");
    assert_eq!(met1.id, LayerId::new(68, 20));
}

#[test]
fn sky130_stack_has_documented_thicknesses() {
    let tech = parse_technology(SKY130).expect("sky130.tech should parse");

    // met5 sits at z = 5371 nm and is 1260 nm thick (official stack diagram).
    let met5 = tech
        .stack_for(LayerId::new(72, 20))
        .expect("met5 stack entry");
    assert_eq!(met5.z_bottom_nm, 5371);
    assert_eq!(met5.thickness_nm, 1260);
    assert_eq!(met5.z_top_nm(), 6631);

    // poly is 180 nm thick, li1 is 100 nm thick.
    assert_eq!(
        tech.stack_for(LayerId::new(66, 20)).unwrap().thickness_nm,
        180
    );
    assert_eq!(
        tech.stack_for(LayerId::new(67, 20)).unwrap().thickness_nm,
        100
    );

    // The conductor ladder (poly and above) is ordered bottom-to-top without
    // overlap. Well and active layers (nwell, diff, tap) are coplanar substrate
    // features that legitimately share z, so they are excluded here.
    let mut conductors: Vec<_> = tech.stack.iter().filter(|e| e.z_bottom_nm >= 300).collect();
    conductors.sort_by_key(|e| e.z_bottom_nm);
    for pair in conductors.windows(2) {
        assert!(
            pair[0].z_top_nm() <= pair[1].z_bottom_nm,
            "stack slabs must not overlap: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
}

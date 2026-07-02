//! Property tests for the in-house OASIS-inspired subset ([`Oasis`]).
//!
//! Generates random documents containing rectangles, polygons, paths, text
//! labels, single placements (instances), and arrays, then asserts that
//! `import(export(doc))` reproduces the document's cells exactly. This
//! complements the fixed-example round-trip tests in `oasis_roundtrip.rs`.
//!
//! # Magnification and the reader's canonical form
//!
//! The container writes a placement magnification as an `f64` bit pattern and the
//! reader reconstructs it as the rational `round(mag * 1_000_000) / 1_000_000`
//! (unit magnification stays exact). So an arbitrary [`Magnification`] is not a
//! fixed point of the round-trip, only values already in that canonical form are.
//! The generator therefore emits exactly those: unity, or `n / 1_000_000` for an
//! `n` that is not itself unity, which the reader returns byte-for-byte.

use proptest::prelude::*;
use reticle_geometry::Transform;
use reticle_geometry::{Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect};
use reticle_io::Oasis;
use reticle_model::{
    Anchor, ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Label,
    ShapeKind,
};

/// Coordinates are kept well inside `i32` so array/path arithmetic never saturates
/// in a way the equality check would notice; the encoding itself is full-range.
const COORD: std::ops::RangeInclusive<i32> = -100_000..=100_000;

/// A strategy for a single on-grid point.
fn point() -> impl Strategy<Value = Point> {
    (COORD, COORD).prop_map(|(x, y)| Point::new(x, y))
}

/// A strategy for a `(layer, datatype)` identifier.
fn layer_id() -> impl Strategy<Value = LayerId> {
    (any::<u16>(), any::<u16>()).prop_map(|(l, d)| LayerId::new(l, d))
}

/// A strategy for an end cap, covering every variant including a custom extension.
fn endcap() -> impl Strategy<Value = Endcap> {
    prop_oneof![
        Just(Endcap::Flat),
        Just(Endcap::Square),
        Just(Endcap::Round),
        (0..=1_000_i32).prop_map(Endcap::Custom),
    ]
}

/// A strategy for an [`Orientation`], covering all eight dihedral elements.
fn orientation() -> impl Strategy<Value = Orientation> {
    prop_oneof![
        Just(Orientation::R0),
        Just(Orientation::R90),
        Just(Orientation::R180),
        Just(Orientation::R270),
        Just(Orientation::MirrorX),
        Just(Orientation::MirrorX90),
        Just(Orientation::MirrorX180),
        Just(Orientation::MirrorX270),
    ]
}

/// A strategy for a [`Magnification`] that is a fixed point of the wire encoding
/// (see the module docs): unity, or an exact `n / 1_000_000`.
fn magnification() -> impl Strategy<Value = Magnification> {
    prop_oneof![
        Just(Magnification::UNITY),
        (1u32..=8_000_000)
            .prop_filter("unity is generated separately", |n| *n != 1_000_000)
            .prop_map(|n| Magnification::new(n, 1_000_000).expect("den is non-zero")),
    ]
}

/// A strategy for a full placement [`Transform`].
fn transform() -> impl Strategy<Value = Transform> {
    (point(), orientation(), magnification()).prop_map(
        |(translation, orientation, magnification)| Transform {
            translation,
            orientation,
            magnification,
        },
    )
}

/// A strategy for a drawable shape (rectangle, polygon, or path).
fn shape() -> impl Strategy<Value = DrawShape> {
    let rect = (layer_id(), point(), point())
        .prop_map(|(layer, a, b)| DrawShape::new(layer, ShapeKind::Rect(Rect::new(a, b))));
    let polygon = (layer_id(), prop::collection::vec(point(), 0..8))
        .prop_map(|(layer, v)| DrawShape::new(layer, ShapeKind::Polygon(Polygon::new(v))));
    let path = (
        layer_id(),
        prop::collection::vec(point(), 0..8),
        0..=10_000_i32,
        endcap(),
    )
        .prop_map(|(layer, pts, width, cap)| {
            DrawShape::new(layer, ShapeKind::Path(Path::new(pts, width, cap)))
        });
    prop_oneof![rect, polygon, path]
}

/// A strategy for an [`Anchor`], covering all five variants.
fn anchor() -> impl Strategy<Value = Anchor> {
    prop_oneof![
        Just(Anchor::Center),
        Just(Anchor::SouthWest),
        Just(Anchor::SouthEast),
        Just(Anchor::NorthWest),
        Just(Anchor::NorthEast),
    ]
}

/// A strategy for a text label: arbitrary (possibly empty) text on any layer,
/// at any position, with any anchor.
fn label() -> impl Strategy<Value = Label> {
    ("[a-zA-Z0-9_/\\[\\]]{0,16}", point(), layer_id(), anchor()).prop_map(
        |(text, position, layer, anchor)| Label {
            text,
            position,
            layer,
            anchor,
        },
    )
}

/// A strategy for a single placement of a child cell.
fn instance() -> impl Strategy<Value = Instance> {
    ("[a-z]{1,8}", transform()).prop_map(|(cell, transform)| Instance { cell, transform })
}

/// A strategy for an array placement of a child cell.
fn array() -> impl Strategy<Value = ArrayInstance> {
    ("[a-z]{1,8}", transform(), 0u32..64, 0u32..64, COORD, COORD).prop_map(
        |(cell, transform, columns, rows, column_pitch, row_pitch)| ArrayInstance {
            cell,
            transform,
            columns,
            rows,
            column_pitch,
            row_pitch,
        },
    )
}

/// A strategy for a whole cell: a name plus random shapes, labels, instances,
/// and arrays.
fn cell() -> impl Strategy<Value = Cell> {
    (
        "[a-z][a-z0-9_]{0,10}",
        prop::collection::vec(shape(), 0..6),
        prop::collection::vec(instance(), 0..4),
        prop::collection::vec(array(), 0..4),
        prop::collection::vec(label(), 0..4),
    )
        .prop_map(|(name, shapes, instances, arrays, labels)| Cell {
            name,
            shapes,
            instances,
            arrays,
            labels,
            ..Cell::default()
        })
}

/// A strategy for a document of uniquely-named cells with a resolution.
fn document() -> impl Strategy<Value = Document> {
    (prop::collection::vec(cell(), 1..5), 1i64..1_000_000).prop_map(|(cells, dbu)| {
        let mut doc = Document::new();
        for (i, mut c) in cells.into_iter().enumerate() {
            // Disambiguate names so no cell is silently overwritten on insert.
            c.name = format!("{}_{i}", c.name);
            doc.insert_cell(c);
        }
        let mut tech = reticle_model::Technology {
            dbu_per_micron: dbu,
            ..reticle_model::Technology::default()
        };
        // The importer rebuilds the layer table from geometry, so seed nothing.
        tech.layers.clear();
        doc.set_technology(tech);
        doc
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Export then import reproduces every cell (shapes, labels, instances,
    /// arrays) exactly.
    #[test]
    fn oasis_roundtrips_random_documents(doc in document()) {
        let bytes = Oasis.export(&doc).expect("export should succeed");
        let imported = Oasis.import(&bytes).expect("import should succeed");

        prop_assert_eq!(imported.cell_count(), doc.cell_count());
        for original in doc.cells() {
            let round = imported
                .cell(&original.name)
                .expect("every cell should survive the round-trip");
            prop_assert_eq!(&round.shapes, &original.shapes);
            prop_assert_eq!(&round.labels, &original.labels);
            prop_assert_eq!(&round.instances, &original.instances);
            prop_assert_eq!(&round.arrays, &original.arrays);
        }
    }

    /// A second export of the re-imported document is byte-identical (idempotent).
    #[test]
    fn oasis_export_is_idempotent(doc in document()) {
        let bytes1 = Oasis.export(&doc).expect("first export should succeed");
        let reimported = Oasis.import(&bytes1).expect("import should succeed");
        let bytes2 = Oasis.export(&reimported).expect("second export should succeed");
        prop_assert_eq!(bytes1, bytes2);
    }
}

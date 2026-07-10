//! Reader tests for the conformant-OASIS [`OasisStd`]: a write-then-read round trip over
//! the writer's own record subset, plus malformed-input hardening (no panics, clean
//! errors) and a couple of hand-built streams for records the writer never emits
//! (a type-17 placement and a single-integer octangular g-delta).

use reticle_geometry::{
    Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_io::OasisStd;
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Label, ShapeKind,
    Technology,
};

/// A document exercising every record the writer emits: rectangles (including negative
/// coordinates), a polygon, three path cap styles, two labels, and four placements
/// spanning orientation and magnification, plus a child cell.
fn sample_document() -> Document {
    let mut doc = Document::new();
    doc.set_technology(Technology {
        dbu_per_micron: 1000,
        ..Technology::default()
    });

    let mut sub = Cell::new("SUB");
    sub.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
    ));
    doc.insert_cell(sub);

    let mut top = Cell::new("TOP");
    top.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
    ));
    top.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(-50, -30), Point::new(10, 5))),
    ));
    top.shapes.push(DrawShape::new(
        LayerId::new(2, 5),
        ShapeKind::Polygon(Polygon::new(vec![
            Point::new(0, 0),
            Point::new(20, 0),
            Point::new(20, 10),
            Point::new(0, 10),
        ])),
    ));
    top.shapes.push(DrawShape::new(
        LayerId::new(3, 0),
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(100, 0), Point::new(100, 50)],
            20,
            Endcap::Flat,
        )),
    ));
    top.shapes.push(DrawShape::new(
        LayerId::new(3, 1),
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(40, 0)],
            20,
            Endcap::Square,
        )),
    ));
    top.shapes.push(DrawShape::new(
        LayerId::new(3, 2),
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(30, 0)],
            20,
            Endcap::Custom(7),
        )),
    ));
    top.labels
        .push(Label::new("PIN", Point::new(5, 5), LayerId::new(10, 0)));
    top.labels
        .push(Label::new("VDD", Point::new(-7, 12), LayerId::new(11, 3)));
    top.instances.push(Instance {
        cell: "SUB".into(),
        transform: Transform::IDENTITY,
    });
    top.instances.push(Instance {
        cell: "SUB".into(),
        transform: Transform {
            translation: Point::new(100, -50),
            orientation: Orientation::R90,
            magnification: Magnification::UNITY,
        },
    });
    top.instances.push(Instance {
        cell: "SUB".into(),
        transform: Transform {
            translation: Point::new(-30, 100),
            orientation: Orientation::MirrorX180,
            magnification: Magnification::new(2, 1).unwrap(),
        },
    });
    top.instances.push(Instance {
        cell: "SUB".into(),
        transform: Transform {
            translation: Point::new(0, 0),
            orientation: Orientation::MirrorX,
            magnification: Magnification::new(1, 2).unwrap(),
        },
    });
    doc.insert_cell(top);

    doc
}

#[test]
fn oasis_std_writer_subset_round_trips_to_equal_document() {
    let doc = sample_document();
    let bytes = OasisStd.export(&doc).expect("export");
    let imported = OasisStd.import(&bytes).expect("import");
    assert_eq!(
        imported, doc,
        "a write-then-read round trip preserves the document"
    );
}

#[test]
fn oasis_std_arrays_round_trip_as_expanded_placements() {
    let mut doc = Document::new();
    doc.set_technology(Technology {
        dbu_per_micron: 1000,
        ..Technology::default()
    });
    let mut sub = Cell::new("SUB");
    sub.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(5, 5))),
    ));
    doc.insert_cell(sub);
    let mut arr = Cell::new("ARR");
    arr.arrays.push(ArrayInstance {
        cell: "SUB".into(),
        transform: Transform::IDENTITY,
        columns: 2,
        rows: 3,
        column_pitch: 100,
        row_pitch: 200,
    });
    doc.insert_cell(arr);

    let bytes = OasisStd.export(&doc).expect("export");
    let imported = OasisStd.import(&bytes).expect("import");

    let arr = imported.cell("ARR").expect("ARR cell");
    assert!(
        arr.arrays.is_empty(),
        "arrays are expanded to placements on the wire"
    );
    assert_eq!(
        arr.instances.len(),
        6,
        "2 columns x 3 rows expands to 6 placements"
    );
    let mut translations: Vec<(i32, i32)> = arr
        .instances
        .iter()
        .map(|i| (i.transform.translation.x, i.transform.translation.y))
        .collect();
    translations.sort_unstable();
    let mut expected = vec![(0, 0), (100, 0), (0, 200), (100, 200), (0, 400), (100, 400)];
    expected.sort_unstable();
    assert_eq!(
        translations, expected,
        "placement grid matches the array pitch"
    );
    assert!(arr.instances.iter().all(|i| i.cell == "SUB"));
}

/// A tiny OASIS byte-stream builder for hand-crafting records the writer never emits and
/// deliberately malformed inputs. It carries only the primitives the reader consumes.
struct Oas {
    bytes: Vec<u8>,
}

impl Oas {
    fn new() -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"%SEMI-OASIS\r\n");
        Self { bytes }
    }

    fn byte(&mut self, b: u8) {
        self.bytes.push(b);
    }

    fn uint(&mut self, mut n: u64) {
        loop {
            let mut b = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                b |= 0x80;
            }
            self.bytes.push(b);
            if n == 0 {
                break;
            }
        }
    }

    fn sint(&mut self, n: i64) {
        let sign = u64::from(n < 0);
        self.uint((n.unsigned_abs() << 1) | sign);
    }

    fn string(&mut self, s: &str) {
        self.uint(s.len() as u64);
        self.bytes.extend_from_slice(s.as_bytes());
    }

    /// Emits the magic-following `START` record with a 1000 DBU/micron unit and zeroed
    /// inline table offsets, exactly as the reader expects to open a stream.
    fn start(&mut self) {
        self.byte(1); // START
        self.string("1.0");
        self.uint(0); // real type 0 (positive whole)
        self.uint(1000); // unit value
        self.uint(0); // offset flag 0: table offsets inline
        for _ in 0..12 {
            self.uint(0);
        }
    }

    fn end(&mut self) {
        self.byte(2); // the reader stops at END; the 256-byte padding is not required
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[test]
fn oasis_std_bad_magic_is_rejected() {
    assert!(OasisStd.import(&[0u8; 64]).is_err());
}

#[test]
fn oasis_std_empty_input_is_rejected() {
    assert!(OasisStd.import(&[]).is_err());
}

#[test]
fn oasis_std_unknown_record_id_errors() {
    let mut o = Oas::new();
    o.start();
    o.byte(99); // not a record id the reader knows
    assert!(OasisStd.import(&o.finish()).is_err());
}

#[test]
fn oasis_std_oversized_point_list_count_errors_without_panic() {
    let mut o = Oas::new();
    o.start();
    o.byte(3);
    o.string("P"); // CELLNAME "P" -> index 0
    o.byte(13);
    o.uint(0); // CELL 0
    o.byte(21); // POLYGON
    o.byte(0b0011_1011); // info 00PXYRDL
    o.uint(7); // layer
    o.uint(0); // datatype
    o.uint(4); // point-list type 4
    o.uint(10_000_000); // absurd vertex count, far past the cap and the input
    assert!(OasisStd.import(&o.finish()).is_err());
}

#[test]
fn oasis_std_truncation_at_every_prefix_never_panics() {
    let bytes = OasisStd.export(&sample_document()).expect("export");
    for len in 0..=bytes.len() {
        // Every prefix must return Ok or Err, never panic or hang.
        let _ = OasisStd.import(&bytes[..len]);
    }
}

#[test]
fn oasis_std_arbitrary_bytes_never_panic() {
    // A cheap seeded LCG; this needs coverage breadth, not statistical quality.
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    for _ in 0..3000 {
        let len = (next() % 512) as usize;
        let mut bytes = Vec::with_capacity(len + 13);
        // Half the time, prefix a valid magic + START so the fuzz reaches deeper records.
        if next() & 1 == 0 {
            bytes.extend_from_slice(b"%SEMI-OASIS\r\n");
            bytes.push(1); // a START record id
        }
        for _ in 0..len {
            bytes.push((next() & 0xff) as u8);
        }
        let _ = OasisStd.import(&bytes);
    }
}

#[test]
fn oasis_std_placement_type_17_reads_angle_and_cell() {
    let mut o = Oas::new();
    o.start();
    o.byte(3);
    o.string("SUB"); // index 0
    o.byte(3);
    o.string("TOP"); // index 1
    o.byte(13);
    o.uint(0); // define SUB
    o.byte(20); // a rectangle so SUB is non-empty
    o.byte(0b0111_1011);
    o.uint(1);
    o.uint(0);
    o.uint(10);
    o.uint(10);
    o.sint(0);
    o.sint(0);
    o.byte(13);
    o.uint(1); // define TOP
    o.byte(17); // PLACEMENT type 17
    o.byte(0b1111_0010); // C1 N1 X1 Y1 R0 AA=01 (90 deg) F0
    o.uint(0); // cell reference 0 -> SUB
    o.sint(40); // x
    o.sint(-25); // y
    o.end();

    let imported = OasisStd.import(&o.finish()).expect("import");
    let top = imported.cell("TOP").expect("TOP");
    assert_eq!(top.instances.len(), 1);
    let inst = &top.instances[0];
    assert_eq!(inst.cell, "SUB");
    assert_eq!(inst.transform.orientation, Orientation::R90);
    assert_eq!(inst.transform.translation, Point::new(40, -25));
    assert_eq!(inst.transform.magnification, Magnification::UNITY);
}

#[test]
fn oasis_std_single_integer_octangular_g_delta_decodes() {
    let mut o = Oas::new();
    o.start();
    o.byte(3);
    o.string("P"); // index 0
    o.byte(13);
    o.uint(0); // define P
    o.byte(21); // POLYGON
    o.byte(0b0011_1011); // info 00PXYRDL
    o.uint(7); // layer 7
    o.uint(0); // datatype 0
    o.uint(4); // point-list type 4
    o.uint(2); // two deltas
    o.uint(30 << 4); // octangular east, magnitude 30 -> (30, 0)
    o.uint((40 << 4) | (1 << 1)); // octangular north, magnitude 40 -> (0, 40)
    o.sint(0); // first vertex x
    o.sint(0); // first vertex y
    o.end();

    let imported = OasisStd.import(&o.finish()).expect("import");
    let cell = imported.cell("P").expect("P");
    let shape = &cell.shapes[0];
    match &shape.kind {
        ShapeKind::Polygon(p) => assert_eq!(
            p.vertices(),
            &[Point::new(0, 0), Point::new(30, 0), Point::new(30, 40)]
        ),
        other => panic!("expected a polygon, got {other:?}"),
    }
    assert_eq!(shape.layer, LayerId::new(7, 0));
}

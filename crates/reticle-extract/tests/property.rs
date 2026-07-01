//! Property test: the [`Extractor`]'s connected components must equal an
//! independent naive `O(n²)` union-find oracle over random single-layer rectangle
//! sets.
//!
//! Both sides use the *same* touch rule, closed-box intersection via
//! [`reticle_extract::rects_touch`] (edge- and corner-adjacent rectangles count as
//! connected). The oracle is deliberately the obvious "connect every touching
//! pair, then take components" reference, with a private, self-contained
//! union-find so it shares no code path with the crate's engine. Partitions are
//! compared as a canonical set-of-sets, independent of net names and order.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use reticle_extract::{Extractor, rects_touch};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};

const LAYER: LayerId = LayerId::new(1, 0);
/// Small coordinate bound so random rectangles meaningfully touch and overlap,
/// producing non-trivial multi-shape nets to compare.
const BOUND: i32 = 40;

/// A private, self-contained union-find for the oracle (no dependency on the
/// crate's `DisjointSet`, so the two implementations are genuinely independent).
struct OracleUf {
    parent: Vec<usize>,
}

impl OracleUf {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path halving
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// Naive oracle: connect every touching pair with a double loop, then return the
/// canonical partition (set of member-index sets).
fn oracle_partition(rects: &[Rect]) -> BTreeSet<BTreeSet<usize>> {
    let mut uf = OracleUf::new(rects.len());
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            if rects_touch(&rects[i], &rects[j]) {
                uf.union(i, j);
            }
        }
    }
    canonicalize((0..rects.len()).map(|i| (i, uf.find(i))))
}

/// The extractor's partition over the same rectangles, keyed by member index.
fn extractor_partition(rects: &[Rect]) -> BTreeSet<BTreeSet<usize>> {
    let mut cell = Cell::new("top");
    cell.shapes = rects
        .iter()
        .map(|r| DrawShape::new(LAYER, ShapeKind::Rect(*r)))
        .collect();
    let mut doc = Document::new();
    doc.insert_cell(cell);

    let netlist = Extractor::new().extract(&doc, "top");
    canonicalize(
        netlist
            .nets
            .iter()
            .enumerate()
            .flat_map(|(net_id, net)| net.shapes.iter().map(move |&s| (s, net_id))),
    )
}

/// Groups `(member, group_key)` pairs into a canonical set-of-sets of members.
fn canonicalize(pairs: impl IntoIterator<Item = (usize, usize)>) -> BTreeSet<BTreeSet<usize>> {
    let mut groups: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (member, key) in pairs {
        groups.entry(key).or_default().insert(member);
    }
    groups.into_values().collect()
}

/// A rectangle with positive area in `[-BOUND, BOUND]`.
fn rect_strategy() -> impl Strategy<Value = Rect> {
    (-BOUND..BOUND, -BOUND..BOUND, 1..=20i32, 1..=20i32)
        .prop_map(|(x, y, w, h)| Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Extractor components == naive oracle components for every random layout.
    #[test]
    fn extractor_components_match_naive_oracle(
        rects in prop::collection::vec(rect_strategy(), 0..30),
    ) {
        let expected = oracle_partition(&rects);
        let got = extractor_partition(&rects);
        prop_assert_eq!(got, expected);
    }

    /// The netlist accounts for every shape exactly once (a partition), and the
    /// reported `shape_count` matches the membership.
    #[test]
    fn netlist_is_a_partition(
        rects in prop::collection::vec(rect_strategy(), 0..30),
    ) {
        let mut cell = Cell::new("top");
        cell.shapes = rects
            .iter()
            .map(|r| DrawShape::new(LAYER, ShapeKind::Rect(*r)))
            .collect();
        let mut doc = Document::new();
        doc.insert_cell(cell);
        let netlist = Extractor::new().extract(&doc, "top");

        let mut seen = BTreeSet::new();
        for net in &netlist.nets {
            prop_assert_eq!(net.shape_count, net.shapes.len());
            for &s in &net.shapes {
                prop_assert!(seen.insert(s), "shape {} appears on two nets", s);
            }
        }
        prop_assert_eq!(seen.len(), rects.len(), "every shape is on exactly one net");
    }
}

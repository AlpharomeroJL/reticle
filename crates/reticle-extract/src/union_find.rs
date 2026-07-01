//! A small disjoint-set (union-find) with union-by-rank and path compression.
//!
//! Extraction groups connected shapes into nets, which is exactly the
//! connected-components problem: start with every shape in its own set and
//! [`union`](DisjointSet::union) the two sets whenever two shapes are found to be
//! electrically connected. The near-constant-time `find`/`union` keep the overall
//! pass close to linear in the number of connectivity edges.
//!
//! This is hand-rolled on purpose: the crate takes no runtime dependency beyond
//! geometry, the model, and the spatial index.

/// A disjoint-set forest over `0..n` elements.
///
/// Each element starts in its own singleton set. [`union`](Self::union) merges the
/// sets containing two elements; [`find`](Self::find) returns a set's
/// canonical representative (its root), so two elements are in the same set iff
/// their roots are equal.
#[derive(Debug, Clone)]
pub struct DisjointSet {
    /// `parent[i]` is `i`'s parent; a root is its own parent.
    parent: Vec<usize>,
    /// An upper bound on each root's tree height, used for union-by-rank.
    rank: Vec<u32>,
}

impl DisjointSet {
    /// Creates a forest of `n` singleton sets (`0..n`).
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    /// The number of elements the forest was created over.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// Returns `true` if the forest has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// Returns the canonical representative (root) of `x`'s set, compressing the
    /// path from `x` to the root so subsequent queries are faster.
    ///
    /// # Panics
    ///
    /// Panics if `x` is out of range (`x >= len`).
    pub fn find(&mut self, x: usize) -> usize {
        assert!(
            x < self.parent.len(),
            "index {x} out of range for union-find"
        );
        // Iterative find with full path compression: first walk to the root, then
        // point every node on the path directly at it. Iterative avoids recursion
        // depth blowing up on adversarial inputs.
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    /// Merges the sets containing `a` and `b`. Returns `true` if they were in
    /// different sets (i.e. a merge actually happened), `false` if they were
    /// already connected.
    ///
    /// # Panics
    ///
    /// Panics if either index is out of range.
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return false;
        }
        // Union by rank: attach the shorter tree under the taller one.
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
        true
    }

    /// Returns `true` if `a` and `b` are in the same set.
    ///
    /// # Panics
    ///
    /// Panics if either index is out of range.
    pub fn connected(&mut self, a: usize, b: usize) -> bool {
        self.find(a) == self.find(b)
    }
}

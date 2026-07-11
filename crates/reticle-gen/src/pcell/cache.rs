//! [`PCellCache`]: a `param_hash`-keyed store of produced cells, so regenerating a PCell
//! with parameters seen before returns the cached geometry instead of re-running its script.
//!
//! SCAFFOLD OWNED BY THE `pcell-cache` LANE. The type and its method signatures are fixed
//! here (an unbounded map with hit/miss counting) so the sandboxed producer
//! (`reticle_script`, `pcell-produce`) and the harness (`pcell-harness`) compile and can
//! exercise a get/insert cache. The `pcell-cache` lane replaces the unbounded map with a
//! size bound and an eviction policy (for example LRU), and hardens [`CacheStats`].

use std::collections::HashMap;

use reticle_model::Cell;

/// Hit and miss counters for a [`PCellCache`], reported for observability.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups that found a cached cell.
    pub hits: u64,
    /// Lookups that did not (the caller then produces and inserts).
    pub misses: u64,
}

/// A cache of produced [`Cell`]s keyed by their [`param_hash`](crate::param_hash).
///
/// The producer looks up a hash before running a script; on a miss it produces the cell and
/// inserts it, so a repeated `(def, params)` is served from memory. Identity is content: two
/// parameter sets with the same hash are the same geometry, so a hit is always correct.
///
/// SCAFFOLD: the map is unbounded here. The `pcell-cache` lane adds the capacity bound and
/// eviction so a long session cannot grow it without limit.
#[derive(Clone, Default, Debug)]
pub struct PCellCache {
    entries: HashMap<String, Cell>,
    stats: CacheStats,
}

impl PCellCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached cell for `param_hash`, cloned, or `None`, counting the lookup as a hit or
    /// a miss.
    pub fn get(&mut self, param_hash: &str) -> Option<Cell> {
        if let Some(cell) = self.entries.get(param_hash) {
            self.stats.hits += 1;
            Some(cell.clone())
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Inserts `cell` under `param_hash` (replacing any existing entry).
    pub fn insert(&mut self, param_hash: String, cell: Cell) {
        self.entries.insert(param_hash, cell);
    }

    /// The number of cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The hit and miss counters accumulated so far.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::PCellCache;
    use reticle_model::Cell;

    #[test]
    fn get_and_insert_count_hits_and_misses() {
        let mut cache = PCellCache::new();
        assert!(cache.is_empty());
        assert!(cache.get("abc").is_none(), "cold lookup misses");

        cache.insert("abc".to_owned(), Cell::new("top"));
        assert_eq!(cache.len(), 1);
        assert!(cache.get("abc").is_some(), "warm lookup hits");

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }
}

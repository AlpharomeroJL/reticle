//! [`PCellCache`]: a `param_hash`-keyed store of produced cells, so regenerating a PCell
//! with parameters seen before returns the cached geometry instead of re-running its script.
//!
//! The cache is capacity-bounded and evicts the least-recently-used entry when a new key
//! would push it past capacity, so a long editing session cannot grow it without limit.
//! Both a cache [`get`](PCellCache::get) hit and an [`insert`](PCellCache::insert) mark an
//! entry as most-recently-used, so entries in active use survive eviction. Identity is
//! content: two parameter sets with the same `param_hash` are the same geometry, so a hit
//! is always correct; the [`CacheStats`] hit/miss/eviction counters exist purely for
//! observability.

use std::collections::{HashMap, VecDeque};

use reticle_model::Cell;

/// Hit, miss, and eviction counters for a [`PCellCache`], reported for observability.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups that found a cached cell.
    pub hits: u64,
    /// Lookups that did not (the caller then produces and inserts).
    pub misses: u64,
    /// Entries removed to make room for a new one past capacity.
    pub evictions: u64,
}

/// The entry count a [`PCellCache::new`] cache is bounded to.
///
/// Generous enough that an ordinary editing session (tens to low hundreds of distinct
/// PCell parameter combinations touched) stays entirely cache-resident, while still
/// bounding memory over a very long session. Construct with [`PCellCache::with_capacity`]
/// for a different bound.
const DEFAULT_CAPACITY: usize = 256;

/// A capacity-bounded cache of produced [`Cell`]s keyed by their
/// [`param_hash`](crate::param_hash), evicting the least-recently-used entry once a new key
/// arrives past capacity.
///
/// The producer looks up a hash before running a script; on a miss it produces the cell and
/// inserts it, so a repeated `(def, params)` is served from memory. Identity is content: two
/// parameter sets with the same hash are the same geometry, so a hit is always correct;
/// correctness of this type is about the capacity bound and the eviction order.
#[derive(Clone, Debug)]
pub struct PCellCache {
    entries: HashMap<String, Cell>,
    /// Recency order: least-recently-used at the front, most-recently-used at the back.
    order: VecDeque<String>,
    capacity: usize,
    stats: CacheStats,
}

impl PCellCache {
    /// An empty cache bounded to a sensible default capacity (`DEFAULT_CAPACITY` entries;
    /// see [`Self::capacity`] to read back the bound on a constructed cache).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// An empty cache bounded to `capacity` entries.
    ///
    /// A capacity of `0` is a well-defined degenerate cache: every [`get`](Self::get)
    /// misses and [`insert`](Self::insert) is a no-op, rather than panicking or growing
    /// unbounded.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
            stats: CacheStats::default(),
        }
    }

    /// The maximum number of entries this cache retains before evicting.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The cached cell for `param_hash`, cloned, or `None`, counting the lookup as a hit or
    /// a miss. A hit refreshes `param_hash` as the most-recently-used entry.
    pub fn get(&mut self, param_hash: &str) -> Option<Cell> {
        let found = self.entries.get(param_hash).cloned();
        if found.is_some() {
            self.touch(param_hash);
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
        }
        found
    }

    /// Inserts `cell` under `param_hash` (replacing any existing entry) and marks it as the
    /// most-recently-used entry. If `param_hash` is a new key that would push the cache past
    /// capacity, evicts the least-recently-used entry first. A zero-capacity cache never
    /// stores anything, so this is a no-op.
    pub fn insert(&mut self, param_hash: String, cell: Cell) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.contains_key(&param_hash) {
            self.touch(&param_hash);
            self.entries.insert(param_hash, cell);
            return;
        }
        if self.entries.len() >= self.capacity {
            self.evict_lru();
        }
        self.order.push_back(param_hash.clone());
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

    /// The hit, miss, and eviction counters accumulated so far.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Moves `key` to the most-recently-used end of the recency order, if present.
    fn touch(&mut self, key: &str) {
        let Some(pos) = self.order.iter().position(|existing| existing == key) else {
            return;
        };
        if let Some(moved) = self.order.remove(pos) {
            self.order.push_back(moved);
        }
    }

    /// Removes the least-recently-used entry, if any, and counts it as an eviction.
    fn evict_lru(&mut self) {
        let Some(oldest) = self.order.pop_front() else {
            return;
        };
        self.entries.remove(&oldest);
        self.stats.evictions += 1;
    }
}

impl Default for PCellCache {
    fn default() -> Self {
        Self::new()
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
        assert_eq!(stats.evictions, 0);
    }

    #[test]
    fn insert_past_capacity_evicts_the_least_recently_used_entry() {
        let mut cache = PCellCache::with_capacity(2);
        cache.insert("a".to_owned(), Cell::new("a"));
        cache.insert("b".to_owned(), Cell::new("b"));
        // No gets in between: "a" is the least-recently-used of the two.
        cache.insert("c".to_owned(), Cell::new("c"));

        assert_eq!(cache.len(), 2, "capacity is never exceeded");
        assert!(cache.get("a").is_none(), "the LRU entry was evicted");
        assert!(
            cache.get("b").is_some(),
            "the other survivor is present (not an arbitrary pick)"
        );
        assert!(cache.get("c").is_some(), "the newest entry is present");
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn get_refreshes_recency_so_the_entry_survives_the_next_eviction() {
        let mut cache = PCellCache::with_capacity(2);
        cache.insert("a".to_owned(), Cell::new("a"));
        cache.insert("b".to_owned(), Cell::new("b"));

        // Touch "a" so "b" becomes the least-recently-used entry.
        assert!(cache.get("a").is_some());
        cache.insert("c".to_owned(), Cell::new("c"));

        assert!(cache.get("b").is_none(), "b was LRU after a was refreshed");
        assert!(
            cache.get("a").is_some(),
            "a survived because get refreshed it"
        );
        assert!(cache.get("c").is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn reinserting_an_existing_key_refreshes_recency_without_growing_len() {
        let mut cache = PCellCache::with_capacity(2);
        cache.insert("a".to_owned(), Cell::new("a"));
        cache.insert("b".to_owned(), Cell::new("b"));

        // Re-inserting "a" should refresh it, the same as a get would.
        cache.insert("a".to_owned(), Cell::new("a-v2"));
        assert_eq!(
            cache.len(),
            2,
            "replacing an existing key does not grow the cache"
        );

        cache.insert("c".to_owned(), Cell::new("c"));
        assert!(
            cache.get("b").is_none(),
            "b was LRU after a was re-inserted"
        );
        assert!(cache.get("a").is_some());
        assert!(cache.get("c").is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn zero_capacity_cache_stores_nothing_but_stays_well_behaved() {
        let mut cache = PCellCache::with_capacity(0);
        cache.insert("a".to_owned(), Cell::new("a"));

        assert!(cache.is_empty());
        assert!(cache.get("a").is_none());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.evictions, 0, "there was never anything to evict");
    }

    #[test]
    fn default_capacity_is_generous_enough_for_ordinary_use() {
        let cache = PCellCache::new();
        assert_eq!(cache.capacity(), super::DEFAULT_CAPACITY);
        assert!(cache.capacity() > 1);
    }

    #[test]
    fn with_capacity_reports_the_requested_bound() {
        let cache = PCellCache::with_capacity(7);
        assert_eq!(cache.capacity(), 7);
    }
}

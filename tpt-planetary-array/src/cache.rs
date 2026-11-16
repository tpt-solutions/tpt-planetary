//! A least-recently-used cache of byte buffers with a total byte budget.
//!
//! [`ChunkCache`] is the single-threaded core used by [`Array`](crate::Array)
//! when a chunk cache is enabled. It needs only `alloc`. The array handle
//! wraps it in a mutex, which requires the `std` feature (core has no mutex,
//! and the crate forbids `unsafe` and adds no locking dependency).

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug)]
struct Entry {
    value: Vec<u8>,
    tick: u64,
}

/// An LRU cache from string keys to byte buffers, bounded by a total byte
/// budget.
///
/// Inserting a value evicts the least-recently-used entries until the new
/// total fits in the budget. A value larger than the whole budget is not
/// cached. Both [`ChunkCache::get`] and [`ChunkCache::insert`] count as a use.
///
/// ```
/// use tpt_planetary_array::cache::ChunkCache;
///
/// let mut cache = ChunkCache::new(16);
/// cache.insert("a", vec![0; 8]);
/// cache.insert("b", vec![0; 8]);
/// assert!(cache.get("a").is_some()); // "a" is now more recent than "b"
/// cache.insert("c", vec![0; 8]); // evicts "b"
/// assert!(cache.get("b").is_none());
/// assert!(cache.get("a").is_some());
/// assert!(cache.used_bytes() <= cache.budget());
/// ```
#[derive(Debug)]
pub struct ChunkCache {
    budget: usize,
    used: usize,
    tick: u64,
    entries: BTreeMap<String, Entry>,
    /// Recency index: tick of last use to key, oldest first.
    order: BTreeMap<u64, String>,
}

impl ChunkCache {
    /// Creates an empty cache that holds at most `budget_bytes` bytes of
    /// values. A budget of zero caches nothing.
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget: budget_bytes,
            used: 0,
            tick: 0,
            entries: BTreeMap::new(),
            order: BTreeMap::new(),
        }
    }

    /// The byte budget.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Total bytes of values currently held. Never exceeds [`Self::budget`].
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the value under `key` and marks it most recently used.
    pub fn get(&mut self, key: &str) -> Option<&[u8]> {
        let tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.tick);
        entry.tick = tick;
        self.order.insert(tick, key.to_string());
        Some(&entry.value)
    }

    /// Stores `value` under `key`, replacing any previous value for `key`,
    /// and evicts least-recently-used entries to stay within the budget.
    /// Returns without caching if `value` alone exceeds the budget (any
    /// previous value for `key` is still removed).
    pub fn insert(&mut self, key: &str, value: Vec<u8>) {
        self.remove(key);
        if value.len() > self.budget {
            return;
        }
        while self.used + value.len() > self.budget {
            let Some((&oldest, _)) = self.order.first_key_value() else {
                break;
            };
            let evicted = self
                .order
                .remove(&oldest)
                .and_then(|victim| self.entries.remove(&victim));
            if let Some(entry) = evicted {
                self.used -= entry.value.len();
            }
        }
        let tick = self.next_tick();
        self.used += value.len();
        self.order.insert(tick, key.to_string());
        self.entries.insert(key.to_string(), Entry { value, tick });
    }

    /// Removes the value under `key`, if present.
    pub fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.order.remove(&entry.tick);
            self.used -= entry.value.len();
        }
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.used = 0;
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_within_budget() {
        let mut c = ChunkCache::new(24);
        c.insert("a", vec![1; 8]);
        c.insert("b", vec![2; 8]);
        c.insert("c", vec![3; 8]);
        assert_eq!(c.used_bytes(), 24);
        // Touch "a" so that "b" becomes the least recently used.
        assert_eq!(c.get("a"), Some(&[1u8; 8][..]));
        c.insert("d", vec![4; 8]);
        assert!(c.get("b").is_none());
        assert_eq!(c.get("a"), Some(&[1u8; 8][..]));
        assert_eq!(c.get("c"), Some(&[3u8; 8][..]));
        assert_eq!(c.get("d"), Some(&[4u8; 8][..]));
        assert!(c.used_bytes() <= c.budget());
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn large_insert_evicts_several_entries() {
        let mut c = ChunkCache::new(20);
        for k in ["a", "b", "c", "d"] {
            c.insert(k, vec![0; 5]);
        }
        c.insert("big", vec![9; 17]);
        assert_eq!(c.len(), 1);
        assert_eq!(c.used_bytes(), 17);
        assert!(c.get("big").is_some());
    }

    #[test]
    fn oversized_values_and_zero_budget_are_not_cached() {
        let mut c = ChunkCache::new(4);
        c.insert("x", vec![0; 5]);
        assert!(c.is_empty());
        let mut z = ChunkCache::new(0);
        z.insert("x", vec![]);
        assert!(z.get("x").is_some(), "zero-length values fit a zero budget");
        z.insert("y", vec![1]);
        assert!(z.get("y").is_none());
    }

    #[test]
    fn replacing_a_key_updates_size_and_recency() {
        let mut c = ChunkCache::new(16);
        c.insert("a", vec![0; 8]);
        c.insert("b", vec![0; 8]);
        c.insert("a", vec![7; 8]);
        assert_eq!(c.used_bytes(), 16);
        c.insert("c", vec![0; 8]);
        assert!(c.get("b").is_none());
        assert_eq!(c.get("a"), Some(&[7u8; 8][..]));
        c.remove("a");
        assert_eq!(c.used_bytes(), 8);
        c.clear();
        assert_eq!(c.used_bytes(), 0);
        assert!(c.is_empty());
    }
}

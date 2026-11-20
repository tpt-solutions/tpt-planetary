//! A wrapper that bounds how many store operations run at once.
//!
//! [`LimitedStore`] holds a fixed number of permits. Each call to the inner
//! store takes one permit and returns it when the call finishes, so a
//! parallel reader (for example the `threads` feature) cannot open more
//! concurrent requests to a remote store than the limit allows.

use alloc::string::String;
use alloc::vec::Vec;
use std::sync::{Condvar, Mutex, MutexGuard};

use super::Store;
use crate::error::Result;

/// A store that allows at most `limit` operations in flight at once.
#[derive(Debug)]
pub struct LimitedStore<S> {
    inner: S,
    state: Mutex<usize>,
    available: Condvar,
    limit: usize,
}

/// Returns a permit to the pool when dropped.
struct Permit<'a> {
    state: &'a Mutex<usize>,
    available: &'a Condvar,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        // A poisoned lock still holds a valid count, so recover it.
        let mut in_flight = self.state.lock().unwrap_or_else(|e| e.into_inner());
        *in_flight -= 1;
        self.available.notify_one();
    }
}

impl<S: Store> LimitedStore<S> {
    /// Wraps `inner` with `limit` permits. A limit of zero is raised to one.
    pub fn new(inner: S, limit: usize) -> Self {
        Self {
            inner,
            state: Mutex::new(0),
            available: Condvar::new(),
            limit: limit.max(1),
        }
    }

    /// The maximum number of concurrent operations.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// The wrapped store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    fn acquire(&self) -> Permit<'_> {
        let mut in_flight: MutexGuard<'_, usize> =
            self.state.lock().unwrap_or_else(|e| e.into_inner());
        while *in_flight >= self.limit {
            in_flight = self
                .available
                .wait(in_flight)
                .unwrap_or_else(|e| e.into_inner());
        }
        *in_flight += 1;
        Permit {
            state: &self.state,
            available: &self.available,
        }
    }

    fn with_permit<T>(&self, op: impl FnOnce(&S) -> Result<T>) -> Result<T> {
        let _permit = self.acquire();
        op(&self.inner)
    }
}

impl<S: Store> Store for LimitedStore<S> {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.with_permit(|s| s.get(key))
    }

    fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        self.with_permit(|s| s.get_range(key, start, len))
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.with_permit(|s| s.set(key, value))
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.with_permit(|s| s.delete(key))
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        self.with_permit(|s| s.list(prefix))
    }

    fn contains(&self, key: &str) -> Result<bool> {
        self.with_permit(|s| s.contains(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    /// Counts how many `get` calls overlap, and records the peak.
    struct Probe {
        inner: MemoryStore,
        active: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Store for Probe {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(5));
            let out = self.inner.get(key);
            self.active.fetch_sub(1, Ordering::SeqCst);
            out
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            self.inner.list(prefix)
        }
    }

    #[test]
    fn never_exceeds_the_permit_count() {
        let probe = Probe {
            inner: MemoryStore::new(),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        };
        probe.inner.set("k", b"v").unwrap();
        let store = LimitedStore::new(probe, 2);
        thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for _ in 0..3 {
                        assert_eq!(store.get("k").unwrap(), Some(b"v".to_vec()));
                    }
                });
            }
        });
        let peak = store.inner().peak.load(Ordering::SeqCst);
        assert!(peak <= 2, "peak concurrency {peak}");
        assert!(peak >= 1);
        assert_eq!(store.inner().active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_limit_is_raised_to_one_and_permits_are_returned_after_errors() {
        let store = LimitedStore::new(MemoryStore::new(), 0);
        assert_eq!(store.limit(), 1);
        // Keys that fail validation return errors; the permit must still be released.
        assert!(store.get("../bad").is_err());
        assert!(store.get("../bad").is_err());
        store.set("ok", b"1").unwrap();
        assert_eq!(store.get("ok").unwrap(), Some(b"1".to_vec()));
    }
}

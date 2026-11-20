//! Retry with exponential backoff for stores whose operations can fail
//! transiently.
//!
//! [`RetryStore`] wraps any [`Store`] and retries operations that fail with
//! an error for which [`Error::is_transient`](crate::Error::is_transient) is
//! true. The delay schedule is pure computation ([`RetryPolicy::delay_for`]).
//! Sleeping is injected, so tests run without waiting.
//!
//! The synchronous local stores ([`MemoryStore`](crate::store::MemoryStore),
//! [`FsStore`](crate::store::FsStore)) never produce transient errors, so a
//! retry wrapper around them only adds overhead.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;

use crate::error::Result;
use crate::store::Store;

/// Backoff schedule for retrying transient errors.
///
/// Retry `r` (counting from zero) waits `base_delay * multiplier^r`, capped
/// at `max_delay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Number of retries after the first attempt. Zero disables retrying.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub base_delay: Duration,
    /// Upper bound on any single delay.
    pub max_delay: Duration,
    /// Factor applied to the delay after each retry (values below 2 give a
    /// constant delay).
    pub multiplier: u32,
}

impl Default for RetryPolicy {
    /// Three retries, starting at 100 ms, doubling, capped at 5 s.
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            multiplier: 2,
        }
    }
}

impl RetryPolicy {
    /// The delay to wait before retry number `retry` (zero-based).
    ///
    /// ```
    /// use core::time::Duration;
    /// use tpt_planetary_array::retry::RetryPolicy;
    ///
    /// let p = RetryPolicy {
    ///     max_retries: 4,
    ///     base_delay: Duration::from_millis(10),
    ///     max_delay: Duration::from_millis(50),
    ///     multiplier: 2,
    /// };
    /// let ms: Vec<u64> = (0..5).map(|r| p.delay_for(r).as_millis() as u64).collect();
    /// assert_eq!(ms, vec![10, 20, 40, 50, 50]);
    /// ```
    pub fn delay_for(&self, retry: u32) -> Duration {
        let cap = self.max_delay;
        let mut delay = self.base_delay.min(cap);
        if self.multiplier < 2 {
            return delay;
        }
        for _ in 0..retry {
            delay = delay.saturating_mul(self.multiplier).min(cap);
            if delay == cap {
                break;
            }
        }
        delay
    }

    /// The full schedule of delays for every retry this policy allows.
    pub fn schedule(&self) -> Vec<Duration> {
        (0..self.max_retries).map(|r| self.delay_for(r)).collect()
    }
}

type SleepFn = Box<dyn Fn(Duration) + Send + Sync>;

/// A [`Store`] that retries transient failures of the wrapped store.
///
/// Each operation is attempted once, and then retried after the policy's
/// delay for as long as it fails with a transient error and retries remain.
/// The last error is returned otherwise.
pub struct RetryStore<S> {
    inner: S,
    policy: RetryPolicy,
    sleep: SleepFn,
}

impl<S: Store> RetryStore<S> {
    /// Wraps `inner`, calling `sleep` with each backoff delay.
    pub fn new(
        inner: S,
        policy: RetryPolicy,
        sleep: impl Fn(Duration) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner,
            policy,
            sleep: Box::new(sleep),
        }
    }

    /// Wraps `inner`, sleeping the current thread between attempts.
    #[cfg(feature = "std")]
    pub fn with_thread_sleep(inner: S, policy: RetryPolicy) -> Self {
        Self::new(inner, policy, std::thread::sleep)
    }

    /// The wrapped store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// The backoff policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    fn retrying<T>(&self, mut op: impl FnMut() -> Result<T>) -> Result<T> {
        let mut retry = 0u32;
        loop {
            match op() {
                Err(e) if e.is_transient() && retry < self.policy.max_retries => {
                    (self.sleep)(self.policy.delay_for(retry));
                    retry += 1;
                }
                other => return other,
            }
        }
    }
}

impl<S: fmt::Debug> fmt::Debug for RetryStore<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetryStore")
            .field("inner", &self.inner)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<S: Store> Store for RetryStore<S> {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.retrying(|| self.inner.get(key))
    }

    fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        self.retrying(|| self.inner.get_range(key, start, len))
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.retrying(|| self.inner.set(key, value))
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.retrying(|| self.inner.delete(key))
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        self.retrying(|| self.inner.list(prefix))
    }

    fn contains(&self, key: &str) -> Result<bool> {
        self.retrying(|| self.inner.contains(key))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::store::MemoryStore;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A store whose `get` fails transiently `failures` times, then succeeds.
    struct Flaky {
        inner: MemoryStore,
        failures: u32,
        calls: AtomicU32,
        permanent: bool,
    }

    impl Store for Flaky {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.failures {
                return Err(if self.permanent {
                    Error::Io("disk gone".into())
                } else {
                    Error::Transient(alloc::format!("timeout #{n}"))
                });
            }
            self.inner.get(key)
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

    fn policy(max_retries: u32) -> RetryPolicy {
        RetryPolicy {
            max_retries,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(40),
            multiplier: 2,
        }
    }

    fn flaky(failures: u32, permanent: bool) -> Flaky {
        let inner = MemoryStore::new();
        inner.set("k", b"v").unwrap();
        Flaky {
            inner,
            failures,
            calls: AtomicU32::new(0),
            permanent,
        }
    }

    type Recorded = std::sync::Arc<Mutex<Vec<Duration>>>;

    fn recording_store(s: Flaky, p: RetryPolicy) -> (RetryStore<Flaky>, Recorded) {
        let log: Recorded = Default::default();
        let sink = log.clone();
        let store = RetryStore::new(s, p, move |d| sink.lock().unwrap().push(d));
        (store, log)
    }

    #[test]
    fn succeeds_after_transient_failures_with_expected_delays() {
        let (store, log) = recording_store(flaky(3, false), policy(5));
        assert_eq!(store.get("k").unwrap(), Some(b"v".to_vec()));
        assert_eq!(store.inner().calls.load(Ordering::SeqCst), 4);
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
            ]
        );
    }

    #[test]
    fn gives_up_when_retries_are_exhausted() {
        let (store, log) = recording_store(flaky(10, false), policy(2));
        let err = store.get("k").unwrap_err();
        assert!(err.is_transient());
        assert_eq!(store.inner().calls.load(Ordering::SeqCst), 3);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn permanent_errors_are_not_retried() {
        let (store, log) = recording_store(flaky(1, true), policy(5));
        assert!(matches!(store.get("k"), Err(Error::Io(_))));
        assert_eq!(store.inner().calls.load(Ordering::SeqCst), 1);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn writes_and_listing_pass_through_when_healthy() {
        let (store, _) = recording_store(flaky(0, false), policy(3));
        store.set("a", b"1").unwrap();
        assert!(store.contains("a").unwrap());
        assert_eq!(store.list("").unwrap().len(), 2);
        store.delete("a").unwrap();
        assert!(!store.contains("a").unwrap());
    }

    #[test]
    fn delay_schedule_is_pure_and_capped() {
        let p = policy(5);
        assert_eq!(
            p.schedule(),
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(40),
                Duration::from_millis(40),
            ]
        );
        let constant = RetryPolicy { multiplier: 1, ..p };
        assert!(
            constant
                .schedule()
                .iter()
                .all(|d| *d == Duration::from_millis(10))
        );
        assert_eq!(RetryPolicy::default().max_retries, 3);
    }
}

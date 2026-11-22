//! An asynchronous storage trait, without a runtime.
//!
//! [`AsyncStore`] has the same operations as [`Store`], returning futures. The
//! crate does not depend on an async runtime: the futures are plain
//! `core::future::Future` values, so the caller can drive them with tokio,
//! async-std, a hand-written executor, or [`block_on`] in this module, which is
//! enough for tests and simple scripts.
//!
//! Every synchronous [`Store`] is an `AsyncStore` through a blanket impl, and
//! its futures complete immediately. A store with real asynchronous I/O
//! implements `AsyncStore` directly, and must not also implement [`Store`]
//! (the blanket impl would conflict).
//!
//! The synchronous array API is unchanged; an async array reader is not
//! provided yet.

use alloc::string::String;
use alloc::vec::Vec;
use core::future::{Future, ready};

use super::Store;
use crate::error::Result;

/// A key-value store whose operations are asynchronous.
///
/// The futures are `Send`, so they can be spawned on a multi-threaded runtime.
pub trait AsyncStore: Send + Sync {
    /// The value under `key`, or `None` if the key does not exist.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// `len` bytes of the value under `key` from byte `start`, clipped to the end.
    fn get_range(
        &self,
        key: &str,
        start: u64,
        len: u64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// The length of the value under `key`, or `None` if it does not exist.
    fn size(&self, key: &str) -> impl Future<Output = Result<Option<u64>>> + Send;

    /// Stores `value` under `key`, replacing any previous value.
    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Removes `key`. Removing a missing key is not an error.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Keys that start with `prefix`, in sorted order.
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>>> + Send;
}

impl<S: Store> AsyncStore for S {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Store::get(self, key))
    }

    fn get_range(
        &self,
        key: &str,
        start: u64,
        len: u64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Store::get_range(self, key, start, len))
    }

    fn size(&self, key: &str) -> impl Future<Output = Result<Option<u64>>> + Send {
        ready(Store::size(self, key))
    }

    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send {
        ready(Store::set(self, key, value))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        ready(Store::delete(self, key))
    }

    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        ready(Store::list(self, prefix))
    }
}

/// Runs a future to completion on the current thread, parking between polls.
///
/// This is a minimal executor for tests and scripts. It does not support
/// futures that wait on other threads: it polls again only when woken, and the
/// wake-up from another thread unparks it.
#[cfg(feature = "std")]
pub fn block_on<F: Future>(future: F) -> F::Output {
    use alloc::sync::Arc;
    use core::pin::pin;
    use core::task::{Context, Poll};
    use std::sync::{Condvar, Mutex};
    use std::task::Wake;

    struct Signal {
        woken: Mutex<bool>,
        cv: Condvar,
    }

    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
            *woken = true;
            self.cv.notify_one();
        }
    }

    let signal = Arc::new(Signal {
        woken: Mutex::new(true),
        cv: Condvar::new(),
    });
    let waker = core::task::Waker::from(signal.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        {
            let mut woken = signal.woken.lock().unwrap_or_else(|e| e.into_inner());
            while !*woken {
                woken = signal.cv.wait(woken).unwrap_or_else(|e| e.into_inner());
            }
            *woken = false;
        }
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::store::MemoryStore;

    /// A store that only implements `AsyncStore`, with asynchronous futures.
    struct Echo;

    impl AsyncStore for Echo {
        async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            Ok(Some(key.as_bytes().to_vec()))
        }

        async fn get_range(&self, _key: &str, _start: u64, _len: u64) -> Result<Option<Vec<u8>>> {
            Err(Error::Unsupported("echo has no ranges".into()))
        }

        async fn size(&self, key: &str) -> Result<Option<u64>> {
            Ok(Some(key.len() as u64))
        }

        async fn set(&self, _key: &str, _value: &[u8]) -> Result<()> {
            Err(Error::Unsupported("echo is read-only".into()))
        }

        async fn delete(&self, _key: &str) -> Result<()> {
            Ok(())
        }

        async fn list(&self, _prefix: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn synchronous_stores_are_async_stores() {
        let store = MemoryStore::new();
        block_on(AsyncStore::set(&store, "a/b", b"hello")).unwrap();
        assert_eq!(
            block_on(AsyncStore::get(&store, "a/b")).unwrap(),
            Some(b"hello".to_vec())
        );
        assert_eq!(
            block_on(AsyncStore::get_range(&store, "a/b", 1, 3)).unwrap(),
            Some(b"ell".to_vec())
        );
        assert_eq!(block_on(AsyncStore::size(&store, "a/b")).unwrap(), Some(5));
        assert_eq!(
            block_on(AsyncStore::list(&store, "a/")).unwrap(),
            vec!["a/b".to_string()]
        );
        block_on(AsyncStore::delete(&store, "a/b")).unwrap();
        assert_eq!(block_on(AsyncStore::get(&store, "a/b")).unwrap(), None);
    }

    #[test]
    fn native_async_store_needs_no_sync_impl() {
        let echo = Echo;
        assert_eq!(
            block_on(AsyncStore::get(&echo, "xyz")).unwrap(),
            Some(b"xyz".to_vec())
        );
        assert_eq!(block_on(AsyncStore::size(&echo, "xyz")).unwrap(), Some(3));
        assert!(block_on(AsyncStore::set(&echo, "k", b"v")).is_err());
    }

    #[test]
    fn block_on_completes_a_future_woken_from_another_thread() {
        use core::task::{Poll, Waker};
        use std::sync::{Arc, Mutex};

        struct Shared {
            done: bool,
            waker: Option<Waker>,
        }
        let shared = Arc::new(Mutex::new(Shared {
            done: false,
            waker: None,
        }));
        let s2 = shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let mut s = s2.lock().unwrap();
            s.done = true;
            if let Some(w) = s.waker.take() {
                w.wake();
            }
        });
        let fut = core::future::poll_fn(move |cx| {
            let mut s = shared.lock().unwrap();
            if s.done {
                Poll::Ready(7)
            } else {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        });
        assert_eq!(block_on(fut), 7);
    }
}

//! Synchronous key-value storage for Zarr hierarchies.
//!
//! Keys are `/`-separated relative paths such as `group/array/c/0/1`. The
//! [`Store`] trait is synchronous; an asynchronous trait is left for a
//! follow-up, since no async runtime is part of this crate.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// A key-value store holding Zarr metadata and chunk bytes.
///
/// Methods take `&self`; implementations that mutate state use interior
/// mutability so that a store can be shared between arrays via `Arc`.
pub trait Store: Send + Sync {
    /// Returns the bytes stored under `key`, or `None` if it does not exist.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Returns `len` bytes of the value under `key`, starting at byte `start`,
    /// or `None` if the key does not exist. A range that extends past the end
    /// of the value is clipped.
    ///
    /// The default reads the whole value and slices it. Stores that can read
    /// part of an object (such as [`HttpStore`]) override it.
    fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        Ok(self.get(key)?.map(|value| {
            let s = usize::try_from(start)
                .unwrap_or(usize::MAX)
                .min(value.len());
            let e = usize::try_from(start.saturating_add(len))
                .unwrap_or(usize::MAX)
                .min(value.len());
            value[s..e].to_vec()
        }))
    }

    /// Length in bytes of the value under `key`, or `None` if the key does not
    /// exist. The default reads the whole value; remote stores override it.
    fn size(&self, key: &str) -> Result<Option<u64>> {
        Ok(self.get(key)?.map(|v| v.len() as u64))
    }

    /// Stores `value` under `key`, replacing any previous value.
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;

    /// Removes `key`. Removing a missing key is not an error.
    fn delete(&self, key: &str) -> Result<()>;

    /// Returns every key that starts with `prefix`, in sorted order.
    /// An empty prefix lists the whole store.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;

    /// True if `key` exists.
    fn contains(&self, key: &str) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }
}

/// Checks that a key is a safe relative path: no empty, `.` or `..`
/// components, no backslashes, no leading slash.
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Ok(());
    }
    if key.starts_with('/') || key.contains('\\') {
        return Err(Error::InvalidSelection(format!(
            "invalid store key '{key}'"
        )));
    }
    for part in key.split('/') {
        if part == ".." || part == "." {
            return Err(Error::InvalidSelection(format!(
                "invalid store key '{key}'"
            )));
        }
    }
    Ok(())
}

/// Joins a group or array path and a child key with `/`, ignoring empty parts.
pub fn join(path: &str, child: &str) -> String {
    match (path.trim_matches('/'), child) {
        ("", c) => c.to_string(),
        (p, "") => p.to_string(),
        (p, c) => format!("{p}/{c}"),
    }
}

pub mod async_store;

#[cfg(feature = "std")]
pub use http::HttpStore;

#[cfg(feature = "std")]
pub use memory::MemoryStore;

#[cfg(feature = "std")]
pub use fs::FsStore;

#[cfg(feature = "std")]
mod http;

#[cfg(feature = "std")]
pub(crate) mod transport;

#[cfg(feature = "std")]
pub(crate) mod sigv4;

#[cfg(feature = "std")]
pub use sigv4::Credentials;

#[cfg(feature = "std")]
pub mod s3;

#[cfg(feature = "std")]
pub use s3::S3Store;

#[cfg(feature = "std")]
pub mod limit;

#[cfg(feature = "std")]
pub use limit::LimitedStore;

#[cfg(feature = "std")]
mod memory {
    use alloc::collections::BTreeMap;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use std::sync::Mutex;

    use super::{Store, validate_key};
    use crate::error::{Error, Result};

    /// An in-memory store backed by a `BTreeMap`, intended for tests and
    /// small arrays.
    #[derive(Debug, Default)]
    pub struct MemoryStore {
        map: Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl MemoryStore {
        /// Creates an empty store.
        pub fn new() -> Self {
            Self::default()
        }

        fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>>> {
            self.map
                .lock()
                .map_err(|_| Error::Io("memory store lock poisoned".to_string()))
        }

        /// Number of keys currently stored.
        pub fn len(&self) -> Result<usize> {
            Ok(self.lock()?.len())
        }

        /// True if the store holds no keys.
        pub fn is_empty(&self) -> Result<bool> {
            Ok(self.len()? == 0)
        }
    }

    impl Store for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            validate_key(key)?;
            Ok(self.lock()?.get(key).cloned())
        }

        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            validate_key(key)?;
            self.lock()?.insert(key.to_string(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<()> {
            validate_key(key)?;
            self.lock()?.remove(key);
            Ok(())
        }

        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            let map = self.lock()?;
            Ok(map
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }

        fn contains(&self, key: &str) -> Result<bool> {
            validate_key(key)?;
            Ok(self.lock()?.contains_key(key))
        }
    }
}

#[cfg(feature = "std")]
mod fs {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{Store, validate_key};
    use crate::error::{Error, Result};

    /// A store that maps keys to files below a root directory.
    ///
    /// Writes go to a temporary sibling file that is then renamed into place,
    /// so readers never observe a partially written chunk.
    #[derive(Debug)]
    pub struct FsStore {
        root: PathBuf,
        counter: AtomicU64,
    }

    fn io_err(e: io::Error) -> Error {
        Error::Io(e.to_string())
    }

    impl FsStore {
        /// Opens (creating if necessary) a store rooted at `root`.
        pub fn open(root: impl AsRef<Path>) -> Result<Self> {
            fs::create_dir_all(root.as_ref()).map_err(io_err)?;
            Ok(Self {
                root: root.as_ref().to_path_buf(),
                counter: AtomicU64::new(0),
            })
        }

        /// The root directory of the store.
        pub fn root(&self) -> &Path {
            &self.root
        }

        fn path_for(&self, key: &str) -> Result<PathBuf> {
            validate_key(key)?;
            let mut p = self.root.clone();
            for part in key.split('/').filter(|s| !s.is_empty()) {
                p.push(part);
            }
            Ok(p)
        }

        fn walk(&self, dir: &Path, out: &mut Vec<String>) -> Result<()> {
            let entries = match fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(io_err(e)),
            };
            for entry in entries {
                let entry = entry.map_err(io_err)?;
                let file_type = entry.file_type().map_err(io_err)?;
                let path = entry.path();
                if file_type.is_dir() {
                    self.walk(&path, out)?;
                } else if file_type.is_file() {
                    let rel = path
                        .strip_prefix(&self.root)
                        .map_err(|_| Error::Io("path outside store root".to_string()))?;
                    let key = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    if !key.ends_with(".partial") {
                        out.push(key);
                    }
                }
            }
            Ok(())
        }
    }

    impl Store for FsStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            match fs::read(self.path_for(key)?) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io_err(e)),
            }
        }

        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            let path = self.path_for(key)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io_err)?;
            }
            let n = self.counter.fetch_add(1, Ordering::Relaxed);
            let mut tmp = path.clone().into_os_string();
            tmp.push(format!(".{}.{n}.partial", std::process::id()));
            let tmp = PathBuf::from(tmp);
            fs::write(&tmp, value).map_err(io_err)?;
            fs::rename(&tmp, &path).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                io_err(e)
            })
        }

        fn delete(&self, key: &str) -> Result<()> {
            match fs::remove_file(self.path_for(key)?) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_err(e)),
            }
        }

        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            // Start the walk at the deepest directory named by the prefix.
            let dir_part = match prefix.rfind('/') {
                Some(i) => &prefix[..i],
                None => "",
            };
            let start = self.path_for(dir_part)?;
            let mut keys = Vec::new();
            self.walk(&start, &mut keys)?;
            keys.retain(|k| k.starts_with(prefix));
            keys.sort();
            Ok(keys)
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip_and_list() {
        let s = MemoryStore::new();
        assert!(s.get("a/b").unwrap().is_none());
        s.set("a/b", b"1").unwrap();
        s.set("a/c", b"2").unwrap();
        s.set("z", b"3").unwrap();
        assert_eq!(s.get("a/b").unwrap(), Some(b"1".to_vec()));
        assert_eq!(
            s.list("a/").unwrap(),
            vec!["a/b".to_string(), "a/c".to_string()]
        );
        s.delete("a/b").unwrap();
        s.delete("a/b").unwrap();
        assert_eq!(s.list("a/").unwrap(), vec!["a/c".to_string()]);
    }

    #[test]
    fn fs_store_round_trip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("tpt-array-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = FsStore::open(&dir).unwrap();
        s.set("g/arr/c/0/1", &[1, 2, 3]).unwrap();
        s.set("g/.zgroup", b"{}").unwrap();
        assert_eq!(s.get("g/arr/c/0/1").unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(s.list("g/arr").unwrap(), vec!["g/arr/c/0/1".to_string()]);
        assert_eq!(s.list("").unwrap().len(), 2);
        s.delete("g/.zgroup").unwrap();
        assert!(!s.contains("g/.zgroup").unwrap());
        assert!(s.set("../escape", b"x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn validates_keys() {
        assert!(validate_key("a/b/c").is_ok());
        assert!(validate_key("/abs").is_err());
        assert!(validate_key("a/../b").is_err());
        assert!(validate_key("a\\b").is_err());
    }
}

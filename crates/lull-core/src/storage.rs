// SPDX-License-Identifier: AGPL-3.0-or-later
//! The storage port: named byte blobs (security.md §5).
//!
//! Apps adapt this to the platform's protected storage (Keychain, Android
//! Keystore, DPAPI, Secret Service, IndexedDB). The engines store a few
//! small values, each CBOR with a leading version byte.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::rt::{MaybeSend, MaybeSync};

/// A storage failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// The backend failed.
    #[error("storage backend: {0}")]
    Backend(String),
    /// A stored value is malformed or of an unknown version.
    #[error("stored value {key:?} is invalid: {why}")]
    Corrupt {
        /// Storage key.
        key: String,
        /// What is wrong.
        why: String,
    },
}

/// Named blobs. Keys are short ASCII names (`device-key`,
/// `station-trust`, ...). Calls are synchronous and small; adapters for
/// asynchronous stores keep a write-through in-memory copy.
pub trait Storage: MaybeSend + MaybeSync {
    /// Reads a value.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    /// Writes a value, replacing any previous one.
    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError>;
    /// Removes a value; absent keys are not an error.
    fn delete(&self, key: &str) -> Result<(), StorageError>;
}

/// In-memory storage; clones share the same data (tests, ephemeral use).
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    data: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl MemoryStorage {
    /// Empty storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stored keys.
    pub fn keys(&self) -> Vec<String> {
        self.data
            .lock()
            .expect("poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.data.lock().expect("poisoned").get(key).cloned())
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.data
            .lock()
            .expect("poisoned")
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.data.lock().expect("poisoned").remove(key);
        Ok(())
    }
}

/// One file per key in a directory, written atomically and readable by the
/// owner only (security.md §5, Linux fallback). For tools and tests; apps
/// use the platform key store.
#[cfg(feature = "native")]
#[derive(Debug, Clone)]
pub struct DirStorage {
    dir: std::path::PathBuf,
}

#[cfg(feature = "native")]
impl DirStorage {
    /// Uses (and creates) `dir`.
    pub fn open(dir: impl Into<std::path::PathBuf>) -> Result<Self, StorageError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(backend)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self { dir })
    }

    /// The directory.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn path(&self, key: &str) -> Result<std::path::PathBuf, StorageError> {
        let ok = !key.is_empty()
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            && !key.starts_with('.');
        if !ok {
            return Err(StorageError::Backend(format!("invalid key {key:?}")));
        }
        Ok(self.dir.join(format!("{key}.bin")))
    }
}

#[cfg(feature = "native")]
fn backend(e: std::io::Error) -> StorageError {
    StorageError::Backend(e.to_string())
}

#[cfg(feature = "native")]
impl Storage for DirStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match std::fs::read(self.path(key)?) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(backend(e)),
        }
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        use std::io::Write;
        let path = self.path(key)?;
        let tmp = path.with_extension("tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(backend)?;
        f.write_all(value).map_err(backend)?;
        f.sync_all().map_err(backend)?;
        drop(f);
        std::fs::rename(&tmp, &path).map_err(backend)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        match std::fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(backend(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_storage_roundtrip_and_shared_clones() {
        let a = MemoryStorage::new();
        let b = a.clone();
        a.put("k", b"v").unwrap();
        assert_eq!(b.get("k").unwrap().as_deref(), Some(&b"v"[..]));
        b.delete("k").unwrap();
        b.delete("k").unwrap();
        assert_eq!(a.get("k").unwrap(), None);
    }

    #[cfg(feature = "native")]
    #[test]
    fn dir_storage_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lull-core-dir-{}", std::process::id()));
        let s = DirStorage::open(&dir).unwrap();
        assert_eq!(s.get("device-key").unwrap(), None);
        s.put("device-key", b"\x01abc").unwrap();
        assert_eq!(s.get("device-key").unwrap().unwrap(), b"\x01abc");
        assert!(s.put("../x", b"").is_err());
        s.delete("device-key").unwrap();
        assert_eq!(s.get("device-key").unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

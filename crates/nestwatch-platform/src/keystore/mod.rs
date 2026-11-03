// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secret storage for the device key and trust stores (security.md §5).
//!
//! Secrets are opaque named byte blobs. Names are short ASCII identifiers
//! (`[a-z0-9._-]`, 1–64 characters, not starting with `.`), so every backend
//! can use them as file names or account names. Secret bytes are never
//! logged or formatted; [`Secret`] zeroes its buffer on drop.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::PlatformError;

pub mod file;
#[cfg(feature = "keyring")]
pub mod os;
pub mod sealed;

/// Longest accepted secret name.
pub const MAX_NAME_LEN: usize = 64;

/// Secret bytes, zeroed on drop and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    /// Wraps bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The secret bytes.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<{} bytes redacted>)", self.0.len())
    }
}

/// Named secret storage.
pub trait KeyStore: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Reads a secret; `Ok(None)` when it does not exist.
    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError>;
    /// Creates or replaces a secret.
    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError>;
    /// Deletes a secret; returns whether it existed.
    fn delete(&self, name: &str) -> Result<bool, PlatformError>;
}

/// Checks a secret name (see the module docs).
pub fn validate_name(name: &str) -> Result<(), PlatformError> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('.')
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(PlatformError::InvalidArgument("secret name"))
    }
}

/// In-memory store, for tests and for platforms without persistence.
#[derive(Default)]
pub struct MemoryKeyStore {
    items: Mutex<HashMap<String, Secret>>,
}

impl fmt::Debug for MemoryKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.items.lock().map(|m| m.len()).unwrap_or(0);
        write!(f, "MemoryKeyStore({n} items)")
    }
}

impl MemoryKeyStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn items(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Secret>>, PlatformError> {
        self.items
            .lock()
            .map_err(|_| PlatformError::Backend("key store lock poisoned".into()))
    }
}

impl KeyStore for MemoryKeyStore {
    fn backend(&self) -> &'static str {
        "memory"
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError> {
        validate_name(name)?;
        Ok(self.items()?.get(name).cloned())
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        validate_name(name)?;
        self.items()?
            .insert(name.to_owned(), Secret::new(value.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<bool, PlatformError> {
        validate_name(name)?;
        Ok(self.items()?.remove(name).is_some())
    }
}

/// A primary store with a read-only fallback: reads try the primary first,
/// then the fallback (so secrets written while the primary was unavailable
/// are still found); writes go to the primary; deletes remove from both.
pub struct LayeredKeyStore {
    primary: Box<dyn KeyStore>,
    fallback: Box<dyn KeyStore>,
}

impl fmt::Debug for LayeredKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LayeredKeyStore({} over {})",
            self.primary.backend(),
            self.fallback.backend()
        )
    }
}

impl LayeredKeyStore {
    /// Layers `primary` over `fallback`.
    pub fn new(primary: Box<dyn KeyStore>, fallback: Box<dyn KeyStore>) -> Self {
        Self { primary, fallback }
    }
}

impl KeyStore for LayeredKeyStore {
    fn backend(&self) -> &'static str {
        self.primary.backend()
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError> {
        match self.primary.get(name)? {
            Some(s) => Ok(Some(s)),
            None => self.fallback.get(name),
        }
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        self.primary.put(name, value)?;
        // Drop a stale fallback copy so the two never disagree.
        if let Err(e) = self.fallback.delete(name) {
            log::warn!("could not remove fallback copy of secret {name}: {e}");
        }
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<bool, PlatformError> {
        let a = self.primary.delete(name)?;
        let b = self.fallback.delete(name)?;
        Ok(a || b)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Behaviour every store must have.
    pub(crate) fn exercise(store: &dyn KeyStore) {
        assert_eq!(store.get("device-key").unwrap(), None);
        store.put("device-key", b"\x00\x01secret").unwrap();
        assert_eq!(
            store.get("device-key").unwrap().unwrap().expose(),
            b"\x00\x01secret"
        );
        store.put("device-key", b"replaced").unwrap();
        assert_eq!(
            store.get("device-key").unwrap().unwrap().expose(),
            b"replaced"
        );
        store.put("trust.listeners", b"").unwrap();
        assert_eq!(store.get("trust.listeners").unwrap().unwrap().len(), 0);
        assert!(store.delete("device-key").unwrap());
        assert!(!store.delete("device-key").unwrap());
        assert_eq!(store.get("device-key").unwrap(), None);
        for bad in ["", ".hidden", "Upper", "a/b", "a b", &"x".repeat(65)] {
            assert!(store.get(bad).is_err(), "{bad:?}");
            assert!(store.put(bad, b"x").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names() {
        assert!(validate_name("device-key").is_ok());
        assert!(validate_name("trust.v1_listeners").is_ok());
        assert!(validate_name(&"a".repeat(64)).is_ok());
        assert!(validate_name("..").is_err());
        assert!(validate_name("é").is_err());
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new(b"hunter2".to_vec());
        assert_eq!(format!("{s:?}"), "Secret(<7 bytes redacted>)");
    }

    #[test]
    fn memory_store() {
        exercise(&MemoryKeyStore::new());
    }

    #[test]
    fn layered_store_reads_fallback_and_deletes_both() {
        let fallback = MemoryKeyStore::new();
        fallback.put("old", b"from-file").unwrap();
        fallback.put("dup", b"stale").unwrap();
        let primary = MemoryKeyStore::new();
        primary.put("dup", b"fresh").unwrap();
        let s = LayeredKeyStore::new(Box::new(primary), Box::new(fallback));
        assert_eq!(s.get("old").unwrap().unwrap().expose(), b"from-file");
        assert_eq!(s.get("dup").unwrap().unwrap().expose(), b"fresh");
        s.put("old", b"new").unwrap();
        assert_eq!(s.get("old").unwrap().unwrap().expose(), b"new");
        assert!(s.delete("dup").unwrap());
        assert_eq!(s.get("dup").unwrap(), None);
        exercise(&s);
    }
}

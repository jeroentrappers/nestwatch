// SPDX-License-Identifier: AGPL-3.0-or-later
//! The engines' `Storage` port over the platform key store (security.md §5).

use std::sync::Arc;

use nestwatch_core::{Storage, StorageError};
use nestwatch_platform::KeyStore;

/// Named engine values (`device-key`, `station-trust`, ...) as secrets in
/// the OS key store (Secret Service / Keychain / Credential Manager, or the
/// 0600 file fallback, PLATFORM-1..3).
#[derive(Clone)]
pub struct KeyStoreStorage {
    store: Arc<dyn KeyStore>,
}

impl std::fmt::Debug for KeyStoreStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStoreStorage")
            .field("backend", &self.store.backend())
            .finish()
    }
}

impl KeyStoreStorage {
    /// Wraps a key store.
    pub fn new(store: Arc<dyn KeyStore>) -> Self {
        Self { store }
    }
}

fn err(e: nestwatch_platform::PlatformError) -> StorageError {
    StorageError::Backend(e.to_string())
}

impl Storage for KeyStoreStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.store
            .get(key)
            .map(|s| s.map(|s| s.expose().to_vec()))
            .map_err(err)
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.store.put(key, value).map_err(err)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.store.delete(key).map(|_| ()).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nestwatch_platform::keystore::MemoryKeyStore;

    #[test]
    fn engine_keys_roundtrip_through_a_key_store() {
        let s = KeyStoreStorage::new(Arc::new(MemoryKeyStore::new()));
        for key in [
            "device-key",
            "station-trust",
            "station-settings",
            "station-cert",
            "listener-stations",
            "listener-settings",
        ] {
            assert_eq!(s.get(key).unwrap(), None);
            s.put(key, b"\x01value").unwrap();
            assert_eq!(s.get(key).unwrap().unwrap(), b"\x01value");
            s.delete(key).unwrap();
            s.delete(key).unwrap();
            assert_eq!(s.get(key).unwrap(), None);
        }
        // The engines can start on it.
        let key = nestwatch_core::load_or_create_device_key(&s).unwrap();
        let again = nestwatch_core::load_or_create_device_key(&s).unwrap();
        assert_eq!(key.device_id(), again.device_id());
    }
}

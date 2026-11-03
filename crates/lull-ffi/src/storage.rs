// SPDX-License-Identifier: AGPL-3.0-or-later
//! The foreign storage port (security.md §5, CORE-1, CORE-13).

use std::sync::Arc;

use lull_core::StorageError;

use crate::error::StorageFailure;

/// Named byte blobs kept by the app in protected storage: Android Keystore
/// encrypted files, iOS Keychain items (security.md §5). Keys are short
/// ASCII names (`device-key`, `station-trust`, `station-settings`,
/// `listener-stations`, ...). Values are opaque and small. Calls are
/// synchronous and rare; they come from engine threads. Implementations
/// must never log values and must keep them out of backups.
#[uniffi::export(foreign)]
pub trait Storage: Send + Sync {
    /// Reads a value; `null`/`nil` if absent.
    fn get(&self, key: String) -> Result<Option<Vec<u8>>, StorageFailure>;
    /// Writes a value atomically, replacing any previous one.
    fn put(&self, key: String, value: Vec<u8>) -> Result<(), StorageFailure>;
    /// Removes a value; absent keys are not an error.
    fn delete(&self, key: String) -> Result<(), StorageFailure>;
}

/// Adapts the foreign storage to the core's port.
pub(crate) struct ForeignStorage(pub Arc<dyn Storage>);

fn backend(e: StorageFailure) -> StorageError {
    match e {
        StorageFailure::Backend { reason } => StorageError::Backend(reason),
    }
}

impl lull_core::Storage for ForeignStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.0.get(key.to_owned()).map_err(backend)
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.0.put(key.to_owned(), value.to_vec()).map_err(backend)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.0.delete(key.to_owned()).map_err(backend)
    }
}

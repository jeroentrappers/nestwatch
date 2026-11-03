// SPDX-License-Identifier: AGPL-3.0-or-later
//! The engine's `Storage` port over the origin's `localStorage`
//! (security.md §5 "Browser"; FR-WEB-3). Values are base64 under
//! `nestwatch:<key>`. `localStorage` is synchronous, which matches the
//! synchronous port (CORE-13) without a write-through cache; see
//! OPEN-QUESTIONS UI-4 for why not IndexedDB.

use base64::Engine as _;
use nestwatch_core::{Storage, StorageError};

const PREFIX: &str = "nestwatch:";

/// `localStorage` of the station's origin.
#[derive(Debug, Clone)]
pub struct LocalStorage {
    store: web_sys::Storage,
}

impl LocalStorage {
    /// The window's local storage, if the browser allows it.
    pub fn open() -> Result<Self, StorageError> {
        let store = web_sys::window()
            .ok_or_else(|| backend("no window"))?
            .local_storage()
            .map_err(|_| backend("localStorage is blocked"))?
            .ok_or_else(|| backend("no localStorage"))?;
        Ok(Self { store })
    }

    /// A plain string value (UI preferences, not secrets).
    pub fn get_text(&self, key: &str) -> Option<String> {
        self.store
            .get_item(&format!("{PREFIX}{key}"))
            .ok()
            .flatten()
    }

    /// Stores a plain string value.
    pub fn set_text(&self, key: &str, value: &str) {
        let _ = self.store.set_item(&format!("{PREFIX}{key}"), value);
    }
}

fn backend(why: &str) -> StorageError {
    StorageError::Backend(why.to_owned())
}

impl Storage for LocalStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let Some(v) = self
            .store
            .get_item(&format!("{PREFIX}{key}"))
            .map_err(|_| backend("read failed"))?
        else {
            return Ok(None);
        };
        base64::engine::general_purpose::STANDARD
            .decode(v)
            .map(Some)
            .map_err(|e| StorageError::Corrupt {
                key: key.to_owned(),
                why: e.to_string(),
            })
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.store
            .set_item(
                &format!("{PREFIX}{key}"),
                &base64::engine::general_purpose::STANDARD.encode(value),
            )
            .map_err(|_| backend("write failed (storage full or blocked)"))
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.store
            .remove_item(&format!("{PREFIX}{key}"))
            .map_err(|_| backend("delete failed"))
    }
}

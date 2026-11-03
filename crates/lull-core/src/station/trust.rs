// SPDX-License-Identifier: AGPL-3.0-or-later
//! The trusted listener list (protocol.md §2.3, product.md FR-ST-7).

use lull_crypto::PublicKey;
use minicbor::{Decode, Encode};

use crate::persist;
use crate::storage::{Storage, StorageError};

/// Storage key of the trusted listener list.
pub const KEY_TRUST: &str = "station-trust";

/// A listener the station trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedListener {
    /// Static public key.
    pub key: PublicKey,
    /// Device ID (protocol.md §2.2), canonical form.
    pub id: String,
    /// Display name from its `hello`.
    pub name: String,
    /// When it paired (Unix ms).
    pub paired_unix_ms: u64,
    /// When it was last connected (Unix ms).
    pub last_seen_unix_ms: u64,
}

#[derive(Encode, Decode)]
struct Record {
    #[n(0)]
    listeners: Vec<Entry>,
}

#[derive(Encode, Decode)]
struct Entry {
    #[b(0)]
    key: Vec<u8>,
    #[n(1)]
    name: String,
    #[n(2)]
    paired: u64,
    #[n(3)]
    last_seen: u64,
}

/// In-memory trust list mirrored to storage.
#[derive(Debug, Default)]
pub(crate) struct TrustStore {
    list: Vec<TrustedListener>,
}

impl TrustStore {
    pub fn load(storage: &dyn Storage) -> Result<Self, StorageError> {
        let Some(rec) = persist::load::<Record>(storage, KEY_TRUST)? else {
            return Ok(Self::default());
        };
        let mut list = Vec::new();
        for e in rec.listeners {
            let key = persist::public_key(KEY_TRUST, &e.key)?;
            list.push(TrustedListener {
                id: key.device_id().as_str().to_owned(),
                key,
                name: e.name,
                paired_unix_ms: e.paired,
                last_seen_unix_ms: e.last_seen,
            });
        }
        Ok(Self { list })
    }

    pub fn save(&self, storage: &dyn Storage) -> Result<(), StorageError> {
        let rec = Record {
            listeners: self
                .list
                .iter()
                .map(|l| Entry {
                    key: l.key.as_bytes().to_vec(),
                    name: l.name.clone(),
                    paired: l.paired_unix_ms,
                    last_seen: l.last_seen_unix_ms,
                })
                .collect(),
        };
        persist::save(storage, KEY_TRUST, &rec)
    }

    pub fn list(&self) -> &[TrustedListener] {
        &self.list
    }

    pub fn contains(&self, key: &PublicKey) -> bool {
        self.list.iter().any(|l| &l.key == key)
    }

    #[cfg(test)]
    pub fn get(&self, key: &PublicKey) -> Option<&TrustedListener> {
        self.list.iter().find(|l| &l.key == key)
    }

    /// Adds (or re-pairs) a listener.
    pub fn add(&mut self, key: PublicKey, name: &str, now_unix: u64) {
        self.list.retain(|l| l.key != key);
        self.list.push(TrustedListener {
            id: key.device_id().as_str().to_owned(),
            key,
            name: name.to_owned(),
            paired_unix_ms: now_unix,
            last_seen_unix_ms: now_unix,
        });
    }

    /// Updates name and last-seen time of a connected listener.
    pub fn seen(&mut self, key: &PublicKey, name: &str, now_unix: u64) {
        if let Some(l) = self.list.iter_mut().find(|l| &l.key == key) {
            l.name = name.to_owned();
            l.last_seen_unix_ms = now_unix;
        }
    }

    /// Removes a listener; returns it if it was trusted.
    pub fn remove(&mut self, key: &PublicKey) -> Option<TrustedListener> {
        let pos = self.list.iter().position(|l| &l.key == key)?;
        Some(self.list.remove(pos))
    }

    /// Finds a key by device ID (either text form).
    pub fn find_id(&self, id: &str) -> Option<PublicKey> {
        let id = lull_proto::DeviceId::parse(id).ok()?;
        self.list
            .iter()
            .find(|l| l.id == id.as_str())
            .map(|l| l.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStorage;

    #[test]
    fn fr_st_7_trust_list_persists_and_revokes() {
        let s = MemoryStorage::new();
        let k = *lull_crypto::DeviceKey::from_secret_bytes(&[3; 32]).public_key();
        let mut t = TrustStore::load(&s).unwrap();
        t.add(k, "Phone", 10);
        t.seen(&k, "Phone 2", 20);
        t.save(&s).unwrap();
        let mut t = TrustStore::load(&s).unwrap();
        let l = t.get(&k).unwrap().clone();
        assert_eq!(
            (l.name.as_str(), l.paired_unix_ms, l.last_seen_unix_ms),
            ("Phone 2", 10, 20)
        );
        assert_eq!(t.find_id(&l.id), Some(k));
        assert!(t.remove(&k).is_some());
        assert!(!t.contains(&k));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secrets in files, sealed with a wrapping key held by an OS key store.
//!
//! OS credential stores limit blob sizes (Windows Credential Manager: 2560
//! bytes), which a trust store can outgrow. This store keeps each secret as
//! `XChaCha20-Poly1305(wrapping key, nonce, secret, aad = name)` in a
//! [`FileKeyStore`] and only the 32-byte wrapping key in the OS store. It
//! gives the user-scope protection security.md §5 asks of DPAPI on Windows.

use std::fmt;
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::file::FileKeyStore;
use super::{KeyStore, Secret, validate_name};
use crate::PlatformError;

/// Name of the wrapping key in the OS store.
pub const WRAPPING_KEY_NAME: &str = "sealed-store-key";
const MAGIC: &[u8; 4] = b"LUL1";
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// A [`FileKeyStore`] whose contents are sealed by a key in another store.
pub struct SealedKeyStore {
    keys: Box<dyn KeyStore>,
    files: FileKeyStore,
    cached: Mutex<Option<Secret>>,
}

impl fmt::Debug for SealedKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SealedKeyStore(key in {}, files in {})",
            self.keys.backend(),
            self.files.dir().display()
        )
    }
}

impl SealedKeyStore {
    /// Seals files in `files` with a wrapping key kept in `keys`.
    pub fn new(keys: Box<dyn KeyStore>, files: FileKeyStore) -> Self {
        Self {
            keys,
            files,
            cached: Mutex::new(None),
        }
    }

    /// Returns the wrapping key, creating it when `create` is set.
    fn wrapping_key(&self, create: bool) -> Result<Option<Secret>, PlatformError> {
        let mut cached = self
            .cached
            .lock()
            .map_err(|_| PlatformError::Backend("key store lock poisoned".into()))?;
        if let Some(k) = cached.as_ref() {
            return Ok(Some(k.clone()));
        }
        let key = match self.keys.get(WRAPPING_KEY_NAME)? {
            Some(k) if k.len() == KEY_LEN => Some(k),
            Some(_) => {
                return Err(PlatformError::Backend(
                    "wrapping key has the wrong size".into(),
                ));
            }
            None if create => {
                let mut bytes = zeroize::Zeroizing::new(vec![0u8; KEY_LEN]);
                getrandom::fill(&mut bytes)
                    .map_err(|e| PlatformError::Backend(format!("random: {e}")))?;
                self.keys.put(WRAPPING_KEY_NAME, &bytes)?;
                Some(Secret::new(bytes.to_vec()))
            }
            None => None,
        };
        cached.clone_from(&key);
        Ok(key)
    }
}

fn cipher(key: &Secret) -> Result<XChaCha20Poly1305, PlatformError> {
    XChaCha20Poly1305::new_from_slice(key.expose())
        .map_err(|_| PlatformError::Backend("bad wrapping key".into()))
}

impl KeyStore for SealedKeyStore {
    fn backend(&self) -> &'static str {
        "sealed file"
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError> {
        validate_name(name)?;
        let Some(blob) = self.files.get(name)? else {
            return Ok(None);
        };
        let blob = blob.expose();
        if blob.len() < MAGIC.len() + NONCE_LEN || &blob[..MAGIC.len()] != MAGIC {
            return Err(PlatformError::Backend(format!(
                "sealed secret {name} is malformed"
            )));
        }
        let key = self.wrapping_key(false)?.ok_or_else(|| {
            PlatformError::Unavailable(format!("wrapping key for secret {name} is missing"))
        })?;
        let (nonce, ct) = blob[MAGIC.len()..].split_at(NONCE_LEN);
        let plain = cipher(&key)?
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| PlatformError::Backend(format!("sealed secret {name} failed to open")))?;
        Ok(Some(Secret::new(plain)))
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        validate_name(name)?;
        let key = self
            .wrapping_key(true)?
            .ok_or_else(|| PlatformError::Backend("no wrapping key".into()))?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| PlatformError::Backend(format!("random: {e}")))?;
        let ct = cipher(&key)?
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: value,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| PlatformError::Backend("seal failed".into()))?;
        let mut blob = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
        blob.extend_from_slice(MAGIC);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        self.files.put(name, &blob)
    }

    fn delete(&self, name: &str) -> Result<bool, PlatformError> {
        self.files.delete(name)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::MemoryKeyStore;
    use super::super::file::temp_dir;
    use super::*;

    /// Shares one memory store between two sealed stores.
    struct Shared(Arc<MemoryKeyStore>);

    impl KeyStore for Shared {
        fn backend(&self) -> &'static str {
            "shared"
        }
        fn get(&self, n: &str) -> Result<Option<Secret>, PlatformError> {
            self.0.get(n)
        }
        fn put(&self, n: &str, v: &[u8]) -> Result<(), PlatformError> {
            self.0.put(n, v)
        }
        fn delete(&self, n: &str) -> Result<bool, PlatformError> {
            self.0.delete(n)
        }
    }

    #[test]
    fn behaves_like_a_key_store() {
        let dir = temp_dir("sealed");
        let store = SealedKeyStore::new(
            Box::new(MemoryKeyStore::new()),
            FileKeyStore::open(&dir).unwrap(),
        );
        super::super::tests::exercise(&store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_are_sealed_and_bound_to_their_name() {
        let dir = temp_dir("sealed-bind");
        let keys = Arc::new(MemoryKeyStore::new());
        let store = SealedKeyStore::new(
            Box::new(Shared(Arc::clone(&keys))),
            FileKeyStore::open(&dir).unwrap(),
        );
        store.put("device-key", b"top secret bytes").unwrap();
        let raw = std::fs::read(dir.join("device-key.secret")).unwrap();
        assert!(!raw.windows(16).any(|w| w == b"top secret bytes"));
        assert_eq!(keys.get(WRAPPING_KEY_NAME).unwrap().unwrap().len(), 32);

        // Reopening with the same wrapping key works.
        let again = SealedKeyStore::new(
            Box::new(Shared(Arc::clone(&keys))),
            FileKeyStore::open(&dir).unwrap(),
        );
        assert_eq!(
            again.get("device-key").unwrap().unwrap().expose(),
            b"top secret bytes"
        );

        // A file renamed to another secret's name fails authentication.
        std::fs::copy(dir.join("device-key.secret"), dir.join("other.secret")).unwrap();
        assert!(again.get("other").is_err());

        // Without the wrapping key the file cannot be opened.
        let lost = SealedKeyStore::new(
            Box::new(MemoryKeyStore::new()),
            FileKeyStore::open(&dir).unwrap(),
        );
        assert!(matches!(
            lost.get("device-key"),
            Err(PlatformError::Unavailable(_))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

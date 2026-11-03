// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stored structures: one version byte, then one CBOR data item.
//!
//! Storage keys and record layouts are owned by the engines; see
//! `OPEN-QUESTIONS.md` CORE-1.

use minicbor::{Decode, Encode};
use nestwatch_crypto::DeviceKey;

use crate::storage::{Storage, StorageError};

/// Version byte of every stored value written by this crate.
pub(crate) const FORMAT_VERSION: u8 = 1;

/// Encodes `value` as `FORMAT_VERSION || CBOR`.
pub(crate) fn encode<T: Encode<()>>(value: &T) -> Vec<u8> {
    let mut out = vec![FORMAT_VERSION];
    minicbor::encode(value, &mut out).expect("writing to a Vec cannot fail");
    out
}

/// Decodes a value written by [`encode`].
pub(crate) fn decode<T>(key: &str, bytes: &[u8]) -> Result<T, StorageError>
where
    T: for<'b> Decode<'b, ()>,
{
    let corrupt = |why: String| StorageError::Corrupt {
        key: key.to_owned(),
        why,
    };
    match bytes.split_first() {
        Some((&FORMAT_VERSION, rest)) => minicbor::decode(rest).map_err(|e| corrupt(e.to_string())),
        Some((v, _)) => Err(corrupt(format!("unknown version {v}"))),
        None => Err(corrupt("empty".into())),
    }
}

/// Loads and decodes `key`, `None` if absent.
pub(crate) fn load<T>(storage: &dyn Storage, key: &str) -> Result<Option<T>, StorageError>
where
    T: for<'b> Decode<'b, ()>,
{
    storage.get(key)?.map(|b| decode(key, &b)).transpose()
}

/// Encodes and stores `value` under `key`.
pub(crate) fn save<T: Encode<()>>(
    storage: &dyn Storage,
    key: &str,
    value: &T,
) -> Result<(), StorageError> {
    storage.put(key, &encode(value))
}

/// The stored device key (CRYPTO-6: the 32 raw secret bytes).
#[derive(Encode, Decode)]
struct DeviceKeyRecord {
    #[b(0)]
    secret: Vec<u8>,
}

pub(crate) fn load_or_create_device_key(storage: &dyn Storage) -> Result<DeviceKey, StorageError> {
    if let Some(rec) = load::<DeviceKeyRecord>(storage, crate::KEY_DEVICE)? {
        return DeviceKey::from_secret_slice(&rec.secret).map_err(|e| StorageError::Corrupt {
            key: crate::KEY_DEVICE.into(),
            why: e.to_string(),
        });
    }
    let key = DeviceKey::generate().map_err(|e| StorageError::Backend(e.to_string()))?;
    let rec = DeviceKeyRecord {
        secret: key.secret_bytes().to_vec(),
    };
    save(storage, crate::KEY_DEVICE, &rec)?;
    Ok(key)
}

/// A 32-byte public key from stored bytes.
pub(crate) fn public_key(
    key: &str,
    bytes: &[u8],
) -> Result<nestwatch_crypto::PublicKey, StorageError> {
    nestwatch_crypto::PublicKey::from_slice(bytes).map_err(|e| StorageError::Corrupt {
        key: key.to_owned(),
        why: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStorage;

    #[derive(Debug, PartialEq, Encode, Decode)]
    struct Sample {
        #[n(0)]
        a: u32,
        #[n(1)]
        b: String,
    }

    #[test]
    fn versioned_cbor_roundtrip() {
        let s = MemoryStorage::new();
        let v = Sample {
            a: 7,
            b: "x".into(),
        };
        save(&s, "k", &v).unwrap();
        let raw = s.get("k").unwrap().unwrap();
        assert_eq!(raw[0], FORMAT_VERSION);
        assert_eq!(load::<Sample>(&s, "k").unwrap(), Some(v));
        s.put("k", &[9, 0]).unwrap();
        assert!(matches!(
            load::<Sample>(&s, "k"),
            Err(StorageError::Corrupt { .. })
        ));
    }

    #[test]
    fn device_key_is_created_once() {
        let s = MemoryStorage::new();
        let a = load_or_create_device_key(&s).unwrap();
        let b = load_or_create_device_key(&s).unwrap();
        assert_eq!(a.public_key(), b.public_key());
    }
}

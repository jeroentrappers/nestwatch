// SPDX-License-Identifier: AGPL-3.0-or-later
//! OS credential stores through `keyring-core`: Secret Service (pure-Rust
//! D-Bus) on Linux and BSD, Keychain on macOS, Credential Manager on
//! Windows.
//!
//! Each [`OsKeyStore`] owns its store instance; no process-wide default
//! store is installed.

use std::fmt;
use std::sync::Arc;

use keyring_core::{CredentialStore, Error};

use super::{KeyStore, Secret, validate_name};
use crate::PlatformError;

/// Service name under which secrets are stored.
pub const SERVICE: &str = crate::APP_ID;

/// A secret store backed by the OS credential store.
pub struct OsKeyStore {
    store: Arc<CredentialStore>,
    service: String,
    kind: &'static str,
}

impl fmt::Debug for OsKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OsKeyStore({}, service {})", self.kind, self.service)
    }
}

impl OsKeyStore {
    /// Connects to the platform's credential store.
    pub fn open() -> Result<Self, PlatformError> {
        Self::open_with_service(SERVICE)
    }

    /// Connects using a custom service name (e.g. for tests).
    pub fn open_with_service(service: &str) -> Result<Self, PlatformError> {
        let (store, kind) = platform_store()?;
        Ok(Self {
            store,
            service: service.to_owned(),
            kind,
        })
    }

    fn entry(&self, name: &str) -> Result<keyring_core::Entry, PlatformError> {
        validate_name(name)?;
        self.store
            .build(&self.service, name, None)
            .map_err(map_error)
    }
}

#[allow(clippy::needless_return)]
fn platform_store() -> Result<(Arc<CredentialStore>, &'static str), PlatformError> {
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    {
        let s = zbus_secret_service_keyring_store::Store::new().map_err(map_error)?;
        return Ok((s, "Secret Service"));
    }
    #[cfg(target_os = "macos")]
    {
        let s = apple_native_keyring_store::keychain::Store::new().map_err(map_error)?;
        return Ok((s, "Keychain"));
    }
    #[cfg(target_os = "windows")]
    {
        let s = windows_native_keyring_store::Store::new().map_err(map_error)?;
        return Ok((s, "Credential Manager"));
    }
    #[allow(unreachable_code)]
    Err(PlatformError::Unsupported(
        "no OS credential store on this platform",
    ))
}

fn map_error(e: Error) -> PlatformError {
    match e {
        Error::NoStorageAccess(inner) => PlatformError::PermissionDenied(inner.to_string()),
        Error::PlatformFailure(inner) => PlatformError::Unavailable(inner.to_string()),
        Error::NoDefaultStore => PlatformError::Unavailable("no credential store".into()),
        Error::TooLong(attr, limit) => {
            PlatformError::Backend(format!("{attr} exceeds the store limit of {limit}"))
        }
        // Other variants may carry secret bytes (BadEncoding, BadDataFormat);
        // never format them.
        Error::BadEncoding(_) | Error::BadDataFormat(..) => {
            PlatformError::Backend("credential store returned malformed data".into())
        }
        other => {
            PlatformError::Backend(format!("credential store error: {}", variant_name(&other)))
        }
    }
}

fn variant_name(e: &Error) -> &'static str {
    match e {
        Error::BadStoreFormat(_) => "bad store format",
        Error::Invalid(..) => "invalid parameter",
        Error::Ambiguous(_) => "ambiguous entry",
        Error::NoEntry => "no entry",
        _ => "other",
    }
}

impl KeyStore for OsKeyStore {
    fn backend(&self) -> &'static str {
        self.kind
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError> {
        match self.entry(name)?.get_secret() {
            Ok(bytes) => Ok(Some(Secret::new(bytes))),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        self.entry(name)?.set_secret(value).map_err(map_error)
    }

    fn delete(&self, name: &str) -> Result<bool, PlatformError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(Error::NoEntry) => Ok(false),
            Err(e) => Err(map_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs an unlocked OS credential store (Secret Service, Keychain or Credential Manager)"]
    fn round_trips_through_the_os_store() {
        let store = OsKeyStore::open_with_service("org.nestwatch.Nestwatch.test").unwrap();
        super::super::tests::exercise(&store);
    }
}

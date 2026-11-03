// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device key pairs (protocol.md §2.1, security.md §5).

use core::fmt;

use curve25519_dalek::montgomery::MontgomeryPoint;
use zeroize::Zeroizing;

use crate::{DeviceId, KeyError, RngError};

/// Length of X25519 public and secret keys.
pub const KEY_LEN: usize = 32;

/// An X25519 public key: a device's long-term identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey([u8; KEY_LEN]);

impl PublicKey {
    /// Wraps 32 raw bytes.
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Parses a 32-byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        bytes
            .try_into()
            .map(Self)
            .map_err(|_| KeyError::InvalidLength)
    }

    /// The raw 32 bytes.
    pub const fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The device ID of this key (protocol.md §2.2).
    pub fn device_id(&self) -> DeviceId {
        DeviceId::of(self)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicKey(")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        f.write_str(")")
    }
}

impl From<[u8; KEY_LEN]> for PublicKey {
    fn from(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// A device's X25519 static key pair (protocol.md §2.1).
///
/// The secret is zeroised on drop and never shown by `Debug`. It leaves this
/// type only through [`DeviceKey::secret_bytes`], for the platform key store.
pub struct DeviceKey {
    secret: Zeroizing<[u8; KEY_LEN]>,
    public: PublicKey,
}

impl DeviceKey {
    /// Generates a new key pair from the system random number generator.
    pub fn generate() -> Result<Self, RngError> {
        let mut secret = Zeroizing::new([0u8; KEY_LEN]);
        crate::fill_random(secret.as_mut())?;
        Ok(Self::from_secret(secret))
    }

    /// Restores a key pair from secret bytes read from a key store.
    pub fn from_secret_bytes(secret: &[u8; KEY_LEN]) -> Self {
        Self::from_secret(Zeroizing::new(*secret))
    }

    /// Restores a key pair from a secret slice read from a key store.
    pub fn from_secret_slice(secret: &[u8]) -> Result<Self, KeyError> {
        let secret: &[u8; KEY_LEN] = secret.try_into().map_err(|_| KeyError::InvalidLength)?;
        Ok(Self::from_secret_bytes(secret))
    }

    fn from_secret(secret: Zeroizing<[u8; KEY_LEN]>) -> Self {
        let public = PublicKey(MontgomeryPoint::mul_base_clamped(*secret).to_bytes());
        Self { secret, public }
    }

    /// The secret bytes, for storing in a platform key store only.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; KEY_LEN]> {
        self.secret.clone()
    }

    pub(crate) fn secret(&self) -> &[u8; KEY_LEN] {
        &self.secret
    }

    /// The public key.
    pub const fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// The device ID of the public key (protocol.md §2.2).
    pub fn device_id(&self) -> DeviceId {
        self.public.device_id()
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceKey")
            .field("public", &self.public)
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
pub(crate) fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 7748 §6.1.
    const ALICE_SECRET: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
    const ALICE_PUBLIC: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";

    #[test]
    fn public_key_derivation_matches_rfc7748() {
        let secret: [u8; 32] = hex(ALICE_SECRET).try_into().unwrap();
        let key = DeviceKey::from_secret_bytes(&secret);
        assert_eq!(key.public_key().as_bytes().as_slice(), hex(ALICE_PUBLIC));
    }

    #[test]
    fn secret_round_trip() {
        let key = DeviceKey::generate().unwrap();
        let restored = DeviceKey::from_secret_slice(key.secret_bytes().as_slice()).unwrap();
        assert_eq!(key.public_key(), restored.public_key());
        assert_ne!(
            DeviceKey::generate().unwrap().public_key(),
            key.public_key()
        );
    }

    #[test]
    fn debug_redacts_secret() {
        let secret = [0xabu8; 32];
        let key = DeviceKey::from_secret_bytes(&secret);
        let text = format!("{key:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("abababab"));
    }

    #[test]
    fn slice_lengths_are_checked() {
        assert_eq!(
            PublicKey::from_slice(&[0; 31]),
            Err(KeyError::InvalidLength)
        );
        assert!(DeviceKey::from_secret_slice(&[0; 33]).is_err());
        assert!(PublicKey::from_slice(&[0; 32]).is_ok());
    }
}

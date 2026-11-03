// SPDX-License-Identifier: AGPL-3.0-or-later
//! Invite tokens and the invite PSK (protocol.md §3.5, §5.2).

use core::fmt;

use zeroize::Zeroizing;

use crate::kdf::hkdf_sha256;
use crate::{KeyError, PSK_LEN, Psk, PublicKey, RngError};

/// Length of an invite token.
pub const INVITE_TOKEN_LEN: usize = 16;

const INFO: &[u8] = b"nestwatch/1 invite psk";

/// The 16-byte random token carried in an invitation (`t`). It is secret
/// while pairing mode is open; zeroised on drop, redacted in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct InviteToken(Zeroizing<[u8; INVITE_TOKEN_LEN]>);

impl InviteToken {
    /// Generates a token from the system random number generator.
    pub fn generate() -> Result<Self, RngError> {
        let mut token = Zeroizing::new([0u8; INVITE_TOKEN_LEN]);
        crate::fill_random(token.as_mut())?;
        Ok(Self(token))
    }

    /// Wraps token bytes, for example decoded from an invitation URI.
    pub fn from_bytes(bytes: &[u8; INVITE_TOKEN_LEN]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// Parses a 16-byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        let bytes: &[u8; INVITE_TOKEN_LEN] =
            bytes.try_into().map_err(|_| KeyError::InvalidLength)?;
        Ok(Self::from_bytes(bytes))
    }

    /// The raw token, for encoding into an invitation.
    pub fn as_bytes(&self) -> &[u8; INVITE_TOKEN_LEN] {
        &self.0
    }

    /// `HKDF-SHA256(ikm = token, salt = station_public_key,
    /// info = "nestwatch/1 invite psk", L = 32)`.
    pub fn psk(&self, station_public_key: &PublicKey) -> Psk {
        let mut out = Zeroizing::new([0u8; PSK_LEN]);
        hkdf_sha256(
            self.0.as_slice(),
            station_public_key.as_bytes(),
            INFO,
            out.as_mut(),
        );
        Psk::new(out)
    }
}

impl fmt::Debug for InviteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InviteToken(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psk_depends_on_token_and_station_key() {
        let a = InviteToken::from_bytes(&[1; 16]);
        let b = InviteToken::from_bytes(&[2; 16]);
        let s1 = PublicKey::from_bytes([3; 32]);
        let s2 = PublicKey::from_bytes([4; 32]);
        assert_eq!(a.psk(&s1), a.psk(&s1));
        assert_ne!(a.psk(&s1), b.psk(&s1));
        assert_ne!(a.psk(&s1), a.psk(&s2));
    }

    #[test]
    fn generated_tokens_differ_and_debug_redacts() {
        let a = InviteToken::generate().unwrap();
        assert_ne!(a, InviteToken::generate().unwrap());
        assert_eq!(format!("{a:?}"), "InviteToken(<redacted>)");
        assert!(InviteToken::from_slice(&[0; 15]).is_err());
    }
}

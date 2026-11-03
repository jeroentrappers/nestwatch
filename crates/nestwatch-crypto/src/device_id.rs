// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device IDs (protocol.md §2.2).

use core::fmt;
use core::str::FromStr;

use sha2::{Digest, Sha256};

use crate::{DeviceIdError, PublicKey};

/// Length of a device ID in characters (canonical form, without hyphens).
pub const DEVICE_ID_LEN: usize = 16;

const DOMAIN: &[u8] = b"nestwatch/1 device id";
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// `base32(SHA-256("nestwatch/1 device id" || public_key)[0..10])`, RFC 4648
/// alphabet in lower case without padding.
///
/// `Display` and [`DeviceId::as_str`] give the canonical 16-character form
/// used in discovery records; [`DeviceId::grouped`] gives the form shown to
/// users (`abcd-efgh-ijkl-mnop`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId([u8; DEVICE_ID_LEN]);

impl DeviceId {
    /// Computes the device ID of a public key.
    pub fn of(public_key: &PublicKey) -> Self {
        let digest = Sha256::new()
            .chain_update(DOMAIN)
            .chain_update(public_key.as_bytes())
            .finalize();
        Self(encode(digest[..10].try_into().expect("10-byte slice")))
    }

    /// Whether `public_key` hashes to this ID.
    pub fn matches(&self, public_key: &PublicKey) -> bool {
        Self::of(public_key) == *self
    }

    /// Parses a device ID in canonical or grouped form. Hyphens are ignored
    /// and letters may be upper or lower case.
    pub fn parse(text: &str) -> Result<Self, DeviceIdError> {
        let mut out = [0u8; DEVICE_ID_LEN];
        let mut len = 0;
        for c in text.bytes().filter(|&c| c != b'-') {
            let c = c.to_ascii_lowercase();
            if !ALPHABET.contains(&c) {
                return Err(DeviceIdError::InvalidCharacter);
            }
            if len == DEVICE_ID_LEN {
                return Err(DeviceIdError::InvalidLength);
            }
            out[len] = c;
            len += 1;
        }
        if len != DEVICE_ID_LEN {
            return Err(DeviceIdError::InvalidLength);
        }
        Ok(Self(out))
    }

    /// The canonical form: 16 lower-case base32 characters.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).expect("base32 alphabet is ASCII")
    }

    /// The canonical form as ASCII bytes (used in the code salt, §5.3).
    pub const fn as_bytes(&self) -> &[u8; DEVICE_ID_LEN] {
        &self.0
    }

    /// The grouped form shown to users, `abcd-efgh-ijkl-mnop`.
    pub fn grouped(&self) -> String {
        let s = self.as_str();
        format!("{}-{}-{}-{}", &s[0..4], &s[4..8], &s[8..12], &s[12..16])
    }
}

/// RFC 4648 base32 of exactly 10 bytes (80 bits, no padding needed).
fn encode(bytes: [u8; 10]) -> [u8; DEVICE_ID_LEN] {
    let mut out = [0u8; DEVICE_ID_LEN];
    for (block, chunk) in bytes.chunks_exact(5).enumerate() {
        let v = chunk.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        for i in 0..8 {
            out[block * 8 + i] = ALPHABET[((v >> (35 - 5 * i)) & 31) as usize];
        }
    }
    out
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self.as_str())
    }
}

impl FromStr for DeviceId {
    type Err = DeviceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::hex;

    fn pk(h: &str) -> PublicKey {
        PublicKey::from_slice(&hex(h)).unwrap()
    }

    #[test]
    fn known_answers() {
        let cases = [
            (
                "0000000000000000000000000000000000000000000000000000000000000000",
                "jxgvemvdl4fipfni",
            ),
            (
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "5oohnb4erchflnxs",
            ),
            (
                "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
                "fe7cgpjgrn5h665a",
            ),
        ];
        for (key, id) in cases {
            let computed = pk(key).device_id();
            assert_eq!(computed.as_str(), id);
            assert_eq!(computed.to_string(), id);
            assert!(computed.matches(&pk(key)));
        }
    }

    #[test]
    fn rfc4648_encoding() {
        // RFC 4648 §10: BASE32("fooba") = "MZXW6YTB".
        assert_eq!(&encode(*b"foobafooba"), b"mzxw6ytbmzxw6ytb");
        assert_eq!(&encode([0xff; 10]), b"7777777777777777");
    }

    #[test]
    fn grouped_form_and_parsing() {
        let id = pk("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a").device_id();
        assert_eq!(id.grouped(), "fe7c-gpjg-rn5h-665a");
        assert_eq!(DeviceId::parse("fe7c-gpjg-rn5h-665a"), Ok(id));
        assert_eq!("FE7CGPJGRN5H665A".parse::<DeviceId>(), Ok(id));
        assert_eq!(DeviceId::parse(id.as_str()), Ok(id));
    }

    #[test]
    fn invalid_ids_are_rejected() {
        assert_eq!(DeviceId::parse(""), Err(DeviceIdError::InvalidLength));
        assert_eq!(
            DeviceId::parse("2r5u2qrs5oqjtnb"),
            Err(DeviceIdError::InvalidLength)
        );
        assert_eq!(
            DeviceId::parse("2r5u2qrs5oqjtnbqa"),
            Err(DeviceIdError::InvalidLength)
        );
        assert_eq!(
            DeviceId::parse("2r5u2qrs5oqjtnb1"),
            Err(DeviceIdError::InvalidCharacter)
        );
        assert_eq!(
            DeviceId::parse("2r5u 2qrs5oqjtnb"),
            Err(DeviceIdError::InvalidCharacter)
        );
        assert_eq!(
            DeviceId::parse("2r5u2qrs5oqjtnb\u{e9}"),
            Err(DeviceIdError::InvalidCharacter)
        );
    }
}

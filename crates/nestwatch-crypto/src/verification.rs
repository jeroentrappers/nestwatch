// SPDX-License-Identifier: AGPL-3.0-or-later
//! Verification number shown after pairing (protocol.md §5.5).

use core::fmt;

/// `u32(h[0..4]) mod 1_000_000`; `Display` gives `042 917`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerificationNumber(u32);

impl VerificationNumber {
    /// Derives the number from a Noise handshake hash.
    pub fn from_handshake_hash(h: &[u8; 32]) -> Self {
        Self(u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000)
    }

    /// The number, `0..1_000_000`.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for VerificationNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(prefix: [u8; 4]) -> [u8; 32] {
        let mut h = [0xee; 32];
        h[..4].copy_from_slice(&prefix);
        h
    }

    #[test]
    fn known_values() {
        let cases = [
            ([0, 0, 0, 0], 0, "000 000"),
            ([0, 0, 0xa4, 0x35], 42037, "042 037"),
            ([0x00, 0x0f, 0x42, 0x3f], 999_999, "999 999"),
            ([0x00, 0x0f, 0x42, 0x40], 0, "000 000"),
            ([0xff, 0xff, 0xff, 0xff], 967_295, "967 295"),
        ];
        for (prefix, value, text) in cases {
            let n = VerificationNumber::from_handshake_hash(&hash(prefix));
            assert_eq!(n.value(), value);
            assert_eq!(n.to_string(), text);
        }
    }
}

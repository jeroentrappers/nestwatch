// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pairing codes and the code PSK (protocol.md §5.3, security.md §4).

use core::fmt;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::kdf::{CODE_ARGON2, argon2id};
use crate::{DeviceId, PSK_LEN, PairingCodeError, Psk, RngError};

/// Length of a normalised pairing code.
pub const CODE_LEN: usize = 10;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const SALT_DOMAIN: &[u8] = b"nestwatch/1 code salt";

/// A normalised pairing code: 10 Crockford base32 characters, 50 bits.
///
/// Secret while pairing mode is open: zeroised on drop, redacted in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode(Zeroizing<[u8; CODE_LEN]>);

impl PairingCode {
    /// Generates a uniformly random code from the system random number
    /// generator.
    pub fn generate() -> Result<Self, RngError> {
        let mut raw = Zeroizing::new([0u8; 8]);
        crate::fill_random(raw.as_mut())?;
        let bits = Zeroizing::new(u64::from_be_bytes(*raw) & ((1 << 50) - 1));
        let mut code = Zeroizing::new([0u8; CODE_LEN]);
        for (i, c) in code.iter_mut().enumerate() {
            *c = ALPHABET[((*bits >> (45 - 5 * i)) & 31) as usize];
        }
        Ok(Self(code))
    }

    /// Normalises user input: removes spaces and hyphens, upper-cases, maps
    /// `O→0`, `I→1`, `L→1`, then requires exactly 10 alphabet characters.
    pub fn parse(input: &str) -> Result<Self, PairingCodeError> {
        let mut code = Zeroizing::new([0u8; CODE_LEN]);
        let mut len = 0;
        for c in input.bytes().filter(|&c| c != b' ' && c != b'-') {
            let c = match c.to_ascii_uppercase() {
                b'O' => b'0',
                b'I' | b'L' => b'1',
                c => c,
            };
            if !ALPHABET.contains(&c) {
                return Err(PairingCodeError::InvalidCharacter);
            }
            if len == CODE_LEN {
                return Err(PairingCodeError::InvalidLength);
            }
            code[len] = c;
            len += 1;
        }
        if len != CODE_LEN {
            return Err(PairingCodeError::InvalidLength);
        }
        Ok(Self(code))
    }

    /// The normalised code, 10 characters without separator.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.0.as_slice()).expect("alphabet is ASCII")
    }

    /// The display form `XXXXX-XXXXX`.
    pub fn display(&self) -> Zeroizing<String> {
        let s = self.as_str();
        Zeroizing::new(format!("{}-{}", &s[..5], &s[5..]))
    }

    /// `SHA-256("nestwatch/1 code salt" || station_device_id_ascii)[0..16]`.
    pub(crate) fn salt(station_id: &DeviceId) -> [u8; 16] {
        let digest = Sha256::new()
            .chain_update(SALT_DOMAIN)
            .chain_update(station_id.as_bytes())
            .finalize();
        digest[..16].try_into().expect("16-byte slice")
    }

    /// `Argon2id(password = code, salt, m = 19456 KiB, t = 2, p = 1,
    /// length = 32)`. Deliberately slow (tens of milliseconds optimised).
    pub fn psk(&self, station_id: &DeviceId) -> Psk {
        let mut out = Zeroizing::new([0u8; PSK_LEN]);
        argon2id(
            self.0.as_slice(),
            &Self::salt(station_id),
            &[],
            &[],
            CODE_ARGON2,
            out.as_mut(),
        );
        Psk::new(out)
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_table() {
        let ok = [
            ("7K3QD-9XW2M", "7K3QD9XW2M"),
            ("7k3qd9xw2m", "7K3QD9XW2M"),
            (" 7k3qd - 9xw2m ", "7K3QD9XW2M"),
            ("7-k-3-q-d-9-x-w-2-m", "7K3QD9XW2M"),
            ("OoIiLl0011", "0011110011"),
            ("abcde-fghjk", "ABCDEFGHJK"),
            ("mnpqr-stvwx", "MNPQRSTVWX"),
            ("yz012-34567", "YZ01234567"),
        ];
        for (input, normalised) in ok {
            assert_eq!(
                PairingCode::parse(input).unwrap().as_str(),
                normalised,
                "{input}"
            );
        }
        let bad = [
            ("", PairingCodeError::InvalidLength),
            ("7K3QD-9XW2", PairingCodeError::InvalidLength),
            ("7K3QD-9XW2MM", PairingCodeError::InvalidLength),
            ("7K3QD-9XW2U", PairingCodeError::InvalidCharacter),
            ("7K3QD_9XW2M", PairingCodeError::InvalidCharacter),
            ("7K3QD\t9XW2M", PairingCodeError::InvalidCharacter),
            ("7K3QD.9XW2M", PairingCodeError::InvalidCharacter),
            ("7K3QD9XW2\u{d8}", PairingCodeError::InvalidCharacter),
        ];
        for (input, err) in bad {
            assert_eq!(PairingCode::parse(input), Err(err), "{input:?}");
        }
    }

    #[test]
    fn display_form() {
        let code = PairingCode::parse("7k3qd9xw2m").unwrap();
        assert_eq!(code.display().as_str(), "7K3QD-9XW2M");
        assert_eq!(format!("{code:?}"), "PairingCode(<redacted>)");
    }

    #[test]
    fn generated_codes_are_valid_and_use_the_whole_alphabet() {
        let mut seen = [false; 32];
        for _ in 0..200 {
            let code = PairingCode::generate().unwrap();
            assert_eq!(PairingCode::parse(&code.display()).unwrap(), code);
            for c in code.as_str().bytes() {
                seen[ALPHABET.iter().position(|&a| a == c).unwrap()] = true;
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn psk_depends_on_code_and_station() {
        let id_a = DeviceId::parse("2r5u2qrs5oqjtnbq").unwrap();
        let id_b = DeviceId::parse("bh6u44sz3tdfitae").unwrap();
        let code = PairingCode::parse("7K3QD-9XW2M").unwrap();
        let other = PairingCode::parse("7K3QD-9XW2N").unwrap();
        let psk = code.psk(&id_a);
        assert_eq!(psk, PairingCode::parse("7k3qd 9xw2m").unwrap().psk(&id_a));
        assert_ne!(psk, other.psk(&id_a));
        assert_ne!(psk, code.psk(&id_b));
    }
}

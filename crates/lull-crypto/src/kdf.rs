// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pre-shared keys and the key derivation functions behind them
//! (protocol.md §5.2, §5.3).

use core::fmt;

use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// Length of a Noise pre-shared key.
pub const PSK_LEN: usize = 32;

/// A Noise pre-shared key for the `invite` or `code` mode. Zeroised on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Psk(Zeroizing<[u8; PSK_LEN]>);

impl Psk {
    pub(crate) fn new(bytes: Zeroizing<[u8; PSK_LEN]>) -> Self {
        Self(bytes)
    }

    /// The raw key bytes.
    pub fn as_bytes(&self) -> &[u8; PSK_LEN] {
        &self.0
    }
}

impl fmt::Debug for Psk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Psk(<redacted>)")
    }
}

/// HKDF-SHA256 (RFC 5869) into `out`.
pub(crate) fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], out: &mut [u8]) {
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, out)
        .expect("output length within HKDF-SHA256 limit");
}

/// Argon2 cost parameters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Argon2Params {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

/// The code PSK parameters of protocol.md §5.3.
pub(crate) const CODE_ARGON2: Argon2Params = Argon2Params {
    m_kib: 19456,
    t: 2,
    p: 1,
};

/// Argon2id version 0x13 (RFC 9106) into `out`.
pub(crate) fn argon2id(
    password: &[u8],
    salt: &[u8],
    secret: &[u8],
    associated_data: &[u8],
    params: Argon2Params,
    out: &mut [u8],
) {
    let params = ParamsBuilder::new()
        .m_cost(params.m_kib)
        .t_cost(params.t)
        .p_cost(params.p)
        .output_len(out.len())
        .data(AssociatedData::new(associated_data).expect("associated data within limit"))
        .build()
        .expect("valid Argon2 parameters");
    let argon = if secret.is_empty() {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    } else {
        Argon2::new_with_secret(secret, Algorithm::Argon2id, Version::V0x13, params)
            .expect("secret within limit")
    };
    argon
        .hash_password_into(password, salt, out)
        .expect("valid Argon2 inputs");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::hex;

    #[test]
    fn hkdf_rfc5869_test_case_1() {
        let ikm = [0x0b; 22];
        let salt = hex("000102030405060708090a0b0c");
        let info = hex("f0f1f2f3f4f5f6f7f8f9");
        let mut okm = [0u8; 42];
        hkdf_sha256(&ikm, &salt, &info, &mut okm);
        assert_eq!(
            okm.as_slice(),
            hex(
                "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
            )
        );
    }

    #[test]
    fn argon2id_rfc9106_section_5_3() {
        let mut tag = [0u8; 32];
        argon2id(
            &[0x01; 32],
            &[0x02; 16],
            &[0x03; 8],
            &[0x04; 12],
            Argon2Params {
                m_kib: 32,
                t: 3,
                p: 4,
            },
            &mut tag,
        );
        assert_eq!(
            tag.as_slice(),
            hex("0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659")
        );
    }

    #[test]
    fn psk_debug_is_redacted() {
        let psk = Psk::new(Zeroizing::new([0x42; 32]));
        assert_eq!(format!("{psk:?}"), "Psk(<redacted>)");
    }
}

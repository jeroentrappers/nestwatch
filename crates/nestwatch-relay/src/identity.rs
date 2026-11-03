// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mailbox identity of a station (rendezvous.md §3.2).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::msg::{SECRET_LEN, Secret};
use nestwatch_proto::invite::MAILBOX_ID_LEN;

/// A 16-byte mailbox ID. It appears in invitations (`m=`), so it is not
/// secret, but `Debug` hides it anyway so it never ends up in logs by
/// accident (rendezvous.md §6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MailboxId(pub [u8; MAILBOX_ID_LEN]);

impl MailboxId {
    /// Raw bytes.
    pub fn as_bytes(&self) -> &[u8; MAILBOX_ID_LEN] {
        &self.0
    }

    /// base64url without padding, as in invitations.
    pub fn to_text(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// Parses the base64url form.
    pub fn parse(text: &str) -> Option<Self> {
        let b = URL_SAFE_NO_PAD.decode(text.trim()).ok()?;
        Some(Self(b.try_into().ok()?))
    }
}

impl fmt::Debug for MailboxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MailboxId(..)")
    }
}

impl From<[u8; MAILBOX_ID_LEN]> for MailboxId {
    fn from(b: [u8; MAILBOX_ID_LEN]) -> Self {
        Self(b)
    }
}

/// `SHA-256(mailbox_secret)`, what the server stores (TOFU, §3.2).
pub fn secret_hash(secret: &[u8; SECRET_LEN]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

/// Errors loading a stored [`MailboxIdentity`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// Wrong length or unknown format version.
    #[error("malformed mailbox identity")]
    Malformed,
    /// The system random generator failed.
    #[error("random generator failed")]
    Random,
}

/// Format version of [`MailboxIdentity::to_bytes`].
const FORMAT: u8 = 1;
/// Length of [`MailboxIdentity::to_bytes`].
pub const IDENTITY_BYTES_LEN: usize = 1 + 4 + MAILBOX_ID_LEN + SECRET_LEN;

/// A station's mailbox: ID, secret and a generation counter that increases
/// with every "reset relay identity" (§3.2). The secret never leaves the
/// station except in `register`.
#[derive(Clone, PartialEq, Eq)]
pub struct MailboxIdentity {
    id: MailboxId,
    secret: [u8; SECRET_LEN],
    generation: u32,
}

impl fmt::Debug for MailboxIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailboxIdentity")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

fn random<const N: usize>() -> Result<[u8; N], IdentityError> {
    let mut b = [0; N];
    getrandom::fill(&mut b).map_err(|_| IdentityError::Random)?;
    Ok(b)
}

impl MailboxIdentity {
    /// A fresh random identity, generation 0. Created when the user first
    /// enables the relay; kept when it is disabled and re-enabled.
    pub fn generate() -> Result<Self, IdentityError> {
        Ok(Self {
            id: MailboxId(random()?),
            secret: random()?,
            generation: 0,
        })
    }

    /// From explicit parts (tests, migrations).
    pub fn from_parts(id: MailboxId, secret: [u8; SECRET_LEN], generation: u32) -> Self {
        Self {
            id,
            secret,
            generation,
        }
    }

    /// "Reset relay identity": a new random ID and secret with the next
    /// generation. Listeners must be given a new invitation.
    pub fn reset(&self) -> Result<Self, IdentityError> {
        Ok(Self {
            generation: self.generation.wrapping_add(1),
            ..Self::generate()?
        })
    }

    /// The mailbox ID.
    pub fn id(&self) -> MailboxId {
        self.id
    }

    /// The secret, for `register`.
    pub fn secret(&self) -> Secret {
        Secret(self.secret)
    }

    /// The generation counter.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// `SHA-256(secret)`.
    pub fn secret_hash(&self) -> [u8; 32] {
        secret_hash(&self.secret)
    }

    /// Storage form: `1 || generation (u32) || id (16) || secret (32)`, 53
    /// bytes. Keep it in the platform's key storage (security.md §5).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(IDENTITY_BYTES_LEN);
        out.push(FORMAT);
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.id.0);
        out.extend_from_slice(&self.secret);
        out
    }

    /// Parses [`to_bytes`](Self::to_bytes).
    pub fn from_bytes(b: &[u8]) -> Result<Self, IdentityError> {
        if b.len() != IDENTITY_BYTES_LEN || b[0] != FORMAT {
            return Err(IdentityError::Malformed);
        }
        let generation = u32::from_be_bytes(b[1..5].try_into().expect("4 bytes"));
        let id = MailboxId(b[5..21].try_into().expect("16 bytes"));
        let secret = b[21..].try_into().expect("32 bytes");
        Ok(Self {
            id,
            secret,
            generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_round_trip() {
        let a = MailboxIdentity::generate().unwrap();
        let b = MailboxIdentity::from_bytes(&a.to_bytes()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_bytes().len(), IDENTITY_BYTES_LEN);
        assert_eq!(
            MailboxIdentity::from_bytes(&[1; 10]),
            Err(IdentityError::Malformed)
        );
        let mut v2 = a.to_bytes();
        v2[0] = 2;
        assert_eq!(
            MailboxIdentity::from_bytes(&v2),
            Err(IdentityError::Malformed)
        );
    }

    #[test]
    fn reset_changes_everything_but_counts_up() {
        let a = MailboxIdentity::from_parts(MailboxId([1; 16]), [2; 32], 7);
        let b = a.reset().unwrap();
        assert_ne!(a.id(), b.id());
        assert_ne!(a.secret(), b.secret());
        assert_eq!(b.generation(), 8);
        assert_eq!(&a.to_bytes()[..5], &[1, 0, 0, 0, 7]);
    }

    #[test]
    fn secret_hash_is_sha256() {
        // SHA-256 of 32 zero bytes.
        let h = secret_hash(&[0; 32]);
        assert_eq!(
            h[..4],
            [0x66, 0x68, 0x7a, 0xad],
            "SHA-256(0^32) starts with 66687aad"
        );
    }

    #[test]
    fn text_form_and_redaction() {
        let id = MailboxId(core::array::from_fn(|i| i as u8 * 3));
        assert_eq!(id.to_text(), "AAMGCQwPEhUYGx4hJCcqLQ");
        assert_eq!(MailboxId::parse(&id.to_text()), Some(id));
        assert_eq!(MailboxId::parse("AAAA"), None);
        assert_eq!(format!("{id:?}"), "MailboxId(..)");
        let ident = MailboxIdentity::from_parts(id, [9; 32], 1);
        assert!(!format!("{ident:?}").contains("9, 9"));
    }
}

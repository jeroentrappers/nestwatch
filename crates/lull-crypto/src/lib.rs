// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device identity, key derivation and Noise handshakes for LWP/1.
//!
//! Sans-I/O: this crate never touches sockets, clocks or threads. It turns
//! keys and handshake messages into further handshake messages and, finally,
//! a [`Transport`] for encrypting records (`specs/protocol.md` §2, §5).

mod code;
mod device_id;
mod error;
mod handshake;
mod invite;
mod kdf;
mod keys;
mod transport;
mod verification;

#[cfg(test)]
mod tests;

pub use code::{CODE_LEN, PairingCode};
pub use device_id::{DEVICE_ID_LEN, DeviceId};
pub use error::{
    DeviceIdError, HandshakeError, KeyError, PairingCodeError, RngError, TransportError,
};
pub use handshake::{Completion, Initiator, Mode, PROLOGUE_LEN, Prologue, Responder, Step};
pub use invite::{INVITE_TOKEN_LEN, InviteToken};
pub use kdf::{PSK_LEN, Psk};
pub use keys::{DeviceKey, KEY_LEN, PublicKey};
pub use transport::{MAX_MESSAGE_LEN, MAX_PLAINTEXT_LEN, NONCE_LIMIT, Transport};
pub use verification::VerificationNumber;

/// Fills `buf` from the operating system's random number generator.
pub(crate) fn fill_random(buf: &mut [u8]) -> Result<(), RngError> {
    getrandom::fill(buf).map_err(|_| RngError)
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transport phase: encrypted records (protocol.md §5.6, §6).

use core::fmt;

use snow::TransportState;

use crate::{Mode, PublicKey, TransportError, VerificationNumber};

/// Largest Noise message, and so largest frame payload (protocol.md §3.1).
pub const MAX_MESSAGE_LEN: usize = 65535;
/// Largest plaintext per transport message (message minus 16-byte tag).
pub const MAX_PLAINTEXT_LEN: usize = MAX_MESSAGE_LEN - TAG_LEN;
/// Messages per direction: nonces `0..NONCE_LIMIT` may be used, after that
/// the session must end (protocol.md §5.6).
pub const NONCE_LIMIT: u64 = 1 << 32;

const TAG_LEN: usize = 16;

/// Noise transport state for one session.
///
/// A decryption failure ends the session: every later call then fails with
/// [`TransportError::Closed`].
pub struct Transport {
    state: Box<TransportState>,
    mode: Mode,
    remote_static: PublicKey,
    handshake_hash: [u8; 32],
    sending_nonce: u64,
    receiving_nonce: u64,
    closed: bool,
}

impl Transport {
    pub(crate) fn new(
        state: TransportState,
        mode: Mode,
        remote_static: PublicKey,
        handshake_hash: [u8; 32],
    ) -> Self {
        Self {
            state: Box::new(state),
            mode,
            remote_static,
            handshake_hash,
            sending_nonce: 0,
            receiving_nonce: 0,
            closed: false,
        }
    }

    /// The mode of the handshake that produced this transport.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The peer's authenticated static key.
    pub fn remote_static(&self) -> &PublicKey {
        &self.remote_static
    }

    /// The Noise handshake hash `h`.
    pub fn handshake_hash(&self) -> &[u8; 32] {
        &self.handshake_hash
    }

    /// The verification number for pairing modes (protocol.md §5.5); `None`
    /// in `trusted` mode.
    pub fn verification_number(&self) -> Option<VerificationNumber> {
        self.mode
            .is_pairing()
            .then(|| VerificationNumber::from_handshake_hash(&self.handshake_hash))
    }

    /// Nonce of the next outgoing message (messages sent so far).
    pub fn sending_nonce(&self) -> u64 {
        self.sending_nonce
    }

    /// Nonce of the next incoming message (messages received so far).
    pub fn receiving_nonce(&self) -> u64 {
        self.receiving_nonce
    }

    /// Encrypts one record into a Noise transport message.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if plaintext.len() > MAX_PLAINTEXT_LEN {
            return Err(TransportError::MessageTooLarge);
        }
        if self.sending_nonce >= NONCE_LIMIT {
            return Err(TransportError::NonceExhausted);
        }
        let mut out = vec![0u8; plaintext.len() + TAG_LEN];
        let n = self.state.write_message(plaintext, &mut out).map_err(|_| {
            self.closed = true;
            TransportError::Closed
        })?;
        out.truncate(n);
        self.sending_nonce += 1;
        Ok(out)
    }

    /// Decrypts one Noise transport message into its record.
    pub fn decrypt(&mut self, message: &[u8]) -> Result<Vec<u8>, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if message.len() > MAX_MESSAGE_LEN {
            self.closed = true;
            return Err(TransportError::MessageTooLarge);
        }
        if self.receiving_nonce >= NONCE_LIMIT {
            return Err(TransportError::NonceExhausted);
        }
        let mut out = vec![0u8; message.len()];
        match self.state.read_message(message, &mut out) {
            Ok(n) => {
                out.truncate(n);
                self.receiving_nonce += 1;
                Ok(out)
            }
            Err(_) => {
                self.closed = true;
                Err(TransportError::Decrypt)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_nonces_for_test(&mut self, sending: u64, receiving: u64) {
        self.sending_nonce = sending;
        self.receiving_nonce = receiving;
    }
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("mode", &self.mode)
            .field("remote_static", &self.remote_static)
            .field("sending_nonce", &self.sending_nonce)
            .field("receiving_nonce", &self.receiving_nonce)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

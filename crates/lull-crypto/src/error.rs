// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types. None of them carries key material, PSKs, codes or plaintext.

use thiserror::Error;

/// The system random number generator failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("system random number generator unavailable")]
pub struct RngError;

/// A key had the wrong encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum KeyError {
    /// X25519 keys are exactly 32 bytes.
    #[error("key must be 32 bytes")]
    InvalidLength,
}

/// A device ID string was malformed (protocol.md §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DeviceIdError {
    /// Not 16 base32 characters after removing hyphens.
    #[error("device ID must be 16 base32 characters")]
    InvalidLength,
    /// A character outside the RFC 4648 base32 alphabet.
    #[error("device ID contains a character outside the base32 alphabet")]
    InvalidCharacter,
}

/// A pairing code was malformed (protocol.md §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PairingCodeError {
    /// Not 10 characters after normalisation.
    #[error("pairing code must be 10 characters")]
    InvalidLength,
    /// A character outside the Crockford base32 alphabet.
    #[error("pairing code contains an invalid character")]
    InvalidCharacter,
}

/// A handshake failed. Every variant means the connection must be closed;
/// a station closes silently whatever the cause (protocol.md §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum HandshakeError {
    /// A method was called that does not fit the current handshake step,
    /// or the handshake already failed.
    #[error("handshake method called in the wrong state")]
    InvalidState,
    /// A handshake message exceeded the Noise limit of 65535 bytes.
    #[error("handshake message too large")]
    MessageTooLarge,
    /// A handshake message was malformed or failed authentication
    /// (wrong key, wrong PSK, prologue mismatch or tampering).
    #[error("handshake message rejected")]
    Rejected,
    /// A handshake message carried a payload (protocol.md §5.4).
    #[error("handshake payload must be empty")]
    NonEmptyPayload,
    /// In `code` mode the station's static key does not hash to the
    /// expected device ID (protocol.md §5.3).
    #[error("station key does not match the expected device ID")]
    DeviceIdMismatch,
    /// The random number generator failed while creating an ephemeral key.
    #[error("system random number generator unavailable")]
    Rng,
}

/// A transport operation failed (protocol.md §5.6, security.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TransportError {
    /// Plaintext longer than [`crate::MAX_PLAINTEXT_LEN`] or ciphertext longer
    /// than [`crate::MAX_MESSAGE_LEN`].
    #[error("transport message too large")]
    MessageTooLarge,
    /// The nonce for this direction reached 2^32; the session must end.
    #[error("transport nonce limit reached")]
    NonceExhausted,
    /// A message failed authentication; the session must end.
    #[error("transport message failed authentication")]
    Decrypt,
    /// The transport is unusable after an earlier decryption failure.
    #[error("transport closed after an earlier failure")]
    Closed,
}

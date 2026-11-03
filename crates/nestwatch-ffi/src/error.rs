// SPDX-License-Identifier: AGPL-3.0-or-later
//! Errors crossing the FFI (Kotlin: `NestwatchException`, Swift: `NestwatchError`).

use nestwatch_core::StorageError;
use nestwatch_core::listener::ListenerError;
use nestwatch_core::station::StationError;

/// Errors of engine calls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum NestwatchError {
    /// The storage backend failed or holds an invalid value.
    #[error("storage: {reason}")]
    Storage {
        /// What failed.
        reason: String,
    },
    /// Input could not be parsed (invitation, code, ID, address).
    #[error("invalid input: {reason}")]
    Invalid {
        /// What was wrong.
        reason: String,
    },
    /// No route reached the station.
    #[error("unreachable: {reason}")]
    Unreachable {
        /// Diagnostics.
        reason: String,
    },
    /// The station refused: pairing closed, busy or rate limited.
    #[error("refused: {reason}")]
    Refused {
        /// Diagnostics.
        reason: String,
    },
    /// The pairing handshake failed: wrong code, expired invitation or a
    /// different station.
    #[error("rejected: {reason}")]
    Rejected {
        /// Diagnostics.
        reason: String,
    },
    /// No such station.
    #[error("unknown station")]
    UnknownStation,
    /// Already following 8 stations (FR-LS-1).
    #[error("too many stations")]
    TooManyStations,
    /// The station is not connected.
    #[error("not connected")]
    NotConnected,
    /// The engine was stopped.
    #[error("stopped")]
    Stopped,
    /// Binding ports or serving failed.
    #[error("network: {reason}")]
    Network {
        /// Diagnostics.
        reason: String,
    },
}

impl From<ListenerError> for NestwatchError {
    fn from(e: ListenerError) -> Self {
        match e {
            ListenerError::Storage(s) => Self::Storage {
                reason: s.to_string(),
            },
            ListenerError::Invalid(reason) => Self::Invalid { reason },
            ListenerError::Unreachable(reason) => Self::Unreachable { reason },
            ListenerError::Refused(reason) => Self::Refused { reason },
            ListenerError::Rejected(reason) => Self::Rejected { reason },
            ListenerError::UnknownStation => Self::UnknownStation,
            ListenerError::TooManyStations => Self::TooManyStations,
            ListenerError::NotConnected => Self::NotConnected,
            ListenerError::Stopped => Self::Stopped,
        }
    }
}

impl From<StationError> for NestwatchError {
    fn from(e: StationError) -> Self {
        match e {
            StationError::Storage(s) => Self::Storage {
                reason: s.to_string(),
            },
            StationError::Net(n) => Self::Network {
                reason: n.to_string(),
            },
            StationError::Rng => Self::Network {
                reason: "random number generator failed".into(),
            },
            StationError::Stopped => Self::Stopped,
            StationError::Invalid(reason) => Self::Invalid { reason },
        }
    }
}

impl From<StorageError> for NestwatchError {
    fn from(e: StorageError) -> Self {
        Self::Storage {
            reason: e.to_string(),
        }
    }
}

/// Error a foreign [`crate::Storage`] implementation throws.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum StorageFailure {
    /// The backend (file system, key store) failed.
    #[error("storage backend: {reason}")]
    Backend {
        /// What failed.
        reason: String,
    },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for StorageFailure {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Backend { reason: e.reason }
    }
}

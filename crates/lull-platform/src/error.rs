// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io;

/// Errors returned by platform backends.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// The OS refused access (microphone, camera or keychain permission).
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The device or service does not exist, is busy or went away.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// The requested device id is unknown.
    #[error("no such device: {0}")]
    NoSuchDevice(String),
    /// This backend or operation is not supported on this platform or build.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// A caller-supplied argument is invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// An I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    /// Any other backend failure.
    #[error("{0}")]
    Backend(String),
}

impl PlatformError {
    /// The protocol `video-state` value for a camera failure
    /// (protocol.md §7.7): `"denied"` for a refused OS permission,
    /// `"unavailable"` otherwise.
    pub fn video_state(&self) -> &'static str {
        match self {
            Self::PermissionDenied(_) => "denied",
            _ => "unavailable",
        }
    }

    /// Classifies a backend message: anything mentioning a refused
    /// permission becomes [`PlatformError::PermissionDenied`], otherwise
    /// [`PlatformError::Unavailable`].
    pub(crate) fn from_message(message: impl Into<String>) -> Self {
        let message = message.into();
        let lower = message.to_ascii_lowercase();
        if lower.contains("permission denied")
            || lower.contains("not authorized")
            || lower.contains("access denied")
            || lower.contains("access is denied")
        {
            Self::PermissionDenied(message)
        } else {
            Self::Unavailable(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_state_mapping() {
        assert_eq!(
            PlatformError::PermissionDenied("x".into()).video_state(),
            "denied"
        );
        assert_eq!(
            PlatformError::Unavailable("x".into()).video_state(),
            "unavailable"
        );
        assert_eq!(
            PlatformError::from_message("open /dev/video0: Permission denied (os error 13)")
                .video_state(),
            "denied"
        );
        assert_eq!(
            PlatformError::from_message("device busy").video_state(),
            "unavailable"
        );
    }
}

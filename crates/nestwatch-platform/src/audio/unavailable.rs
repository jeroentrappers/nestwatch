// SPDX-License-Identifier: AGPL-3.0-or-later
//! Placeholder audio backend used when nothing else is available.

use super::{
    AudioDevice, AudioInput, AudioOutput, AudioStream, CaptureCallback, CaptureRequest,
    PlaybackRequest, PlaybackSource,
};
use crate::PlatformError;

/// Fails every request with the reason no backend was found.
#[derive(Debug, Clone)]
pub(crate) struct NoAudio(pub(crate) String);

impl AudioInput for NoAudio {
    fn backend(&self) -> &'static str {
        "none"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        Err(PlatformError::Unavailable(self.0.clone()))
    }

    fn start(
        &self,
        _: &CaptureRequest,
        _: CaptureCallback,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        Err(PlatformError::Unavailable(self.0.clone()))
    }
}

impl AudioOutput for NoAudio {
    fn backend(&self) -> &'static str {
        "none"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        Err(PlatformError::Unavailable(self.0.clone()))
    }

    fn start(
        &self,
        _: &PlaybackRequest,
        _: PlaybackSource,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        Err(PlatformError::Unavailable(self.0.clone()))
    }
}

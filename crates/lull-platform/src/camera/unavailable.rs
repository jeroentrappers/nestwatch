// SPDX-License-Identifier: AGPL-3.0-or-later
//! Placeholder camera backend used when nothing else is available.

use super::{Camera, CameraInfo, CameraRequest, CameraStream, FrameCallback};
use crate::PlatformError;

/// Lists no cameras and fails every start with `unavailable`.
#[derive(Debug, Clone)]
pub(crate) struct NoCamera(pub(crate) String);

impl Camera for NoCamera {
    fn backend(&self) -> &'static str {
        "none"
    }

    fn cameras(&self) -> Result<Vec<CameraInfo>, PlatformError> {
        Ok(Vec::new())
    }

    fn start(
        &self,
        _: &CameraRequest,
        _: FrameCallback,
    ) -> Result<Box<dyn CameraStream>, PlatformError> {
        Err(PlatformError::Unavailable(self.0.clone()))
    }
}

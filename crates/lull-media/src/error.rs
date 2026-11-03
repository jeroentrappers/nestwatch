// SPDX-License-Identifier: AGPL-3.0-or-later
use std::fmt;

/// Errors returned by media operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// The codec id is unknown or not compiled in.
    UnsupportedCodec(u8),
    /// An encoded payload does not parse.
    InvalidPayload(&'static str),
    /// A parameter is out of range.
    InvalidParameter(&'static str),
    /// The underlying codec library reported an error.
    Codec(String),
    /// JPEG encoding or decoding failed.
    Jpeg(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCodec(id) => write!(f, "unsupported audio codec {id}"),
            Self::InvalidPayload(why) => write!(f, "invalid payload: {why}"),
            Self::InvalidParameter(why) => write!(f, "invalid parameter: {why}"),
            Self::Codec(why) => write!(f, "codec error: {why}"),
            Self::Jpeg(why) => write!(f, "jpeg error: {why}"),
        }
    }
}

impl std::error::Error for MediaError {}

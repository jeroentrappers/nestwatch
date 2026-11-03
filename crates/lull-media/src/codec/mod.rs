// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio codecs for 20 ms frames (media.md §2).

pub mod adpcm;
#[cfg(feature = "opus")]
pub mod opus;

use std::fmt::Debug;

use crate::{CodecId, MediaError, PcmFrame};

/// Encodes 20 ms frames of 16 kHz mono PCM.
pub trait AudioEncoder: Debug + Send {
    /// Codec produced by this encoder.
    fn codec(&self) -> CodecId;
    /// Encodes one frame into one payload.
    fn encode(&mut self, pcm: &PcmFrame) -> Result<Vec<u8>, MediaError>;
}

/// Decodes 20 ms payloads and conceals lost frames (§2.2).
pub trait AudioDecoder: Debug + Send {
    /// Codec accepted by this decoder.
    fn codec(&self) -> CodecId;
    /// Decodes one payload.
    fn decode(&mut self, payload: &[u8]) -> Result<PcmFrame, MediaError>;
    /// Produces a replacement for one lost frame. `next` is the payload that
    /// follows the lost one, if already received (for FEC). Returns `None`
    /// once concealment is exhausted and silence should be played.
    fn conceal(&mut self, next: Option<&[u8]>) -> Option<PcmFrame>;
    /// Forgets all history.
    fn reset(&mut self);
}

/// Codecs compiled into this build, in preference order (for `hello.audio`).
pub fn supported_codecs() -> &'static [CodecId] {
    #[cfg(feature = "opus")]
    {
        &[CodecId::OPUS, CodecId::ADPCM]
    }
    #[cfg(not(feature = "opus"))]
    {
        &[CodecId::ADPCM]
    }
}

/// Creates an encoder for `codec`.
pub fn new_encoder(codec: CodecId) -> Result<Box<dyn AudioEncoder>, MediaError> {
    match codec {
        CodecId::ADPCM => Ok(Box::new(adpcm::AdpcmEncoder::new())),
        #[cfg(feature = "opus")]
        CodecId::OPUS => Ok(Box::new(opus::OpusEncoder::new()?)),
        CodecId(other) => Err(MediaError::UnsupportedCodec(other)),
    }
}

/// Creates a decoder for `codec`.
pub fn new_decoder(codec: CodecId) -> Result<Box<dyn AudioDecoder>, MediaError> {
    match codec {
        CodecId::ADPCM => Ok(Box::new(adpcm::AdpcmDecoder::new())),
        #[cfg(feature = "opus")]
        CodecId::OPUS => Ok(Box::new(opus::OpusDecoder::new()?)),
        CodecId(other) => Err(MediaError::UnsupportedCodec(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_2_codec_2_always_supported() {
        assert!(supported_codecs().contains(&CodecId::ADPCM));
        assert_eq!(new_encoder(CodecId::ADPCM).unwrap().codec(), CodecId::ADPCM);
        assert_eq!(new_decoder(CodecId::ADPCM).unwrap().codec(), CodecId::ADPCM);
        assert_eq!(
            new_decoder(CodecId(9)).unwrap_err(),
            MediaError::UnsupportedCodec(9)
        );
    }
}

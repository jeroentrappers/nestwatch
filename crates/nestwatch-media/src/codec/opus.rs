// SPDX-License-Identifier: AGPL-3.0-or-later
//! Codec 1: Opus via libopus (media.md §2). Native targets only.
//!
//! Mono, 20 ms frames, wideband (16 kHz audio bandwidth), VBR targeting
//! 24 kbit/s, in-band FEC on. The 48 kHz RTP clock only affects AUDIO
//! timestamps; the codec runs at the internal 16 kHz rate.

use std::fmt;

use opus::{Application, Bandwidth, Bitrate, Channels, Decoder, Encoder};

use super::{AudioDecoder, AudioEncoder};
use crate::{CodecId, FRAME_SAMPLES, MediaError, PcmFrame, SAMPLE_RATE};

/// VBR target bitrate (§2).
pub const BITRATE: i32 = 24_000;
/// Expected loss used to size in-band FEC.
const PACKET_LOSS_PERC: i32 = 10;
/// Largest payload produced.
const MAX_PACKET: usize = 1275;

fn err(e: opus::Error) -> MediaError {
    MediaError::Codec(e.to_string())
}

/// Opus encoder with the §2 parameters.
pub struct OpusEncoder {
    inner: Encoder,
}

impl fmt::Debug for OpusEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpusEncoder").finish_non_exhaustive()
    }
}

impl OpusEncoder {
    /// Creates an encoder.
    pub fn new() -> Result<Self, MediaError> {
        let mut inner =
            Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).map_err(err)?;
        inner.set_bitrate(Bitrate::Bits(BITRATE)).map_err(err)?;
        inner.set_vbr(true).map_err(err)?;
        inner.set_inband_fec(true).map_err(err)?;
        inner.set_packet_loss_perc(PACKET_LOSS_PERC).map_err(err)?;
        inner.set_max_bandwidth(Bandwidth::Wideband).map_err(err)?;
        Ok(Self { inner })
    }
}

impl AudioEncoder for OpusEncoder {
    fn codec(&self) -> CodecId {
        CodecId::OPUS
    }

    fn encode(&mut self, pcm: &PcmFrame) -> Result<Vec<u8>, MediaError> {
        self.inner.encode_vec(pcm, MAX_PACKET).map_err(err)
    }
}

/// Opus decoder using libopus PLC and in-band FEC for concealment (§2.2).
pub struct OpusDecoder {
    inner: Decoder,
}

impl fmt::Debug for OpusDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpusDecoder").finish_non_exhaustive()
    }
}

impl OpusDecoder {
    /// Creates a decoder.
    pub fn new() -> Result<Self, MediaError> {
        Ok(Self {
            inner: Decoder::new(SAMPLE_RATE, Channels::Mono).map_err(err)?,
        })
    }

    fn run(&mut self, input: &[u8], fec: bool) -> Result<PcmFrame, MediaError> {
        let mut out = [0i16; FRAME_SAMPLES];
        let n = self.inner.decode(input, &mut out, fec).map_err(err)?;
        if n != FRAME_SAMPLES {
            return Err(MediaError::InvalidPayload("opus frame is not 20 ms"));
        }
        Ok(out)
    }
}

impl AudioDecoder for OpusDecoder {
    fn codec(&self) -> CodecId {
        CodecId::OPUS
    }

    fn decode(&mut self, payload: &[u8]) -> Result<PcmFrame, MediaError> {
        if payload.is_empty() {
            return Err(MediaError::InvalidPayload("empty opus frame"));
        }
        self.run(payload, false)
    }

    fn conceal(&mut self, next: Option<&[u8]>) -> Option<PcmFrame> {
        if let Some(next) = next.filter(|p| !p.is_empty())
            && let Ok(pcm) = self.run(next, true)
        {
            return Some(pcm);
        }
        self.run(&[], false).ok()
    }

    fn reset(&mut self) {
        let _ = self.inner.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::level_dbfs;

    fn sine_frames(n: usize) -> Vec<PcmFrame> {
        (0..n)
            .map(|k| {
                let mut f = [0i16; FRAME_SAMPLES];
                for (i, s) in f.iter_mut().enumerate() {
                    let t = (k * FRAME_SAMPLES + i) as f64 / 16_000.0;
                    *s = (8000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
                }
                f
            })
            .collect()
    }

    #[test]
    fn media_2_opus_round_trip_and_bitrate() {
        let mut enc = OpusEncoder::new().unwrap();
        let mut dec = OpusDecoder::new().unwrap();
        let frames = sine_frames(100);
        let mut bytes = 0;
        let mut last = [0i16; FRAME_SAMPLES];
        for f in &frames {
            let p = enc.encode(f).unwrap();
            bytes += p.len();
            last = dec.decode(&p).unwrap();
        }
        // 2 s of audio; VBR around 24 kbit/s.
        let kbps = bytes as f64 * 8.0 / 2.0 / 1000.0;
        assert!(kbps > 8.0 && kbps < 40.0, "{kbps} kbit/s");
        let l = level_dbfs(&last);
        assert!((l - level_dbfs(&frames[99])).abs() < 3.0, "{l}");
    }

    #[test]
    fn media_2_2_opus_plc_and_fec() {
        let mut enc = OpusEncoder::new().unwrap();
        let mut dec = OpusDecoder::new().unwrap();
        let frames = sine_frames(20);
        let payloads: Vec<_> = frames.iter().map(|f| enc.encode(f).unwrap()).collect();
        for p in &payloads[..10] {
            dec.decode(p).unwrap();
        }
        // Frame 10 lost, frame 11 available: FEC.
        let fec = dec.conceal(Some(&payloads[11])).unwrap();
        assert!(level_dbfs(&fec) > -40.0);
        dec.decode(&payloads[11]).unwrap();
        // Nothing available: PLC.
        assert!(dec.conceal(None).is_some());
    }
}

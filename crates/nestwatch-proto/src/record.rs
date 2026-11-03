// SPDX-License-Identifier: AGPL-3.0-or-later
//! Records: the plaintext of each Noise transport message (protocol.md §6).

use thiserror::Error;

use crate::control::{Control, ControlError};
use crate::frame::MAX_NOISE_MESSAGE_LEN;

/// Noise AEAD tag length; a record is at most 65535 - 16 bytes.
pub const NOISE_TAG_LEN: usize = 16;
/// Largest record that fits one Noise transport message.
pub const MAX_RECORD_LEN: usize = MAX_NOISE_MESSAGE_LEN - NOISE_TAG_LEN;
/// AUDIO header length after the type byte (protocol.md §6.1).
pub const AUDIO_HEADER_LEN: usize = 11;
/// VIDEO header length after the type byte (protocol.md §6.2).
pub const VIDEO_HEADER_LEN: usize = 17;
/// Largest VIDEO fragment payload that fits one record.
pub const MAX_VIDEO_FRAGMENT_LEN: usize = MAX_RECORD_LEN - 1 - VIDEO_HEADER_LEN;

/// Audio codec 1: Opus (media.md §2).
pub const AUDIO_CODEC_OPUS: u8 = 1;
/// Audio codec 2: IMA ADPCM 16 kHz (media.md §2).
pub const AUDIO_CODEC_ADPCM: u8 = 2;
/// Video codec 1: baseline JPEG (protocol.md §6.2).
pub const VIDEO_CODEC_JPEG: u8 = 1;

/// Record type byte (protocol.md §6).
pub mod record_type {
    /// CONTROL.
    pub const CONTROL: u8 = 0x01;
    /// AUDIO.
    pub const AUDIO: u8 = 0x02;
    /// VIDEO.
    pub const VIDEO: u8 = 0x03;
    /// PING.
    pub const PING: u8 = 0x04;
    /// PONG.
    pub const PONG: u8 = 0x05;
}

/// Errors decoding or encoding a record. Any decode error is a
/// `protocol-error` close (protocol.md §6).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecordError {
    /// The record has no type byte.
    #[error("empty record")]
    Empty,
    /// The body is shorter than its fixed header.
    #[error("record type {record_type:#04x} body truncated ({len} bytes)")]
    Truncated {
        /// Record type.
        record_type: u8,
        /// Body length received.
        len: usize,
    },
    /// A PING/PONG body is not exactly 8 bytes.
    #[error("record type {record_type:#04x} body must be 8 bytes, got {len}")]
    BadLength {
        /// Record type.
        record_type: u8,
        /// Body length received.
        len: usize,
    },
    /// AUDIO `stream` is neither 0 nor 1.
    #[error("unknown audio stream {0}")]
    InvalidStream(u8),
    /// VIDEO `frag_count` is 0 or `frag_index >= frag_count`.
    #[error("invalid video fragment {index}/{count}")]
    InvalidFragment {
        /// `frag_index`.
        index: u16,
        /// `frag_count`.
        count: u16,
    },
    /// The CONTROL body is invalid.
    #[error("control message: {0}")]
    Control(#[from] ControlError),
    /// The encoded record would exceed [`MAX_RECORD_LEN`].
    #[error("record of {0} bytes exceeds maximum")]
    TooLarge(usize),
}

/// AUDIO `stream` (protocol.md §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AudioStream {
    /// Station microphone, station→listener.
    Station = 0,
    /// Talk-back, listener→station.
    TalkBack = 1,
}

/// AUDIO `flags` (protocol.md §6.1). Undefined bits are preserved and
/// otherwise ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AudioFlags(pub u8);

impl AudioFlags {
    /// Bit 0: pre-roll.
    pub const PREROLL: u8 = 0x01;
    /// Bit 1: first frame of a burst.
    pub const BURST_START: u8 = 0x02;

    /// Builds flags from the two defined bits.
    pub const fn new(preroll: bool, burst_start: bool) -> Self {
        Self((preroll as u8) | ((burst_start as u8) << 1))
    }

    /// Pre-roll frame sent late because sound activation opened.
    pub const fn preroll(self) -> bool {
        self.0 & Self::PREROLL != 0
    }

    /// First frame after silence, a gap or talk start.
    pub const fn burst_start(self) -> bool {
        self.0 & Self::BURST_START != 0
    }
}

/// AUDIO record header (protocol.md §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioHeader {
    /// Stream.
    pub stream: AudioStream,
    /// Codec id (media.md §2).
    pub codec: u8,
    /// Flags.
    pub flags: AudioFlags,
    /// Per-stream sequence number.
    pub seq: u32,
    /// Capture time in 1/48000 s, wrapping.
    pub timestamp: u32,
}

impl AudioHeader {
    /// Appends the 11 header bytes.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.stream as u8);
        out.push(self.codec);
        out.push(self.flags.0);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
    }

    /// Parses an AUDIO body into header and payload.
    pub fn decode(body: &[u8]) -> Result<(Self, &[u8]), RecordError> {
        let (h, payload) =
            body.split_first_chunk::<AUDIO_HEADER_LEN>()
                .ok_or(RecordError::Truncated {
                    record_type: record_type::AUDIO,
                    len: body.len(),
                })?;
        let stream = match h[0] {
            0 => AudioStream::Station,
            1 => AudioStream::TalkBack,
            s => return Err(RecordError::InvalidStream(s)),
        };
        Ok((
            Self {
                stream,
                codec: h[1],
                flags: AudioFlags(h[2]),
                seq: u32::from_be_bytes([h[3], h[4], h[5], h[6]]),
                timestamp: u32::from_be_bytes([h[7], h[8], h[9], h[10]]),
            },
            payload,
        ))
    }
}

/// VIDEO record header (protocol.md §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoHeader {
    /// Codec id; [`VIDEO_CODEC_JPEG`].
    pub codec: u8,
    /// Frame counter, +1 per captured frame.
    pub frame_id: u32,
    /// Station clock in ms, wrapping.
    pub capture_ms: u32,
    /// Frame width.
    pub width: u16,
    /// Frame height.
    pub height: u16,
    /// 0-based fragment index.
    pub frag_index: u16,
    /// Number of fragments, at least 1.
    pub frag_count: u16,
}

impl VideoHeader {
    /// Appends the 17 header bytes.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.codec);
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.capture_ms.to_be_bytes());
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.frag_index.to_be_bytes());
        out.extend_from_slice(&self.frag_count.to_be_bytes());
    }

    /// Checks `frag_count >= 1` and `frag_index < frag_count`.
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.frag_count == 0 || self.frag_index >= self.frag_count {
            Err(RecordError::InvalidFragment {
                index: self.frag_index,
                count: self.frag_count,
            })
        } else {
            Ok(())
        }
    }

    /// Parses a VIDEO body into header and payload.
    pub fn decode(body: &[u8]) -> Result<(Self, &[u8]), RecordError> {
        let (h, payload) =
            body.split_first_chunk::<VIDEO_HEADER_LEN>()
                .ok_or(RecordError::Truncated {
                    record_type: record_type::VIDEO,
                    len: body.len(),
                })?;
        let u32_at = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        let u16_at = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let header = Self {
            codec: h[0],
            frame_id: u32_at(1),
            capture_ms: u32_at(5),
            width: u16_at(9),
            height: u16_at(11),
            frag_index: u16_at(13),
            frag_count: u16_at(15),
        };
        header.validate()?;
        Ok((header, payload))
    }
}

/// A decoded record (protocol.md §6).
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// CONTROL (0x01).
    Control(Control),
    /// AUDIO (0x02).
    Audio {
        /// Header.
        header: AudioHeader,
        /// One 20 ms codec frame.
        payload: Vec<u8>,
    },
    /// VIDEO (0x03), one fragment of an encoded frame.
    Video {
        /// Header.
        header: VideoHeader,
        /// Fragment bytes.
        payload: Vec<u8>,
    },
    /// PING (0x04) with the sender clock in ms.
    Ping(u64),
    /// PONG (0x05) echoing the PING's 8 bytes.
    Pong(u64),
    /// A record type not defined in NWP/1; receivers ignore it (§6).
    Unknown {
        /// Type byte.
        record_type: u8,
        /// Raw body.
        body: Vec<u8>,
    },
}

fn u64_body(record_type: u8, body: &[u8]) -> Result<u64, RecordError> {
    let b: [u8; 8] = body.try_into().map_err(|_| RecordError::BadLength {
        record_type,
        len: body.len(),
    })?;
    Ok(u64::from_be_bytes(b))
}

impl Record {
    /// The record type byte.
    pub fn record_type(&self) -> u8 {
        match self {
            Self::Control(_) => record_type::CONTROL,
            Self::Audio { .. } => record_type::AUDIO,
            Self::Video { .. } => record_type::VIDEO,
            Self::Ping(_) => record_type::PING,
            Self::Pong(_) => record_type::PONG,
            Self::Unknown { record_type, .. } => *record_type,
        }
    }

    /// The PONG answering this PING.
    pub fn pong_for(&self) -> Option<Self> {
        match self {
            Self::Ping(t) => Some(Self::Pong(*t)),
            _ => None,
        }
    }

    /// Encodes `type || body`.
    pub fn encode(&self) -> Result<Vec<u8>, RecordError> {
        let mut out = vec![self.record_type()];
        match self {
            Self::Control(c) => out.extend_from_slice(&c.encode()?),
            Self::Audio { header, payload } => {
                header.encode_into(&mut out);
                out.extend_from_slice(payload);
            }
            Self::Video { header, payload } => {
                header.validate()?;
                header.encode_into(&mut out);
                out.extend_from_slice(payload);
            }
            Self::Ping(t) | Self::Pong(t) => out.extend_from_slice(&t.to_be_bytes()),
            Self::Unknown { body, .. } => out.extend_from_slice(body),
        }
        if out.len() > MAX_RECORD_LEN {
            return Err(RecordError::TooLarge(out.len()));
        }
        Ok(out)
    }

    /// Decodes a record. Unknown types yield [`Record::Unknown`]; malformed
    /// bodies of known types are errors.
    pub fn decode(bytes: &[u8]) -> Result<Self, RecordError> {
        let (&t, body) = bytes.split_first().ok_or(RecordError::Empty)?;
        Ok(match t {
            record_type::CONTROL => Self::Control(Control::decode(body)?),
            record_type::AUDIO => {
                let (header, payload) = AudioHeader::decode(body)?;
                Self::Audio {
                    header,
                    payload: payload.to_vec(),
                }
            }
            record_type::VIDEO => {
                let (header, payload) = VideoHeader::decode(body)?;
                Self::Video {
                    header,
                    payload: payload.to_vec(),
                }
            }
            record_type::PING => Self::Ping(u64_body(t, body)?),
            record_type::PONG => Self::Pong(u64_body(t, body)?),
            _ => Self::Unknown {
                record_type: t,
                body: body.to_vec(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio() -> Record {
        Record::Audio {
            header: AudioHeader {
                stream: AudioStream::Station,
                codec: AUDIO_CODEC_ADPCM,
                flags: AudioFlags::new(true, true),
                seq: 0x0102_0304,
                timestamp: 0xA0B0_C0D0,
            },
            payload: vec![0xEE; 5],
        }
    }

    fn video() -> Record {
        Record::Video {
            header: VideoHeader {
                codec: VIDEO_CODEC_JPEG,
                frame_id: 7,
                capture_ms: 1000,
                width: 640,
                height: 480,
                frag_index: 1,
                frag_count: 3,
            },
            payload: vec![0xFF, 0xD8],
        }
    }

    #[test]
    fn audio_layout_section_6_1() {
        let bytes = audio().encode().unwrap();
        assert_eq!(
            bytes,
            [
                0x02, 0x00, 0x02, 0x03, 0x01, 0x02, 0x03, 0x04, 0xA0, 0xB0, 0xC0, 0xD0, 0xEE, 0xEE,
                0xEE, 0xEE, 0xEE
            ]
        );
        assert_eq!(Record::decode(&bytes).unwrap(), audio());
    }

    #[test]
    fn video_layout_section_6_2() {
        let bytes = video().encode().unwrap();
        assert_eq!(
            bytes,
            [
                0x03, 0x01, 0, 0, 0, 7, 0, 0, 0x03, 0xE8, 0x02, 0x80, 0x01, 0xE0, 0, 1, 0, 3, 0xFF,
                0xD8
            ]
        );
        assert_eq!(Record::decode(&bytes).unwrap(), video());
    }

    #[test]
    fn ping_pong_section_6() {
        let ping = Record::Ping(0x0011_2233_4455_6677);
        let bytes = ping.encode().unwrap();
        assert_eq!(bytes, [4, 0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
        let pong = ping.pong_for().unwrap();
        let pb = pong.encode().unwrap();
        assert_eq!(pb[0], 5);
        assert_eq!(pb[1..], bytes[1..]);
        assert_eq!(Record::decode(&pb).unwrap(), pong);
        assert_eq!(audio().pong_for(), None);
    }

    #[test]
    fn control_record() {
        let r = Record::Control(Control::VideoStop);
        let bytes = r.encode().unwrap();
        assert_eq!(bytes[0], 1);
        assert_eq!(Record::decode(&bytes).unwrap(), r);
    }

    #[test]
    fn unknown_types_are_surfaced_section_6() {
        for t in [0x00, 0x06, 0x7F, 0xFF] {
            let r = Record::decode(&[t, 1, 2, 3]).unwrap();
            assert_eq!(
                r,
                Record::Unknown {
                    record_type: t,
                    body: vec![1, 2, 3]
                }
            );
            assert_eq!(r.encode().unwrap(), vec![t, 1, 2, 3]);
        }
    }

    #[test]
    fn malformed_bodies_are_errors_section_6() {
        assert_eq!(Record::decode(&[]), Err(RecordError::Empty));
        let a = audio().encode().unwrap();
        for n in 1..12 {
            assert_eq!(
                Record::decode(&a[..n]),
                Err(RecordError::Truncated {
                    record_type: 2,
                    len: n - 1
                })
            );
        }
        // Header only: empty payload is accepted at this layer.
        assert!(Record::decode(&a[..12]).is_ok());
        let mut bad_stream = a.clone();
        bad_stream[1] = 2;
        assert_eq!(
            Record::decode(&bad_stream),
            Err(RecordError::InvalidStream(2))
        );
        let v = video().encode().unwrap();
        assert!(matches!(
            Record::decode(&v[..17]),
            Err(RecordError::Truncated { record_type: 3, .. })
        ));
        let mut zero_count = v.clone();
        zero_count[16..18].copy_from_slice(&[0, 0]);
        zero_count[14..16].copy_from_slice(&[0, 0]);
        assert_eq!(
            Record::decode(&zero_count),
            Err(RecordError::InvalidFragment { index: 0, count: 0 })
        );
        let mut idx_too_big = v.clone();
        idx_too_big[14..16].copy_from_slice(&[0, 3]);
        assert_eq!(
            Record::decode(&idx_too_big),
            Err(RecordError::InvalidFragment { index: 3, count: 3 })
        );
        for n in [0usize, 7, 9] {
            let mut p = vec![4];
            p.extend(std::iter::repeat_n(0, n));
            assert_eq!(
                Record::decode(&p),
                Err(RecordError::BadLength {
                    record_type: 4,
                    len: n
                })
            );
        }
        assert!(matches!(
            Record::decode(&[1, 0x80]),
            Err(RecordError::Control(ControlError::NotAMap))
        ));
    }

    #[test]
    fn flags_preserve_reserved_bits() {
        let f = AudioFlags(0xFC);
        assert!(!f.preroll() && !f.burst_start());
        let mut r = audio();
        if let Record::Audio { header, .. } = &mut r {
            header.flags = AudioFlags(0x81);
        }
        assert_eq!(Record::decode(&r.encode().unwrap()).unwrap(), r);
    }

    #[test]
    fn size_limit() {
        let mut r = audio();
        if let Record::Audio { payload, .. } = &mut r {
            *payload = vec![0; MAX_RECORD_LEN - 12];
        }
        assert_eq!(r.encode().unwrap().len(), MAX_RECORD_LEN);
        if let Record::Audio { payload, .. } = &mut r {
            payload.push(0);
        }
        assert_eq!(r.encode(), Err(RecordError::TooLarge(MAX_RECORD_LEN + 1)));
    }

    #[test]
    fn header_round_trips_property() {
        let mut seed = 0x1234_5678u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            seed >> 16
        };
        for _ in 0..500 {
            let a = AudioHeader {
                stream: if next() & 1 == 0 {
                    AudioStream::Station
                } else {
                    AudioStream::TalkBack
                },
                codec: next() as u8,
                flags: AudioFlags(next() as u8),
                seq: next() as u32,
                timestamp: next() as u32,
            };
            let mut out = Vec::new();
            a.encode_into(&mut out);
            assert_eq!(AudioHeader::decode(&out).unwrap(), (a, &[][..]));
            let count = (next() as u16).max(1);
            let v = VideoHeader {
                codec: next() as u8,
                frame_id: next() as u32,
                capture_ms: next() as u32,
                width: next() as u16,
                height: next() as u16,
                frag_index: next() as u16 % count,
                frag_count: count,
            };
            let mut out = Vec::new();
            v.encode_into(&mut out);
            out.push(9);
            assert_eq!(VideoHeader::decode(&out).unwrap(), (v, &[9u8][..]));
        }
    }
}

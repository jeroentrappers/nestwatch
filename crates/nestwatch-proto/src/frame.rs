// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transport framing of Noise messages (protocol.md §3.1, §3.3).

use thiserror::Error;

/// Largest Noise message a frame can carry (protocol.md §3.1).
pub const MAX_NOISE_MESSAGE_LEN: usize = u16::MAX as usize;
/// Size of the big-endian length prefix on TCP (protocol.md §3.1).
pub const LENGTH_PREFIX_LEN: usize = 2;

/// Errors in framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FrameError {
    /// A Noise message must be 1..=65535 bytes long.
    #[error("invalid noise message length {0} (must be 1..=65535)")]
    InvalidLength(usize),
    /// A WebSocket text message was received (protocol.md §3.3).
    #[error("websocket text message is a protocol error")]
    TextMessage,
}

fn check_len(len: usize) -> Result<(), FrameError> {
    if (1..=MAX_NOISE_MESSAGE_LEN).contains(&len) {
        Ok(())
    } else {
        Err(FrameError::InvalidLength(len))
    }
}

/// Appends `length (u16) || noise_message` to `out` (protocol.md §3.1).
pub fn encode_frame_into(noise_message: &[u8], out: &mut Vec<u8>) -> Result<(), FrameError> {
    check_len(noise_message.len())?;
    out.reserve(LENGTH_PREFIX_LEN + noise_message.len());
    out.extend_from_slice(&(noise_message.len() as u16).to_be_bytes());
    out.extend_from_slice(noise_message);
    Ok(())
}

/// Returns `length (u16) || noise_message` (protocol.md §3.1).
pub fn encode_frame(noise_message: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::new();
    encode_frame_into(noise_message, &mut out)?;
    Ok(out)
}

/// Incremental decoder for the TCP byte stream (protocol.md §3.1).
///
/// Feed arbitrary chunks with [`push`](Self::push) and drain complete Noise
/// messages with [`next_message`](Self::next_message). A zero length prefix
/// is an error; after an error the decoder stays failed.
#[derive(Debug, Default, Clone)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    start: usize,
    failed: Option<FrameError>,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Number of buffered bytes not yet returned as messages.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }

    /// Returns the next complete Noise message, `Ok(None)` if more bytes are
    /// needed, or an error for an invalid length prefix.
    pub fn next_message(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if let Some(err) = self.failed {
            return Err(err);
        }
        let avail = &self.buf[self.start..];
        let Some(prefix) = avail.get(..LENGTH_PREFIX_LEN) else {
            return Ok(None);
        };
        let len = u16::from_be_bytes([prefix[0], prefix[1]]) as usize;
        if let Err(err) = check_len(len) {
            self.failed = Some(err);
            return Err(err);
        }
        let Some(msg) = avail.get(LENGTH_PREFIX_LEN..LENGTH_PREFIX_LEN + len) else {
            return Ok(None);
        };
        let msg = msg.to_vec();
        self.start += LENGTH_PREFIX_LEN + len;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start >= self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(msg))
    }

    /// Consumes the decoder and returns bytes not yet returned as messages.
    pub fn into_remaining(mut self) -> Vec<u8> {
        self.buf.drain(..self.start);
        self.buf
    }
}

impl Iterator for FrameDecoder {
    type Item = Result<Vec<u8>, FrameError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_message().transpose()
    }
}

/// Kind of a received WebSocket data message (protocol.md §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsKind {
    /// Binary message.
    Binary,
    /// Text message (always a protocol error in NWP/1).
    Text,
}

/// Validates a received WebSocket message and returns the Noise message it
/// carries (no length prefix, protocol.md §3.3).
pub fn ws_noise_message(kind: WsKind, payload: &[u8]) -> Result<&[u8], FrameError> {
    if kind == WsKind::Text {
        return Err(FrameError::TextMessage);
    }
    check_len(payload.len())?;
    Ok(payload)
}

/// Validates a Noise message before sending it as one binary WebSocket
/// message (protocol.md §3.3).
pub fn ws_encode(noise_message: &[u8]) -> Result<&[u8], FrameError> {
    check_len(noise_message.len())?;
    Ok(noise_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_messages() -> Vec<Vec<u8>> {
        vec![
            vec![1],
            vec![0xAB; 255],
            vec![0x5A; 256],
            (0..1000).map(|i| i as u8).collect(),
            vec![7; MAX_NOISE_MESSAGE_LEN],
        ]
    }

    #[test]
    fn encode_prefixes_big_endian_length() {
        assert_eq!(encode_frame(&[9, 8, 7]).unwrap(), vec![0, 3, 9, 8, 7]);
        let f = encode_frame(&vec![0; 0x1234]).unwrap();
        assert_eq!(&f[..2], &[0x12, 0x34]);
    }

    #[test]
    fn encode_rejects_bad_sizes() {
        assert_eq!(encode_frame(&[]), Err(FrameError::InvalidLength(0)));
        assert_eq!(
            encode_frame(&vec![0; 65536]),
            Err(FrameError::InvalidLength(65536))
        );
    }

    #[test]
    fn decode_whole_stream() {
        let msgs = sample_messages();
        let mut stream = Vec::new();
        for m in &msgs {
            encode_frame_into(m, &mut stream).unwrap();
        }
        let mut dec = FrameDecoder::new();
        dec.push(&stream);
        let got: Vec<_> = dec.by_ref().collect::<Result<_, _>>().unwrap();
        assert_eq!(got, msgs);
        assert_eq!(dec.buffered(), 0);
    }

    #[test]
    fn decode_byte_by_byte() {
        let msgs = sample_messages();
        let mut stream = Vec::new();
        for m in &msgs {
            encode_frame_into(m, &mut stream).unwrap();
        }
        let mut dec = FrameDecoder::new();
        let mut got = Vec::new();
        for b in &stream {
            dec.push(std::slice::from_ref(b));
            while let Some(m) = dec.next_message().unwrap() {
                got.push(m);
            }
        }
        assert_eq!(got, msgs);
    }

    #[test]
    fn decode_split_length_prefix() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0x00]);
        assert_eq!(dec.next_message(), Ok(None));
        dec.push(&[0x02, 0xAA]);
        assert_eq!(dec.next_message(), Ok(None));
        dec.push(&[0xBB, 0x00]);
        assert_eq!(dec.next_message(), Ok(Some(vec![0xAA, 0xBB])));
        assert_eq!(dec.next_message(), Ok(None));
        assert_eq!(dec.buffered(), 1);
        dec.push(&[0x01, 0xCC]);
        assert_eq!(dec.next_message(), Ok(Some(vec![0xCC])));
    }

    #[test]
    fn decode_arbitrary_chunkings() {
        let msgs = sample_messages();
        let mut stream = Vec::new();
        for m in &msgs {
            encode_frame_into(m, &mut stream).unwrap();
        }
        // Deterministic pseudo-random chunk sizes.
        let mut seed = 0x9E37_79B9u32;
        for _ in 0..50 {
            let mut dec = FrameDecoder::new();
            let mut got = Vec::new();
            let mut pos = 0;
            while pos < stream.len() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let n = (seed as usize % 4000 + 1).min(stream.len() - pos);
                dec.push(&stream[pos..pos + n]);
                pos += n;
                for m in dec.by_ref() {
                    got.push(m.unwrap());
                }
            }
            assert_eq!(got, msgs);
        }
    }

    #[test]
    fn zero_length_prefix_is_sticky_error() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0, 1, 5, 0, 0, 0, 1, 6]);
        assert_eq!(dec.next_message(), Ok(Some(vec![5])));
        assert_eq!(dec.next_message(), Err(FrameError::InvalidLength(0)));
        assert_eq!(dec.next_message(), Err(FrameError::InvalidLength(0)));
    }

    #[test]
    fn into_remaining_returns_partial_frame() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0, 1, 5, 0, 3, 1]);
        assert_eq!(dec.next_message(), Ok(Some(vec![5])));
        assert_eq!(dec.into_remaining(), vec![0, 3, 1]);
    }

    #[test]
    fn websocket_variant_has_no_prefix() {
        assert_eq!(ws_noise_message(WsKind::Binary, &[1, 2]), Ok(&[1u8, 2][..]));
        assert_eq!(
            ws_noise_message(WsKind::Text, b"hi"),
            Err(FrameError::TextMessage)
        );
        assert_eq!(
            ws_noise_message(WsKind::Binary, &[]),
            Err(FrameError::InvalidLength(0))
        );
        assert_eq!(
            ws_encode(&vec![0; 65536]),
            Err(FrameError::InvalidLength(65536))
        );
        assert!(ws_encode(&[0; 65535]).is_ok());
    }
}

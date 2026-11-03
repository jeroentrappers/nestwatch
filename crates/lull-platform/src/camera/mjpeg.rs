// SPDX-License-Identifier: AGPL-3.0-or-later
//! Splits a byte stream of concatenated JPEG images (MJPEG over a pipe) into
//! individual images.
//!
//! The splitter walks JPEG marker segments rather than searching for the
//! first `FF D9`, so an EOI inside an embedded EXIF thumbnail does not end a
//! frame early. Inside entropy-coded data, `FF 00` stuffing and `RSTn`
//! markers are skipped. Bytes before an SOI are discarded.

/// Largest image accepted before the buffer is resynchronised.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

impl Default for JpegSplitter {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental JPEG stream splitter.
#[derive(Debug)]
pub struct JpegSplitter {
    buf: Vec<u8>,
    limit: usize,
    /// Bytes discarded as garbage or oversized images.
    discarded: u64,
}

enum Scan {
    /// A complete image occupies `buf[start..end]`.
    Complete { start: usize, end: usize },
    /// An image starts at `start` but is not complete yet.
    Partial { start: usize },
    /// No SOI in the buffer; keep the last byte (it may be `FF`).
    NoStart,
}

impl JpegSplitter {
    /// Creates an empty splitter with the default size limit.
    pub fn new() -> Self {
        Self::with_limit(MAX_IMAGE_BYTES)
    }

    /// Creates an empty splitter accepting images up to `limit` bytes.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
            discarded: 0,
        }
    }

    /// Bytes discarded so far.
    pub fn discarded(&self) -> u64 {
        self.discarded
    }

    /// Appends bytes and returns every image completed by them.
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            match scan(&self.buf) {
                Scan::Complete { start, end } => {
                    self.discarded += start as u64;
                    out.push(self.buf[start..end].to_vec());
                    self.buf.drain(..end);
                }
                Scan::Partial { start } => {
                    self.discarded += start as u64;
                    self.buf.drain(..start);
                    if self.buf.len() > self.limit {
                        // Oversized or corrupt: drop this SOI and resync.
                        self.discarded += 2;
                        self.buf.drain(..2);
                        continue;
                    }
                    break;
                }
                Scan::NoStart => {
                    let keep = usize::from(self.buf.last() == Some(&0xFF));
                    let drop = self.buf.len() - keep;
                    self.discarded += drop as u64;
                    self.buf.drain(..drop);
                    break;
                }
            }
        }
        out
    }
}

fn find_soi(buf: &[u8], from: usize) -> Option<usize> {
    buf.get(from..)?
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .map(|p| p + from)
}

fn scan(buf: &[u8]) -> Scan {
    let Some(start) = find_soi(buf, 0) else {
        return Scan::NoStart;
    };
    let mut i = start + 2;
    loop {
        // Expect a marker; tolerate fill bytes (FF FF ...).
        let Some(&b) = buf.get(i) else {
            return Scan::Partial { start };
        };
        if b != 0xFF {
            // Corrupt segment structure: resync at the next SOI.
            return match find_soi(buf, start + 2) {
                Some(next) => scan_from(buf, next),
                None => Scan::Partial { start },
            };
        }
        let Some(&marker) = buf.get(i + 1) else {
            return Scan::Partial { start };
        };
        match marker {
            0xFF => i += 1,
            0xD9 => {
                return Scan::Complete { start, end: i + 2 };
            }
            // A new SOI before EOI: the previous image was truncated.
            0xD8 => return scan_from(buf, i),
            0x01 | 0xD0..=0xD7 => i += 2,
            _ => {
                let Some(len) = buf.get(i + 2..i + 4) else {
                    return Scan::Partial { start };
                };
                let len = usize::from(u16::from_be_bytes([len[0], len[1]]));
                if len < 2 {
                    return match find_soi(buf, start + 2) {
                        Some(next) => scan_from(buf, next),
                        None => Scan::Partial { start },
                    };
                }
                i += 2 + len;
                if marker == 0xDA {
                    // Entropy-coded data until a marker other than
                    // stuffing or RSTn.
                    loop {
                        let Some(&b) = buf.get(i) else {
                            return Scan::Partial { start };
                        };
                        if b != 0xFF {
                            i += 1;
                            continue;
                        }
                        let Some(&next) = buf.get(i + 1) else {
                            return Scan::Partial { start };
                        };
                        match next {
                            0x00 | 0xD0..=0xD7 => i += 2,
                            0xFF => i += 1,
                            _ => break,
                        }
                    }
                }
            }
        }
    }
}

/// Scans from a later SOI, reporting positions relative to the whole buffer.
fn scan_from(buf: &[u8], offset: usize) -> Scan {
    match scan(&buf[offset..]) {
        Scan::Complete { start, end } => Scan::Complete {
            start: start + offset,
            end: end + offset,
        },
        Scan::Partial { start } => Scan::Partial {
            start: start + offset,
        },
        Scan::NoStart => Scan::Partial { start: offset },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(seed: u8) -> Vec<u8> {
        let rgb: Vec<u8> = (0..32 * 16 * 3)
            .map(|i| (i as u8).wrapping_mul(seed))
            .collect();
        lull_media::video::encode_rgb(&rgb, 32, 16, 80).unwrap()
    }

    /// Inserts an APP1 segment holding a complete "thumbnail" JPEG.
    fn with_thumbnail(main: &[u8], thumb: &[u8]) -> Vec<u8> {
        let mut out = main[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        let len = (thumb.len() + 2 + 6) as u16;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(thumb);
        out.extend_from_slice(&main[2..]);
        out
    }

    #[test]
    fn splits_concatenated_images() {
        let (a, b) = (jpeg(3), jpeg(7));
        let mut stream = b"garbage".to_vec();
        stream.extend_from_slice(&a);
        stream.extend_from_slice(b"\x00\x01");
        stream.extend_from_slice(&b);
        let mut s = JpegSplitter::new();
        let frames = s.push(&stream);
        assert_eq!(frames, vec![a, b]);
        assert_eq!(s.discarded(), 9);
    }

    #[test]
    fn splits_across_every_chunk_boundary() {
        let (a, b) = (jpeg(5), jpeg(11));
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        for chunk in [1, 2, 3, 7, 64, 1000] {
            let mut s = JpegSplitter::new();
            let frames: Vec<_> = stream.chunks(chunk).flat_map(|c| s.push(c)).collect();
            assert_eq!(frames, vec![a.clone(), b.clone()], "chunk {chunk}");
        }
    }

    #[test]
    fn embedded_thumbnail_eoi_does_not_end_the_frame() {
        let main = with_thumbnail(&jpeg(9), &jpeg(2));
        let mut s = JpegSplitter::new();
        let frames = s.push(&main);
        assert_eq!(frames, vec![main.clone()]);
        assert!(lull_media::video::decode_to_rgba(&frames[0]).is_ok());
    }

    #[test]
    fn stuffing_and_restart_markers_are_skipped() {
        // SOI, SOS with a 2-byte header, data with FF00, RST0 and fill, EOI.
        let img = [
            0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0xFF,
            0xFF, 0xD9,
        ];
        let mut s = JpegSplitter::new();
        assert_eq!(s.push(&img), vec![img.to_vec()]);
    }

    #[test]
    fn truncated_image_is_dropped_at_next_soi() {
        let (a, b) = (jpeg(4), jpeg(6));
        let mut stream = a[..a.len() / 2].to_vec();
        stream.extend_from_slice(&b);
        let mut s = JpegSplitter::new();
        let frames = s.push(&stream);
        assert_eq!(frames.last(), Some(&b));
    }

    #[test]
    fn keeps_trailing_ff_when_no_soi() {
        let mut s = JpegSplitter::new();
        assert!(s.push(&[1, 2, 0xFF]).is_empty());
        let a = jpeg(8);
        assert_eq!(s.push(&a[1..]), vec![a]);
    }

    #[test]
    fn oversized_garbage_after_soi_is_bounded() {
        let mut s = JpegSplitter::with_limit(10_000);
        // SOI followed by an endless SOS with no end.
        s.push(&[0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02]);
        for _ in 0..20 {
            s.push(&[0x11; 1000]);
        }
        assert!(s.buf.len() <= 10_000);
        let a = jpeg(10);
        assert_eq!(s.push(&a), vec![a]);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video frame fragmentation and reassembly (protocol.md §6.2).

use std::collections::BTreeMap;

use thiserror::Error;

use crate::record::{MAX_VIDEO_FRAGMENT_LEN, Record, VideoHeader};

/// Maximum size of a complete encoded frame: 2 MiB (protocol.md §6.2).
pub const MAX_FRAME_LEN: usize = 2 * 1024 * 1024;
/// Incomplete frames a [`Reassembler`] tracks at once.
pub const MAX_PENDING_FRAMES: usize = 3;

/// Errors fragmenting or reassembling video.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VideoError {
    /// A frame must contain at least one byte.
    #[error("empty video frame")]
    EmptyFrame,
    /// The frame exceeds [`MAX_FRAME_LEN`].
    #[error("video frame exceeds 2 MiB")]
    FrameTooLarge,
    /// Fragment size must be at least 1.
    #[error("invalid fragment size")]
    InvalidFragmentSize,
    /// More than 65535 fragments would be needed.
    #[error("too many fragments")]
    TooManyFragments,
    /// Fragments of one frame disagree on header fields or fragment size,
    /// or a fragment index was repeated.
    #[error("inconsistent fragments for frame {0}")]
    Inconsistent(u32),
}

/// A complete encoded video frame (protocol.md §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Codec id.
    pub codec: u8,
    /// Frame counter.
    pub frame_id: u32,
    /// Station clock in ms.
    pub capture_ms: u32,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
    /// Encoded frame bytes.
    pub data: Vec<u8>,
}

/// Splits a frame into VIDEO records with one fragment size of
/// `min(max_fragment, MAX_VIDEO_FRAGMENT_LEN)` bytes (protocol.md §6.2).
pub fn fragment(frame: &VideoFrame, max_fragment: usize) -> Result<Vec<Record>, VideoError> {
    if frame.data.is_empty() {
        return Err(VideoError::EmptyFrame);
    }
    if frame.data.len() > MAX_FRAME_LEN {
        return Err(VideoError::FrameTooLarge);
    }
    if max_fragment == 0 {
        return Err(VideoError::InvalidFragmentSize);
    }
    let size = max_fragment.min(MAX_VIDEO_FRAGMENT_LEN);
    let count =
        u16::try_from(frame.data.len().div_ceil(size)).map_err(|_| VideoError::TooManyFragments)?;
    Ok(frame
        .data
        .chunks(size)
        .enumerate()
        .map(|(i, chunk)| Record::Video {
            header: VideoHeader {
                codec: frame.codec,
                frame_id: frame.frame_id,
                capture_ms: frame.capture_ms,
                width: frame.width,
                height: frame.height,
                frag_index: i as u16,
                frag_count: count,
            },
            payload: chunk.to_vec(),
        })
        .collect())
}

/// Whether frame `a` is newer than `b` under wrapping (RFC 1982) order.
pub fn frame_id_newer(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

#[derive(Debug)]
struct Pending {
    first: VideoHeader,
    fragments: BTreeMap<u16, Vec<u8>>,
    fragment_size: Option<usize>,
    bytes: usize,
}

impl Pending {
    fn new(first: VideoHeader) -> Self {
        Self {
            first,
            fragments: BTreeMap::new(),
            fragment_size: None,
            bytes: 0,
        }
    }

    fn add(&mut self, h: &VideoHeader, payload: &[u8]) -> Result<bool, VideoError> {
        let f = &self.first;
        let err = VideoError::Inconsistent(h.frame_id);
        if (h.codec, h.capture_ms, h.width, h.height, h.frag_count)
            != (f.codec, f.capture_ms, f.width, f.height, f.frag_count)
            || payload.is_empty()
            || self.fragments.contains_key(&h.frag_index)
        {
            return Err(err);
        }
        let last = h.frag_index + 1 == h.frag_count;
        if !last {
            match self.fragment_size {
                None => self.fragment_size = Some(payload.len()),
                Some(s) if s != payload.len() => return Err(err),
                Some(_) => {}
            }
        }
        if let Some(s) = self.fragment_size {
            let l = self.last_len(h, payload);
            if l.is_some_and(|l| l > s) {
                return Err(err);
            }
            let min_total = s * (usize::from(h.frag_count) - 1) + l.unwrap_or(1);
            if min_total > MAX_FRAME_LEN {
                return Err(VideoError::FrameTooLarge);
            }
        }
        self.bytes += payload.len();
        if self.bytes > MAX_FRAME_LEN {
            return Err(VideoError::FrameTooLarge);
        }
        self.fragments.insert(h.frag_index, payload.to_vec());
        Ok(self.fragments.len() == usize::from(h.frag_count))
    }

    fn last_len(&self, h: &VideoHeader, payload: &[u8]) -> Option<usize> {
        let last = h.frag_count - 1;
        if h.frag_index == last {
            Some(payload.len())
        } else {
            self.fragments.get(&last).map(Vec::len)
        }
    }

    fn into_frame(self) -> VideoFrame {
        let mut data = Vec::with_capacity(self.bytes);
        for frag in self.fragments.into_values() {
            data.extend_from_slice(&frag);
        }
        VideoFrame {
            codec: self.first.codec,
            frame_id: self.first.frame_id,
            capture_ms: self.first.capture_ms,
            width: self.first.width,
            height: self.first.height,
            data,
        }
    }
}

/// Reassembles VIDEO fragments by `frame_id` (protocol.md §6.2).
///
/// Yields each frame that completes and is newer than every frame yielded
/// before; incomplete older frames are discarded when a newer one completes.
/// At most [`MAX_PENDING_FRAMES`] incomplete frames are kept (the oldest is
/// evicted). On an error the offending frame is discarded.
#[derive(Debug, Default)]
pub struct Reassembler {
    pending: Vec<Pending>,
    last_complete: Option<u32>,
}

impl Reassembler {
    /// Creates an empty reassembler.
    pub fn new() -> Self {
        Self::default()
    }

    /// The `frame_id` of the last frame returned.
    pub fn last_complete(&self) -> Option<u32> {
        self.last_complete
    }

    /// Number of incomplete frames currently held.
    pub fn pending_frames(&self) -> usize {
        self.pending.len()
    }

    /// Adds one fragment; returns the frame if it is now complete.
    pub fn push(
        &mut self,
        header: &VideoHeader,
        payload: &[u8],
    ) -> Result<Option<VideoFrame>, VideoError> {
        let id = header.frame_id;
        if header.frag_count == 0 || header.frag_index >= header.frag_count {
            return Err(VideoError::Inconsistent(id));
        }
        if self.last_complete.is_some_and(|l| !frame_id_newer(id, l)) {
            return Ok(None);
        }
        let idx = match self.pending.iter().position(|p| p.first.frame_id == id) {
            Some(i) => i,
            None => {
                if self.pending.len() >= MAX_PENDING_FRAMES {
                    let oldest = self.oldest_index();
                    if frame_id_newer(self.pending[oldest].first.frame_id, id) {
                        return Ok(None);
                    }
                    self.pending.swap_remove(oldest);
                }
                self.pending.push(Pending::new(*header));
                self.pending.len() - 1
            }
        };
        match self.pending[idx].add(header, payload) {
            Ok(false) => Ok(None),
            Ok(true) => {
                let done = self.pending.swap_remove(idx);
                self.pending
                    .retain(|p| frame_id_newer(p.first.frame_id, id));
                self.last_complete = Some(id);
                Ok(Some(done.into_frame()))
            }
            Err(e) => {
                self.pending.swap_remove(idx);
                Err(e)
            }
        }
    }

    /// Convenience wrapper accepting a decoded record; non-VIDEO records
    /// yield `Ok(None)`.
    pub fn push_record(&mut self, record: &Record) -> Result<Option<VideoFrame>, VideoError> {
        match record {
            Record::Video { header, payload } => self.push(header, payload),
            _ => Ok(None),
        }
    }

    fn oldest_index(&self) -> usize {
        let mut oldest = 0;
        for (i, p) in self.pending.iter().enumerate() {
            if frame_id_newer(self.pending[oldest].first.frame_id, p.first.frame_id) {
                oldest = i;
            }
        }
        oldest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: u32, len: usize) -> VideoFrame {
        VideoFrame {
            codec: 1,
            frame_id: id,
            capture_ms: id.wrapping_mul(100),
            width: 640,
            height: 480,
            data: (0..len).map(|i| (i as u32 ^ id) as u8).collect(),
        }
    }

    fn parts(r: &[Record]) -> Vec<(VideoHeader, Vec<u8>)> {
        r.iter()
            .map(|r| match r {
                Record::Video { header, payload } => (*header, payload.clone()),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn fragment_sizes_section_6_2() {
        let f = frame(1, 10);
        let recs = parts(&fragment(&f, 4).unwrap());
        assert_eq!(recs.len(), 3);
        assert_eq!(
            recs.iter().map(|(_, p)| p.len()).collect::<Vec<_>>(),
            [4, 4, 2]
        );
        for (i, (h, _)) in recs.iter().enumerate() {
            assert_eq!((h.frag_index, h.frag_count), (i as u16, 3));
        }
        assert_eq!(parts(&fragment(&f, 10).unwrap()).len(), 1);
        assert_eq!(parts(&fragment(&f, 1000).unwrap()).len(), 1);
        let big = frame(1, 200_000);
        let recs = fragment(&big, usize::MAX).unwrap();
        assert_eq!(recs.len(), 200_000usize.div_ceil(MAX_VIDEO_FRAGMENT_LEN));
        for r in &recs {
            assert!(r.encode().is_ok());
        }
    }

    #[test]
    fn fragment_errors() {
        assert_eq!(fragment(&frame(1, 0), 4), Err(VideoError::EmptyFrame));
        assert_eq!(
            fragment(&frame(1, 4), 0),
            Err(VideoError::InvalidFragmentSize)
        );
        assert_eq!(
            fragment(&frame(1, MAX_FRAME_LEN + 1), 60_000),
            Err(VideoError::FrameTooLarge)
        );
        assert_eq!(
            fragment(&frame(1, 70_000), 1),
            Err(VideoError::TooManyFragments)
        );
        assert!(fragment(&frame(1, MAX_FRAME_LEN), 60_000).is_ok());
    }

    #[test]
    fn reassemble_in_order_and_out_of_order() {
        let f = frame(5, 1000);
        let recs = parts(&fragment(&f, 300).unwrap());
        let mut r = Reassembler::new();
        for (h, p) in &recs[..3] {
            assert_eq!(r.push(h, p), Ok(None));
        }
        assert_eq!(r.push(&recs[3].0, &recs[3].1), Ok(Some(f.clone())));
        assert_eq!(r.pending_frames(), 0);

        let f2 = frame(6, 1000);
        let mut recs = parts(&fragment(&f2, 300).unwrap());
        recs.reverse();
        let mut out = None;
        for (h, p) in &recs {
            out = r.push(h, p).unwrap().or(out);
        }
        assert_eq!(out, Some(f2));
        assert_eq!(r.last_complete(), Some(6));
    }

    #[test]
    fn newer_complete_frame_discards_older_incomplete() {
        let mut r = Reassembler::new();
        let old = parts(&fragment(&frame(10, 100), 40).unwrap());
        let new = parts(&fragment(&frame(11, 100), 40).unwrap());
        r.push(&old[0].0, &old[0].1).unwrap();
        r.push(&new[0].0, &new[0].1).unwrap();
        r.push(&new[1].0, &new[1].1).unwrap();
        assert_eq!(r.pending_frames(), 2);
        assert!(r.push(&new[2].0, &new[2].1).unwrap().is_some());
        assert_eq!(r.pending_frames(), 0);
        // Late fragments of the older frame are ignored.
        assert_eq!(r.push(&old[1].0, &old[1].1), Ok(None));
        assert_eq!(r.push(&old[2].0, &old[2].1), Ok(None));
        assert_eq!(r.pending_frames(), 0);
        // So is a repeat of the delivered frame.
        assert_eq!(r.push(&new[0].0, &new[0].1), Ok(None));
    }

    #[test]
    fn older_frame_may_complete_before_newer() {
        let mut r = Reassembler::new();
        let a = parts(&fragment(&frame(1, 20), 10).unwrap());
        let b = parts(&fragment(&frame(2, 20), 10).unwrap());
        r.push(&b[0].0, &b[0].1).unwrap();
        r.push(&a[0].0, &a[0].1).unwrap();
        assert_eq!(r.push(&a[1].0, &a[1].1).unwrap().unwrap().frame_id, 1);
        assert_eq!(r.pending_frames(), 1);
        assert_eq!(r.push(&b[1].0, &b[1].1).unwrap().unwrap().frame_id, 2);
    }

    #[test]
    fn pending_limit_evicts_oldest() {
        let mut r = Reassembler::new();
        let frames: Vec<_> = (0..5)
            .map(|i| parts(&fragment(&frame(100 + i, 20), 10).unwrap()))
            .collect();
        for f in &frames {
            r.push(&f[0].0, &f[0].1).unwrap();
        }
        assert_eq!(r.pending_frames(), MAX_PENDING_FRAMES);
        // Frames 100 and 101 were evicted.
        assert_eq!(r.push(&frames[0][1].0, &frames[0][1].1), Ok(None));
        assert_eq!(r.pending_frames(), MAX_PENDING_FRAMES);
        assert!(r.push(&frames[4][1].0, &frames[4][1].1).unwrap().is_some());
    }

    #[test]
    fn frame_id_wraps() {
        assert!(frame_id_newer(0, u32::MAX));
        assert!(!frame_id_newer(u32::MAX, 0));
        assert!(!frame_id_newer(3, 3));
        let mut r = Reassembler::new();
        for id in [u32::MAX - 1, u32::MAX, 0, 1] {
            let f = frame(id, 5);
            let recs = parts(&fragment(&f, 5).unwrap());
            assert_eq!(r.push(&recs[0].0, &recs[0].1), Ok(Some(f)));
        }
    }

    #[test]
    fn inconsistent_fragments_are_errors() {
        let recs = parts(&fragment(&frame(3, 30), 10).unwrap());
        // Different fragment sizes within one frame.
        let mut r = Reassembler::new();
        r.push(&recs[0].0, &recs[0].1).unwrap();
        assert_eq!(
            r.push(&recs[1].0, &recs[1].1[..5]),
            Err(VideoError::Inconsistent(3))
        );
        assert_eq!(r.pending_frames(), 0);
        // Header field mismatch.
        let mut r = Reassembler::new();
        r.push(&recs[0].0, &recs[0].1).unwrap();
        let mut h = recs[1].0;
        h.width = 320;
        assert_eq!(r.push(&h, &recs[1].1), Err(VideoError::Inconsistent(3)));
        // Duplicate index.
        let mut r = Reassembler::new();
        r.push(&recs[0].0, &recs[0].1).unwrap();
        assert_eq!(
            r.push(&recs[0].0, &recs[0].1),
            Err(VideoError::Inconsistent(3))
        );
        // Last fragment longer than the fragment size.
        let mut r = Reassembler::new();
        r.push(&recs[2].0, &[0; 11]).unwrap();
        assert_eq!(
            r.push(&recs[0].0, &recs[0].1),
            Err(VideoError::Inconsistent(3))
        );
        // Empty fragment.
        let mut r = Reassembler::new();
        assert_eq!(r.push(&recs[0].0, &[]), Err(VideoError::Inconsistent(3)));
        // Invalid index.
        let mut h = recs[0].0;
        h.frag_index = 3;
        assert_eq!(r.push(&h, &[1]), Err(VideoError::Inconsistent(3)));
    }

    #[test]
    fn two_mib_limit_section_6_2() {
        let mut r = Reassembler::new();
        let h = VideoHeader {
            codec: 1,
            frame_id: 1,
            capture_ms: 0,
            width: 1,
            height: 1,
            frag_index: 0,
            frag_count: 40,
        };
        // 40 fragments of 60000 bytes would be 2.4 MB: rejected on first fragment.
        assert_eq!(r.push(&h, &vec![0; 60_000]), Err(VideoError::FrameTooLarge));
        // Detected once the fragment size is learnt from a later fragment.
        let mut last = h;
        last.frag_index = 39;
        assert_eq!(r.push(&last, &[0; 10]), Ok(None));
        let mut mid = h;
        mid.frag_index = 1;
        assert_eq!(
            r.push(&mid, &vec![0; 60_000]),
            Err(VideoError::FrameTooLarge)
        );
        // Exactly 2 MiB reassembles.
        let f = frame(9, MAX_FRAME_LEN);
        let mut r = Reassembler::new();
        let mut out = None;
        for rec in fragment(&f, 60_000).unwrap() {
            out = r.push_record(&rec).unwrap().or(out);
        }
        assert_eq!(out, Some(f));
    }

    #[test]
    fn non_video_records_are_ignored() {
        let mut r = Reassembler::new();
        assert_eq!(r.push_record(&Record::Ping(1)), Ok(None));
    }

    #[test]
    fn round_trip_property_through_encoded_records() {
        let mut seed = 42u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut r = Reassembler::new();
        for id in 0..40u32 {
            let len = (next() % 20_000 + 1) as usize;
            let size = (next() % 5_000 + 1) as usize;
            let f = frame(id, len);
            let mut got = None;
            for rec in fragment(&f, size).unwrap() {
                let bytes = rec.encode().unwrap();
                let decoded = Record::decode(&bytes).unwrap();
                got = r.push_record(&decoded).unwrap().or(got);
            }
            assert_eq!(got, Some(f));
        }
    }
}

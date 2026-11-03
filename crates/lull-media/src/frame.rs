// SPDX-License-Identifier: AGPL-3.0-or-later
use std::ops::BitOr;

/// Audio codec identifier (media.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodecId(pub u8);

impl CodecId {
    /// Codec 1: Opus.
    pub const OPUS: Self = Self(1);
    /// Codec 2: IMA ADPCM at 16 kHz.
    pub const ADPCM: Self = Self(2);
}

/// AUDIO record flags (protocol.md §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FrameFlags(pub u8);

impl FrameFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// Bit 0: sent late because sound activation opened.
    pub const PRE_ROLL: Self = Self(1);
    /// Bit 1: first frame of a burst (after silence, gap or talk start).
    pub const BURST_START: Self = Self(2);

    /// Raw flag byte.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether all bits of `other` are set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the pre-roll bit is set.
    pub const fn is_pre_roll(self) -> bool {
        self.contains(Self::PRE_ROLL)
    }

    /// Whether the first-of-burst bit is set.
    pub const fn is_burst_start(self) -> bool {
        self.contains(Self::BURST_START)
    }
}

impl BitOr for FrameFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// One encoded 20 ms audio frame, mirroring the AUDIO record body
/// (protocol.md §6.1) without the stream byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    /// Codec of `payload`.
    pub codec: CodecId,
    /// Pre-roll / first-of-burst flags.
    pub flags: FrameFlags,
    /// Sequence number, +1 per captured frame.
    pub seq: u32,
    /// Capture time in 1/48000 s units, wrapping.
    pub timestamp: u32,
    /// One codec frame of 20 ms.
    pub payload: Vec<u8>,
}

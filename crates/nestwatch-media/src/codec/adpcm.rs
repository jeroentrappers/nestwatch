// SPDX-License-Identifier: AGPL-3.0-or-later
//! Codec 2: IMA/DVI ADPCM at 16 kHz (media.md §2.1, §2.2).
//!
//! Frame layout (164 bytes): predictor `i16` big-endian (sample 0, exact),
//! `step_index` `u8` (0..88), reserved `u8` (0), then 160 bytes holding the
//! 4-bit codes of samples 1..319, high nibble first; the last low nibble is 0.

use super::{AudioDecoder, AudioEncoder};
use crate::{CodecId, FRAME_SAMPLES, MediaError, PcmFrame};

/// Encoded size of one frame.
pub const FRAME_BYTES: usize = 4 + 160;
/// Highest valid step index.
pub const MAX_STEP_INDEX: u8 = 88;
/// Samples examined when the encoder picks the initial step index.
const INDEX_SEARCH_SAMPLES: usize = 32;

/// Standard IMA ADPCM step-size table.
pub const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// Standard IMA ADPCM index adjustment, indexed by code magnitude 0..7.
pub const INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Debug, Clone, Copy)]
struct State {
    predictor: i32,
    index: i32,
}

impl State {
    fn apply(&mut self, code: u8) -> i16 {
        let step = STEP_TABLE[self.index as usize];
        let mut diff = step >> 3;
        if code & 4 != 0 {
            diff += step;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 1 != 0 {
            diff += step >> 2;
        }
        if code & 8 != 0 {
            self.predictor -= diff;
        } else {
            self.predictor += diff;
        }
        self.predictor = self.predictor.clamp(-32768, 32767);
        self.index = (self.index + INDEX_TABLE[usize::from(code & 7)]).clamp(0, 88);
        self.predictor as i16
    }

    fn encode(&mut self, sample: i16) -> u8 {
        let step = STEP_TABLE[self.index as usize];
        let mut diff = i32::from(sample) - self.predictor;
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut s = step;
        if diff >= s {
            code |= 4;
            diff -= s;
        }
        s >>= 1;
        if diff >= s {
            code |= 2;
            diff -= s;
        }
        s >>= 1;
        if diff >= s {
            code |= 1;
        }
        self.apply(code);
        code
    }
}

/// Picks the initial step index that minimises the squared coding error over
/// the first 32 samples (ties: lowest index). Deterministic and history-free,
/// so each frame's encoding depends only on its own samples.
fn choose_step_index(pcm: &PcmFrame) -> u8 {
    let mut best = (u64::MAX, 0u8);
    for idx in 0..=MAX_STEP_INDEX {
        let mut st = State {
            predictor: i32::from(pcm[0]),
            index: i32::from(idx),
        };
        let mut err = 0u64;
        for &s in &pcm[1..INDEX_SEARCH_SAMPLES] {
            st.encode(s);
            let e = i64::from(s) - i64::from(st.predictor);
            err += (e * e) as u64;
        }
        if err < best.0 {
            best = (err, idx);
        }
    }
    best.1
}

/// Encodes one frame to the §2.1 bitstream.
pub fn encode_frame(pcm: &PcmFrame) -> [u8; FRAME_BYTES] {
    let mut out = [0u8; FRAME_BYTES];
    let index = choose_step_index(pcm);
    out[0..2].copy_from_slice(&pcm[0].to_be_bytes());
    out[2] = index;
    let mut st = State {
        predictor: i32::from(pcm[0]),
        index: i32::from(index),
    };
    for (n, &s) in pcm[1..].iter().enumerate() {
        let code = st.encode(s);
        let byte = &mut out[4 + n / 2];
        if n % 2 == 0 {
            *byte |= code << 4;
        } else {
            *byte |= code;
        }
    }
    out
}

/// Decodes one §2.1 frame. The reserved byte is ignored.
pub fn decode_frame(payload: &[u8]) -> Result<PcmFrame, MediaError> {
    if payload.len() != FRAME_BYTES {
        return Err(MediaError::InvalidPayload(
            "codec 2 frame must be 164 bytes",
        ));
    }
    if payload[2] > MAX_STEP_INDEX {
        return Err(MediaError::InvalidPayload("codec 2 step_index above 88"));
    }
    let mut pcm = [0i16; FRAME_SAMPLES];
    let first = i16::from_be_bytes([payload[0], payload[1]]);
    pcm[0] = first;
    let mut st = State {
        predictor: i32::from(first),
        index: i32::from(payload[2]),
    };
    for (n, s) in pcm[1..].iter_mut().enumerate() {
        let byte = payload[4 + n / 2];
        let code = if n % 2 == 0 { byte >> 4 } else { byte & 0x0f };
        *s = st.apply(code);
    }
    Ok(pcm)
}

/// Codec 2 encoder. Stateless: every frame is coded independently.
#[derive(Debug, Clone, Default)]
pub struct AdpcmEncoder;

impl AdpcmEncoder {
    /// Creates an encoder.
    pub fn new() -> Self {
        Self
    }
}

impl AudioEncoder for AdpcmEncoder {
    fn codec(&self) -> CodecId {
        CodecId::ADPCM
    }

    fn encode(&mut self, pcm: &PcmFrame) -> Result<Vec<u8>, MediaError> {
        Ok(encode_frame(pcm).to_vec())
    }
}

/// Maximum number of concealed frames before silence (§2.2).
pub const MAX_CONCEALED: u8 = 3;
/// Attenuation per repetition: 6 dB, 12 dB, 18 dB (§2.2).
const CONCEAL_GAINS: [f32; 3] = [0.501_187_2, 0.251_188_64, 0.125_892_54];

/// Codec 2 decoder with §2.2 concealment.
#[derive(Debug, Clone, Default)]
pub struct AdpcmDecoder {
    last: Option<PcmFrame>,
    repeats: u8,
}

impl AdpcmDecoder {
    /// Creates a decoder.
    pub fn new() -> Self {
        Self::default()
    }
}

impl AudioDecoder for AdpcmDecoder {
    fn codec(&self) -> CodecId {
        CodecId::ADPCM
    }

    fn decode(&mut self, payload: &[u8]) -> Result<PcmFrame, MediaError> {
        let pcm = decode_frame(payload)?;
        self.last = Some(pcm);
        self.repeats = 0;
        Ok(pcm)
    }

    fn conceal(&mut self, _next: Option<&[u8]>) -> Option<PcmFrame> {
        let last = self.last?;
        if self.repeats >= MAX_CONCEALED {
            return None;
        }
        let g = CONCEAL_GAINS[usize::from(self.repeats)];
        self.repeats += 1;
        let mut out = [0i16; FRAME_SAMPLES];
        for (o, &s) in out.iter_mut().zip(last.iter()) {
            *o = (f32::from(s) * g).round() as i16;
        }
        Some(out)
    }

    fn reset(&mut self) {
        self.last = None;
        self.repeats = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::level_dbfs;

    fn snr_db(a: &PcmFrame, b: &PcmFrame) -> f64 {
        let (mut sig, mut err) = (0.0f64, 0.0f64);
        for (&x, &y) in a.iter().zip(b.iter()) {
            sig += f64::from(x).powi(2);
            err += (f64::from(x) - f64::from(y)).powi(2);
        }
        10.0 * (sig / err.max(1e-9)).log10()
    }

    fn sine(freq: f64, amp: f64, offset: usize) -> PcmFrame {
        let mut f = [0i16; FRAME_SAMPLES];
        for (i, s) in f.iter_mut().enumerate() {
            let t = (i + offset) as f64 / 16_000.0;
            *s = (amp * (2.0 * std::f64::consts::PI * freq * t).sin()).round() as i16;
        }
        f
    }

    fn noise(seed: u32, amp: i32) -> PcmFrame {
        let mut x = seed;
        let mut f = [0i16; FRAME_SAMPLES];
        for s in f.iter_mut() {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *s = ((x >> 16) as i32 % (2 * amp + 1) - amp) as i16;
        }
        f
    }

    #[test]
    fn media_2_1_header_layout() {
        let mut pcm = sine(440.0, 8000.0, 7);
        pcm[0] = -1234;
        let enc = encode_frame(&pcm);
        assert_eq!(enc.len(), 164);
        assert_eq!(&enc[0..2], &(-1234i16).to_be_bytes());
        assert!(enc[2] <= 88);
        assert_eq!(enc[3], 0);
        assert_eq!(enc[163] & 0x0f, 0, "final low nibble is 0");
        let dec = decode_frame(&enc).unwrap();
        assert_eq!(dec[0], -1234, "first sample stored exactly");
    }

    #[test]
    fn media_2_1_nibble_order_high_first() {
        // Hand-built frame: predictor 0, index 0 (step 7).
        // Code 4 → +7 + 0 = +7 (diff = step + step>>3 = 7 + 0)
        let mut payload = [0u8; FRAME_BYTES];
        payload[4] = 0x4C; // sample 1: code 4 (+), sample 2: code 0xC (-)
        let pcm = decode_frame(&payload).unwrap();
        // step 7: code 4 → diff = 7>>3 + 7 = 7 → pred 7, index 0+2 = 2.
        assert_eq!(pcm[1], 7);
        // step 9: code 0xC → diff = 9>>3 + 9 = 10 → pred -3.
        assert_eq!(pcm[2], -3);
    }

    #[test]
    fn media_2_1_ignores_final_low_nibble() {
        let enc = encode_frame(&sine(300.0, 5000.0, 0));
        let mut tampered = enc;
        tampered[163] |= 0x0f;
        assert_eq!(
            decode_frame(&enc).unwrap(),
            decode_frame(&tampered).unwrap()
        );
    }

    #[test]
    fn media_2_1_rejects_bad_frames() {
        assert!(decode_frame(&[0u8; 163]).is_err());
        let mut p = [0u8; FRAME_BYTES];
        p[2] = 89;
        assert!(decode_frame(&p).is_err());
    }

    #[test]
    fn media_2_1_round_trip_snr_sine() {
        // Bounds: > 25 dB up to 1 kHz, > 20 dB for a loud 3 kHz tone.
        for (freq, amp, bound) in [
            (200.0, 3000.0, 25.0),
            (1000.0, 10_000.0, 25.0),
            (3000.0, 16_000.0, 20.0),
        ] {
            for k in 0..5 {
                let pcm = sine(freq, amp, k * FRAME_SAMPLES);
                let dec = decode_frame(&encode_frame(&pcm)).unwrap();
                let snr = snr_db(&pcm, &dec);
                assert!(snr > bound, "{freq} Hz frame {k}: {snr:.1} dB");
            }
        }
    }

    #[test]
    fn media_2_1_round_trip_snr_noise() {
        // White noise is the worst case for ADPCM; bound: > 10 dB.
        for seed in 1..6 {
            let pcm = noise(seed, 8000);
            let dec = decode_frame(&encode_frame(&pcm)).unwrap();
            let snr = snr_db(&pcm, &dec);
            assert!(snr > 10.0, "seed {seed}: {snr:.1} dB");
        }
    }

    #[test]
    fn media_2_1_extremes_do_not_overflow() {
        let mut square = [0i16; FRAME_SAMPLES];
        for (i, s) in square.iter_mut().enumerate() {
            *s = if (i / 8) % 2 == 0 { 32767 } else { -32768 };
        }
        let dec = decode_frame(&encode_frame(&square)).unwrap();
        assert!(snr_db(&square, &dec) > 5.0);
        let silence = [0i16; FRAME_SAMPLES];
        let dec = decode_frame(&encode_frame(&silence)).unwrap();
        assert!(dec.iter().all(|&s| s.abs() <= 1));
    }

    #[test]
    fn media_2_1_frames_are_independent() {
        let a = sine(500.0, 9000.0, 0);
        let b = sine(500.0, 9000.0, FRAME_SAMPLES);
        let ea = encode_frame(&a);
        let eb = encode_frame(&b);
        // Encoding b does not depend on having encoded a.
        let mut enc = AdpcmEncoder::new();
        enc.encode(&a).unwrap();
        assert_eq!(enc.encode(&b).unwrap(), eb.to_vec());
        // Decoding b alone, or after a corrupted a, gives the same result.
        let alone = AdpcmDecoder::new().decode(&eb).unwrap();
        let mut dec = AdpcmDecoder::new();
        let mut bad = ea;
        for byte in &mut bad[4..] {
            *byte ^= 0xA5;
        }
        dec.decode(&bad).unwrap();
        assert_eq!(dec.decode(&eb).unwrap(), alone);
    }

    #[test]
    fn media_2_2_concealment_attenuates_then_silence() {
        let pcm = sine(400.0, 12_000.0, 0);
        let mut dec = AdpcmDecoder::new();
        assert!(dec.conceal(None).is_none(), "nothing to repeat yet");
        let base = level_dbfs(&dec.decode(&encode_frame(&pcm)).unwrap());
        for n in 1..=3 {
            let c = dec.conceal(None).unwrap();
            let l = level_dbfs(&c);
            assert!((base - l - 6.0 * n as f32).abs() < 0.1, "rep {n}: {l}");
        }
        assert!(dec.conceal(None).is_none());
        // A good frame resets the count.
        dec.decode(&encode_frame(&pcm)).unwrap();
        assert!(dec.conceal(None).is_some());
    }
}

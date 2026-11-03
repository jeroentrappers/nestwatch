// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal CBOR map helpers over `minicbor` for control messages
//! (protocol.md §7.1, RFC 8949).

use minicbor::data::Type;
use minicbor::{Decoder, Encoder};

use crate::control::ControlError;

/// Decodes an IEEE 754 binary16 value (RFC 8949 Appendix D).
pub(crate) fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f64::from(bits & 0x03ff);
    let mag = match exp {
        0 => mant * 2f64.powi(-24),
        31 if mant == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        e => (1024.0 + mant) * 2f64.powi(e - 25),
    };
    sign * mag
}

/// Returns the binary16 encoding of `x` if it represents `x` exactly.
pub(crate) fn f32_to_f16_exact(x: f32) -> Option<u16> {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        return (mant == 0).then_some(sign | 0x7c00);
    }
    if exp == 0 {
        return (mant == 0).then_some(sign);
    }
    let e = exp - 127;
    if (-14..=15).contains(&e) {
        return (mant & 0x1fff == 0)
            .then(|| sign | (((e + 15) as u16) << 10) | (mant >> 13) as u16);
    }
    if (-24..=-15).contains(&e) {
        let full = mant | 0x0080_0000;
        let shift = (-e - 1) as u32;
        return (full & ((1 << shift) - 1) == 0).then(|| sign | (full >> shift) as u16);
    }
    None
}

/// Builder for a CBOR map in core deterministic encoding (RFC 8949 §4.2.1):
/// definite lengths, shortest integer and float forms, keys sorted by their
/// encoded bytes.
#[derive(Debug, Default)]
pub(crate) struct MapWriter {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

fn enc() -> Encoder<Vec<u8>> {
    Encoder::new(Vec::new())
}

impl MapWriter {
    pub(crate) fn new(t: &str) -> Self {
        let mut w = Self::default();
        w.str("t", t);
        w
    }

    fn put(&mut self, key: &str, value: Vec<u8>) {
        let mut k = enc();
        k.str(key).expect("vec write is infallible");
        self.entries.push((k.into_writer(), value));
    }

    pub(crate) fn str(&mut self, key: &str, v: &str) {
        let mut e = enc();
        e.str(v).expect("vec write is infallible");
        self.put(key, e.into_writer());
    }

    pub(crate) fn uint(&mut self, key: &str, v: u64) {
        let mut e = enc();
        e.u64(v).expect("vec write is infallible");
        self.put(key, e.into_writer());
    }

    pub(crate) fn bool(&mut self, key: &str, v: bool) {
        let mut e = enc();
        e.bool(v).expect("vec write is infallible");
        self.put(key, e.into_writer());
    }

    pub(crate) fn null(&mut self, key: &str) {
        let mut e = enc();
        e.null().expect("vec write is infallible");
        self.put(key, e.into_writer());
    }

    pub(crate) fn float(&mut self, key: &str, v: f32) {
        let mut e = enc();
        match f32_to_f16_exact(v) {
            Some(h) => {
                let w = e.writer_mut();
                w.push(0xf9);
                w.extend_from_slice(&h.to_be_bytes());
            }
            None => {
                e.f32(v).expect("vec write is infallible");
            }
        }
        self.put(key, e.into_writer());
    }

    pub(crate) fn opt_str(&mut self, key: &str, v: Option<&str>) {
        match v {
            Some(s) => self.str(key, s),
            None => self.null(key),
        }
    }

    pub(crate) fn str_array<'a>(&mut self, key: &str, v: impl ExactSizeIterator<Item = &'a str>) {
        let mut e = enc();
        e.array(v.len() as u64).expect("vec write is infallible");
        for s in v {
            e.str(s).expect("vec write is infallible");
        }
        self.put(key, e.into_writer());
    }

    pub(crate) fn uint_array(&mut self, key: &str, v: &[u8]) {
        let mut e = enc();
        e.array(v.len() as u64).expect("vec write is infallible");
        for n in v {
            e.u8(*n).expect("vec write is infallible");
        }
        self.put(key, e.into_writer());
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut e = enc();
        e.map(self.entries.len() as u64)
            .expect("vec write is infallible");
        let mut out = e.into_writer();
        for (k, v) in self.entries {
            out.extend_from_slice(&k);
            out.extend_from_slice(&v);
        }
        out
    }
}

fn malformed(err: minicbor::decode::Error) -> ControlError {
    ControlError::Malformed(err.to_string())
}

/// A decoded CBOR map with text keys, holding the position of each value.
#[derive(Debug)]
pub(crate) struct MapReader<'b> {
    input: &'b [u8],
    entries: Vec<(&'b str, usize)>,
}

impl<'b> MapReader<'b> {
    /// Parses `input` as exactly one CBOR map. Non-text keys are skipped.
    pub(crate) fn parse(input: &'b [u8]) -> Result<Self, ControlError> {
        let mut d = Decoder::new(input);
        let len = match d.datatype().map_err(malformed)? {
            Type::Map | Type::MapIndef => d.map().map_err(malformed)?,
            _ => return Err(ControlError::NotAMap),
        };
        let mut entries: Vec<(&'b str, usize)> = Vec::new();
        let mut remaining = len;
        loop {
            match remaining {
                Some(0) => break,
                Some(ref mut n) => *n -= 1,
                None => {
                    if d.datatype().map_err(malformed)? == Type::Break {
                        d.set_position(d.position() + 1);
                        break;
                    }
                }
            }
            let key = if d.datatype().map_err(malformed)? == Type::String {
                Some(d.str().map_err(malformed)?)
            } else {
                d.skip().map_err(malformed)?;
                None
            };
            let pos = d.position();
            d.skip().map_err(malformed)?;
            if let Some(k) = key {
                if entries.iter().any(|(e, _)| *e == k) {
                    return Err(ControlError::DuplicateKey(k.to_owned()));
                }
                entries.push((k, pos));
            }
        }
        if d.position() != input.len() {
            return Err(ControlError::TrailingBytes);
        }
        Ok(Self { input, entries })
    }

    fn at(&self, key: &str) -> Option<Decoder<'b>> {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|&(_, pos)| {
                let mut d = Decoder::new(self.input);
                d.set_position(pos);
                d
            })
    }

    /// Value of `key` unless it is absent or `null`.
    fn present(&self, key: &str) -> Option<Decoder<'b>> {
        self.at(key)
            .filter(|d| !matches!(d.datatype(), Ok(Type::Null | Type::Undefined)))
    }

    #[cfg(test)]
    pub(crate) fn has(&self, key: &str) -> bool {
        self.at(key).is_some()
    }

    fn require(&self, key: &'static str) -> Result<Decoder<'b>, ControlError> {
        self.at(key).ok_or(ControlError::MissingKey(key))
    }

    pub(crate) fn str(&self, key: &'static str) -> Result<&'b str, ControlError> {
        read_str(&mut self.require(key)?, key)
    }

    pub(crate) fn opt_str(&self, key: &'static str) -> Result<Option<&'b str>, ControlError> {
        self.present(key)
            .map(|mut d| read_str(&mut d, key))
            .transpose()
    }

    /// A key that must be present but may be `null`.
    pub(crate) fn nullable_str(&self, key: &'static str) -> Result<Option<&'b str>, ControlError> {
        self.require(key)?;
        self.opt_str(key)
    }

    pub(crate) fn uint(&self, key: &'static str) -> Result<u64, ControlError> {
        read_uint(&mut self.require(key)?, key)
    }

    pub(crate) fn opt_uint(&self, key: &'static str) -> Result<Option<u64>, ControlError> {
        self.present(key)
            .map(|mut d| read_uint(&mut d, key))
            .transpose()
    }

    pub(crate) fn nullable_uint(&self, key: &'static str) -> Result<Option<u64>, ControlError> {
        self.require(key)?;
        self.opt_uint(key)
    }

    pub(crate) fn bool(&self, key: &'static str) -> Result<bool, ControlError> {
        let mut d = self.require(key)?;
        d.bool().map_err(|_| ControlError::WrongType(key))
    }

    pub(crate) fn nullable_bool(&self, key: &'static str) -> Result<Option<bool>, ControlError> {
        self.require(key)?;
        self.present(key)
            .map(|mut d| d.bool().map_err(|_| ControlError::WrongType(key)))
            .transpose()
    }

    pub(crate) fn float(&self, key: &'static str) -> Result<f64, ControlError> {
        let mut d = self.require(key)?;
        match d.datatype().map_err(|_| ControlError::WrongType(key))? {
            Type::F16 => {
                let p = d.position();
                let b = &self.input[p + 1..p + 3];
                Ok(f16_to_f64(u16::from_be_bytes([b[0], b[1]])))
            }
            Type::F32 | Type::F64 => d.f64().map_err(|_| ControlError::WrongType(key)),
            _ => Err(ControlError::WrongType(key)),
        }
    }

    pub(crate) fn str_array(&self, key: &'static str) -> Result<Vec<&'b str>, ControlError> {
        let mut d = self.require(key)?;
        let mut out = Vec::new();
        read_array(&mut d, key, |d| {
            out.push(read_str(d, key)?);
            Ok(())
        })?;
        Ok(out)
    }

    pub(crate) fn uint_array(&self, key: &'static str) -> Result<Vec<u64>, ControlError> {
        let mut d = self.require(key)?;
        let mut out = Vec::new();
        read_array(&mut d, key, |d| {
            out.push(read_uint(d, key)?);
            Ok(())
        })?;
        Ok(out)
    }
}

fn read_str<'b>(d: &mut Decoder<'b>, key: &'static str) -> Result<&'b str, ControlError> {
    if d.datatype().ok() != Some(Type::String) {
        return Err(ControlError::WrongType(key));
    }
    d.str().map_err(|_| ControlError::WrongType(key))
}

fn read_uint(d: &mut Decoder<'_>, key: &'static str) -> Result<u64, ControlError> {
    match d.datatype() {
        Ok(Type::U8 | Type::U16 | Type::U32 | Type::U64) => {
            d.u64().map_err(|_| ControlError::WrongType(key))
        }
        _ => Err(ControlError::WrongType(key)),
    }
}

fn read_array<'b>(
    d: &mut Decoder<'b>,
    key: &'static str,
    mut item: impl FnMut(&mut Decoder<'b>) -> Result<(), ControlError>,
) -> Result<(), ControlError> {
    let len = match d.datatype() {
        Ok(Type::Array | Type::ArrayIndef) => {
            d.array().map_err(|_| ControlError::WrongType(key))?
        }
        _ => return Err(ControlError::WrongType(key)),
    };
    match len {
        Some(n) => {
            for _ in 0..n {
                item(d)?;
            }
        }
        None => {
            while d.datatype().map_err(|_| ControlError::WrongType(key))? != Type::Break {
                item(d)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_rfc8949_appendix_a() {
        let cases: &[(f32, Option<u16>)] = &[
            (0.0, Some(0x0000)),
            (-0.0, Some(0x8000)),
            (1.0, Some(0x3c00)),
            (1.5, Some(0x3e00)),
            (65504.0, Some(0x7bff)),
            (5.960_464_5e-8, Some(0x0001)),
            (0.000_061_035_156, Some(0x0400)),
            (-4.0, Some(0xc400)),
            (-50.0, Some(0xd240)),
            (f32::INFINITY, Some(0x7c00)),
            (f32::NEG_INFINITY, Some(0xfc00)),
            (100_000.0, None),
            (1.1, None),
            (3.402_823_5e38, None),
            (-42.5, Some(0xd150)),
        ];
        for &(x, want) in cases {
            assert_eq!(f32_to_f16_exact(x), want, "{x}");
            if let Some(h) = want {
                assert_eq!(f16_to_f64(h), f64::from(x), "{x}");
            }
        }
        assert!(f16_to_f64(0x7e00).is_nan());
    }

    #[test]
    fn f16_exhaustive_round_trip() {
        for h in 0u16..=0xffff {
            let x = f16_to_f64(h);
            if x.is_nan() {
                continue;
            }
            assert_eq!(f32_to_f16_exact(x as f32), Some(h), "{h:#06x}");
        }
    }

    #[test]
    fn map_writer_sorts_keys_deterministically() {
        let mut w = MapWriter::new("x");
        w.uint("w", 1);
        w.uint("h", 2);
        w.uint("aa", 3);
        // {"h":2,"t":"x","w":1,"aa":3}
        assert_eq!(
            w.finish(),
            [
                0xa4, 0x61, b'h', 0x02, 0x61, b't', 0x61, b'x', 0x61, b'w', 0x01, 0x62, b'a', b'a',
                0x03
            ]
        );
    }

    #[test]
    fn reader_handles_indefinite_maps_and_skips_non_text_keys() {
        // {_ "t": "x", 1: [_ 1, 2], "k": true }
        let bytes = [
            0xbf, 0x61, b't', 0x61, b'x', 0x01, 0x9f, 0x01, 0x02, 0xff, 0x61, b'k', 0xf5, 0xff,
        ];
        let m = MapReader::parse(&bytes).unwrap();
        assert_eq!(m.str("t").unwrap(), "x");
        assert!(m.bool("k").unwrap());
        assert!(!m.has("z"));
    }

    #[test]
    fn reader_rejects_duplicates_and_trailing() {
        let dup = [0xa2, 0x61, b't', 0x00, 0x61, b't', 0x01];
        assert!(matches!(
            MapReader::parse(&dup),
            Err(ControlError::DuplicateKey(k)) if k == "t"
        ));
        let trailing = [0xa0, 0x00];
        assert_eq!(
            MapReader::parse(&trailing).unwrap_err(),
            ControlError::TrailingBytes
        );
        assert_eq!(
            MapReader::parse(&[0x80]).unwrap_err(),
            ControlError::NotAMap
        );
        assert!(matches!(
            MapReader::parse(&[0xa1, 0x61]),
            Err(ControlError::Malformed(_))
        ));
        assert!(matches!(
            MapReader::parse(&[]),
            Err(ControlError::Malformed(_))
        ));
    }

    #[test]
    fn reader_survives_deep_nesting_in_unknown_values() {
        let depth = 100_000;
        let mut bytes = vec![0xa2, 0x61, b't', 0x61, b'x', 0x61, b'z'];
        bytes.extend(std::iter::repeat_n(0x81, depth));
        bytes.push(0x00);
        let m = MapReader::parse(&bytes).unwrap();
        assert_eq!(m.str("t").unwrap(), "x");
    }
}

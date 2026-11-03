// SPDX-License-Identifier: AGPL-3.0-or-later
//! DNS-SD TXT record for `_nestwatch._tcp.local.` (protocol.md §3.4).

use thiserror::Error;

use crate::device_id::DeviceId;

/// DNS-SD service type (protocol.md §3.4).
pub const SERVICE_TYPE: &str = "_nestwatch._tcp.local.";

/// Errors parsing a TXT record.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TxtError {
    /// `v` is not `1`.
    #[error("unsupported TXT version")]
    UnsupportedVersion,
    /// A required key is missing.
    #[error("missing TXT key {0:?}")]
    MissingKey(&'static str),
    /// A known key has an invalid value.
    #[error("invalid TXT value for {0:?}")]
    InvalidValue(&'static str),
}

/// Station discovery data (protocol.md §3.4). mDNS data is never trusted
/// for authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationTxt {
    /// Station device ID.
    pub id: DeviceId,
    /// Pairing mode open.
    pub pair: bool,
    /// HTTP port for the browser listener.
    pub http: Option<u16>,
    /// HTTPS port for the browser listener.
    pub https: Option<u16>,
}

fn port(v: &str, key: &'static str) -> Result<u16, TxtError> {
    match v.parse::<u16>() {
        Ok(p) if p != 0 && v.bytes().all(|b| b.is_ascii_digit()) => Ok(p),
        _ => Err(TxtError::InvalidValue(key)),
    }
}

impl StationTxt {
    /// TXT key/value pairs in the order `v, id, pair, http, https`.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("v".to_owned(), "1".to_owned()),
            ("id".to_owned(), self.id.to_string()),
            (
                "pair".to_owned(),
                if self.pair { "1" } else { "0" }.to_owned(),
            ),
        ];
        if let Some(p) = self.http {
            out.push(("http".to_owned(), p.to_string()));
        }
        if let Some(p) = self.https {
            out.push(("https".to_owned(), p.to_string()));
        }
        out
    }

    /// TXT strings `key=value` in the order of [`to_pairs`](Self::to_pairs).
    pub fn to_strings(&self) -> Vec<String> {
        self.to_pairs()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect()
    }

    /// Parses key/value pairs. Keys are case-insensitive and only the first
    /// occurrence of a key counts (RFC 6763 §6.4); unknown keys are ignored.
    pub fn from_pairs<'a>(
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, TxtError> {
        let (mut v, mut id, mut pair, mut http, mut https) = (None, None, None, None, None);
        for (k, val) in pairs {
            let slot = match k.to_ascii_lowercase().as_str() {
                "v" => &mut v,
                "id" => &mut id,
                "pair" => &mut pair,
                "http" => &mut http,
                "https" => &mut https,
                _ => continue,
            };
            slot.get_or_insert(val);
        }
        if v.ok_or(TxtError::MissingKey("v"))? != "1" {
            return Err(TxtError::UnsupportedVersion);
        }
        let id = DeviceId::parse(id.ok_or(TxtError::MissingKey("id"))?)
            .map_err(|_| TxtError::InvalidValue("id"))?;
        let pair = match pair.ok_or(TxtError::MissingKey("pair"))? {
            "1" => true,
            "0" => false,
            _ => return Err(TxtError::InvalidValue("pair")),
        };
        Ok(Self {
            id,
            pair,
            http: http.map(|p| port(p, "http")).transpose()?,
            https: https.map(|p| port(p, "https")).transpose()?,
        })
    }

    /// Parses TXT strings of the form `key=value`; strings without `=` are
    /// ignored.
    pub fn from_strings<'a>(strings: impl IntoIterator<Item = &'a str>) -> Result<Self, TxtError> {
        Self::from_pairs(strings.into_iter().filter_map(|s| s.split_once('=')))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StationTxt {
        StationTxt {
            id: DeviceId::parse("mfrggzdfmztwq2lk").unwrap(),
            pair: true,
            http: Some(47180),
            https: Some(47181),
        }
    }

    #[test]
    fn build_section_3_4() {
        assert_eq!(
            sample().to_strings(),
            [
                "v=1",
                "id=mfrggzdfmztwq2lk",
                "pair=1",
                "http=47180",
                "https=47181"
            ]
        );
        let mut closed = sample();
        closed.pair = false;
        assert_eq!(closed.to_strings()[2], "pair=0");
    }

    #[test]
    fn round_trip() {
        let s = sample();
        let strings = s.to_strings();
        assert_eq!(
            StationTxt::from_strings(strings.iter().map(String::as_str)).unwrap(),
            s
        );
        let mut no_ports = s.clone();
        no_ports.http = None;
        no_ports.https = None;
        let strings = no_ports.to_strings();
        assert_eq!(strings.len(), 3);
        assert_eq!(
            StationTxt::from_strings(strings.iter().map(String::as_str)).unwrap(),
            no_ports
        );
    }

    #[test]
    fn tolerant_parsing() {
        let t = StationTxt::from_strings([
            "future=x",
            "V=1",
            "ID=mfrggzdfmztwq2lk",
            "pair=0",
            "pair=1",
            "flag",
            "https=47185",
        ])
        .unwrap();
        assert!(!t.pair);
        assert_eq!((t.http, t.https), (None, Some(47185)));
    }

    #[test]
    fn errors() {
        let base = ["v=1", "id=mfrggzdfmztwq2lk", "pair=1"];
        let parse = |s: &[&str]| StationTxt::from_strings(s.iter().copied());
        assert_eq!(parse(&base[1..]), Err(TxtError::MissingKey("v")));
        assert_eq!(
            parse(&["v=2", base[1], base[2]]),
            Err(TxtError::UnsupportedVersion)
        );
        assert_eq!(parse(&[base[0], base[2]]), Err(TxtError::MissingKey("id")));
        assert_eq!(
            parse(&[base[0], "id=short", base[2]]),
            Err(TxtError::InvalidValue("id"))
        );
        assert_eq!(parse(&base[..2]), Err(TxtError::MissingKey("pair")));
        assert_eq!(
            parse(&[base[0], base[1], "pair=yes"]),
            Err(TxtError::InvalidValue("pair"))
        );
        for bad in ["http=0", "http=70000", "http=+1", "http="] {
            assert_eq!(
                parse(&[base[0], base[1], base[2], bad]),
                Err(TxtError::InvalidValue("http")),
                "{bad}"
            );
        }
    }
}

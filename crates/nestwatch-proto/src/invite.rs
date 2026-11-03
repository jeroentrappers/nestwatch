// SPDX-License-Identifier: AGPL-3.0-or-later
//! Invitation URIs (protocol.md §3.5).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use thiserror::Error;

use crate::control::MAX_NAME_LEN;
use crate::endpoint::{Endpoint, EndpointError};

/// URI prefix of an invitation (protocol.md §3.5).
pub const INVITE_PREFIX: &str = "nestwatch://invite?";
/// Station public key length (protocol.md §3.5).
pub const STATION_KEY_LEN: usize = 32;
/// Invite token length (protocol.md §3.5).
pub const INVITE_TOKEN_LEN: usize = 16;
/// Maximum endpoints in an invitation (protocol.md §3.5).
pub const MAX_INVITE_ENDPOINTS: usize = 8;

/// Characters left unescaped: RFC 3986 unreserved (`ALPHA DIGIT - . _ ~`).
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Errors building or parsing an invitation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InviteError {
    /// Not a `nestwatch://invite?` URI (or no fragment in a web URL).
    #[error("not a nestwatch invitation")]
    NotAnInvitation,
    /// `v` is not `1`.
    #[error("unsupported invitation version")]
    UnsupportedVersion,
    /// A required parameter is missing.
    #[error("missing parameter {0:?}")]
    MissingParameter(&'static str),
    /// A single-valued parameter occurs twice.
    #[error("duplicate parameter {0:?}")]
    DuplicateParameter(&'static str),
    /// A parameter value is malformed.
    #[error("invalid parameter {0:?}")]
    InvalidParameter(&'static str),
    /// `n` exceeds 64 bytes.
    #[error("name exceeds 64 bytes")]
    NameTooLong,
    /// More than 8 endpoints when building.
    #[error("more than 8 endpoints")]
    TooManyEndpoints,
    /// An `a` value is not a valid endpoint.
    #[error("invalid endpoint: {0}")]
    Endpoint(#[from] EndpointError),
}

/// A parsed invitation (protocol.md §3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    /// Station static public key.
    pub station_key: [u8; STATION_KEY_LEN],
    /// Random invite token.
    pub token: [u8; INVITE_TOKEN_LEN],
    /// Station display name.
    pub name: String,
    /// Endpoints in preference order, at most [`MAX_INVITE_ENDPOINTS`].
    pub endpoints: Vec<Endpoint>,
}

fn decode_fixed<const N: usize>(v: &str, key: &'static str) -> Result<[u8; N], InviteError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(v)
        .map_err(|_| InviteError::InvalidParameter(key))?;
    bytes
        .try_into()
        .map_err(|_| InviteError::InvalidParameter(key))
}

fn set_once<'a>(
    slot: &mut Option<&'a str>,
    v: &'a str,
    key: &'static str,
) -> Result<(), InviteError> {
    if slot.replace(v).is_some() {
        Err(InviteError::DuplicateParameter(key))
    } else {
        Ok(())
    }
}

fn decode_component(v: &str, key: &'static str) -> Result<String, InviteError> {
    percent_decode_str(v)
        .decode_utf8()
        .map(|c| c.into_owned())
        .map_err(|_| InviteError::InvalidParameter(key))
}

impl Invitation {
    /// Builds the `nestwatch://invite?...` URI with parameters in the order
    /// `v, s, t, n, a...`.
    pub fn to_uri(&self) -> Result<String, InviteError> {
        if self.name.len() > MAX_NAME_LEN {
            return Err(InviteError::NameTooLong);
        }
        if self.endpoints.len() > MAX_INVITE_ENDPOINTS {
            return Err(InviteError::TooManyEndpoints);
        }
        let mut uri = format!(
            "{INVITE_PREFIX}v=1&s={}&t={}&n={}",
            URL_SAFE_NO_PAD.encode(self.station_key),
            URL_SAFE_NO_PAD.encode(self.token),
            utf8_percent_encode(&self.name, COMPONENT),
        );
        for e in &self.endpoints {
            uri.push_str("&a=");
            uri.extend(utf8_percent_encode(&e.to_string(), COMPONENT));
        }
        Ok(uri)
    }

    /// Parses a `nestwatch://invite?...` URI. Unknown parameters are ignored;
    /// `v`, `s` and `t` are required; endpoints after the eighth are ignored.
    pub fn parse(uri: &str) -> Result<Self, InviteError> {
        let uri = uri.trim();
        let query = uri
            .get(..INVITE_PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(INVITE_PREFIX))
            .map(|_| &uri[INVITE_PREFIX.len()..])
            .ok_or(InviteError::NotAnInvitation)?;
        let query = query.split('#').next().unwrap_or_default();
        let (mut v, mut s, mut t, mut n) = (None, None, None, None);
        let mut endpoints = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "v" => set_once(&mut v, value, "v")?,
                "s" => set_once(&mut s, value, "s")?,
                "t" => set_once(&mut t, value, "t")?,
                "n" => set_once(&mut n, value, "n")?,
                "a" if endpoints.len() < MAX_INVITE_ENDPOINTS => {
                    endpoints.push(Endpoint::parse(&decode_component(value, "a")?)?);
                }
                _ => {}
            }
        }
        if v.ok_or(InviteError::MissingParameter("v"))? != "1" {
            return Err(InviteError::UnsupportedVersion);
        }
        let station_key = decode_fixed(s.ok_or(InviteError::MissingParameter("s"))?, "s")?;
        let token = decode_fixed(t.ok_or(InviteError::MissingParameter("t"))?, "t")?;
        let name = n
            .map(|n| decode_component(n, "n"))
            .transpose()?
            .unwrap_or_default();
        if name.len() > MAX_NAME_LEN {
            return Err(InviteError::NameTooLong);
        }
        Ok(Self {
            station_key,
            token,
            name,
            endpoints,
        })
    }

    /// Builds the browser URL `<origin>/#<percent-encoded invitation>`, e.g.
    /// with origin `http://192.168.1.20:47180` (protocol.md §3.5).
    pub fn to_web_url(&self, origin: &str) -> Result<String, InviteError> {
        let uri = self.to_uri()?;
        Ok(format!(
            "{}/#{}",
            origin.trim_end_matches('/'),
            utf8_percent_encode(&uri, COMPONENT)
        ))
    }

    /// Parses an invitation carried percent-encoded in a URL fragment
    /// (protocol.md §3.5). Only the fragment is examined.
    pub fn parse_web_url(url: &str) -> Result<Self, InviteError> {
        let (_, fragment) = url.split_once('#').ok_or(InviteError::NotAnInvitation)?;
        let decoded = percent_decode_str(fragment)
            .decode_utf8()
            .map_err(|_| InviteError::NotAnInvitation)?;
        Self::parse(&decoded)
    }

    /// Accepts either a `nestwatch://` URI or a web URL with the invitation in its
    /// fragment.
    pub fn parse_any(input: &str) -> Result<Self, InviteError> {
        let input = input.trim();
        const SCHEME: &str = "nestwatch://";
        if input
            .get(..SCHEME.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(SCHEME))
        {
            Self::parse(input)
        } else {
            Self::parse_web_url(input)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Invitation {
        Invitation {
            station_key: core::array::from_fn(|i| i as u8),
            token: core::array::from_fn(|i| 0xF0 ^ i as u8),
            name: "Baby's room & co".into(),
            endpoints: vec![
                Endpoint::parse("nursery.tail1234.ts.net:47100").unwrap(),
                Endpoint::parse("100.101.102.103:47100").unwrap(),
                Endpoint::parse("[fd7a:115c:a1e0::7]:47100").unwrap(),
            ],
        }
    }

    #[test]
    fn build_section_3_5() {
        let uri = sample().to_uri().unwrap();
        assert_eq!(
            uri,
            "nestwatch://invite?v=1&s=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8\
             &t=8PHy8_T19vf4-fr7_P3-_w&n=Baby%27s%20room%20%26%20co\
             &a=nursery.tail1234.ts.net%3A47100&a=100.101.102.103%3A47100\
             &a=%5Bfd7a%3A115c%3Aa1e0%3A%3A7%5D%3A47100"
        );
        assert_eq!(Invitation::parse(&uri).unwrap(), sample());
    }

    #[test]
    fn unknown_and_reordered_parameters() {
        let s = sample();
        let uri = format!(
            "NESTWATCH://INVITE?x=9&a=10.0.0.1%3A1&t={}&future=%FF&n=Caf%C3%A9&s={}&v=1",
            URL_SAFE_NO_PAD.encode(s.token),
            URL_SAFE_NO_PAD.encode(s.station_key)
        );
        let inv = Invitation::parse(&uri).unwrap();
        assert_eq!(inv.name, "Café");
        assert_eq!(inv.endpoints, [Endpoint::parse("10.0.0.1:1").unwrap()]);
    }

    #[test]
    fn missing_name_and_endpoints() {
        let mut s = sample();
        s.endpoints.clear();
        s.name.clear();
        let uri = s.to_uri().unwrap();
        assert!(uri.ends_with("&n="));
        assert_eq!(Invitation::parse(&uri).unwrap(), s);
        let without_n = uri.trim_end_matches("&n=");
        assert_eq!(Invitation::parse(without_n).unwrap(), s);
    }

    #[test]
    fn endpoint_limit() {
        let mut s = sample();
        s.endpoints = (1..=9)
            .map(|i| Endpoint::parse(&format!("10.0.0.{i}:47100")).unwrap())
            .collect();
        assert_eq!(s.to_uri(), Err(InviteError::TooManyEndpoints));
        let extra = s.endpoints.pop().unwrap();
        let mut uri = s.to_uri().unwrap();
        uri.push_str(&format!("&a={}", extra.to_string().replace(':', "%3A")));
        assert_eq!(Invitation::parse(&uri).unwrap().endpoints.len(), 8);
    }

    #[test]
    fn name_limit() {
        let mut s = sample();
        s.name = "ü".repeat(32);
        let uri = s.to_uri().unwrap();
        assert_eq!(Invitation::parse(&uri).unwrap().name, s.name);
        s.name.push('x');
        assert_eq!(s.to_uri(), Err(InviteError::NameTooLong));
        let long = uri.replace("&n=", "&n=x");
        assert_eq!(Invitation::parse(&long), Err(InviteError::NameTooLong));
    }

    #[test]
    fn invalid_invitations() {
        let good = sample().to_uri().unwrap();
        let s_param = format!("s={}", URL_SAFE_NO_PAD.encode(sample().station_key));
        let t_param = format!("t={}", URL_SAFE_NO_PAD.encode(sample().token));
        let cases: Vec<(String, InviteError)> = vec![
            ("https://x/".into(), InviteError::NotAnInvitation),
            ("nestwatch://pair?v=1".into(), InviteError::NotAnInvitation),
            (good.replace("v=1", "v=2"), InviteError::UnsupportedVersion),
            (good.replace("v=1&", ""), InviteError::MissingParameter("v")),
            (
                good.replace(&format!("{s_param}&"), ""),
                InviteError::MissingParameter("s"),
            ),
            (
                good.replace(&format!("{t_param}&"), ""),
                InviteError::MissingParameter("t"),
            ),
            (format!("{good}&v=1"), InviteError::DuplicateParameter("v")),
            (
                good.replace(&s_param, "s=AAEC"),
                InviteError::InvalidParameter("s"),
            ),
            (
                good.replace(&s_param, &format!("{s_param}=")),
                InviteError::InvalidParameter("s"),
            ),
            (
                good.replace(&t_param, "t=+/+/"),
                InviteError::InvalidParameter("t"),
            ),
            (format!("{good}&n=x"), InviteError::DuplicateParameter("n")),
            (
                good.replace("&n=", "&n=%FF"),
                InviteError::InvalidParameter("n"),
            ),
            (
                format!("{good}&a=nohost"),
                InviteError::Endpoint(EndpointError::MissingPort),
            ),
            (
                format!("{good}&a="),
                InviteError::Endpoint(EndpointError::MissingPort),
            ),
        ];
        for (uri, want) in cases {
            assert_eq!(Invitation::parse(&uri), Err(want), "{uri}");
        }
    }

    #[test]
    fn web_url_variant() {
        let s = sample();
        let url = s.to_web_url("http://192.168.1.20:47180/").unwrap();
        assert!(
            url.starts_with("http://192.168.1.20:47180/#nestwatch%3A%2F%2Finvite%3Fv%3D1%26s%3D")
        );
        assert!(!url[url.find('#').unwrap() + 1..].contains(['&', '=', '#', '?']));
        assert_eq!(Invitation::parse_web_url(&url).unwrap(), s);
        assert_eq!(Invitation::parse_any(&url).unwrap(), s);
        assert_eq!(Invitation::parse_any(&s.to_uri().unwrap()).unwrap(), s);
        assert_eq!(
            Invitation::parse_web_url("http://host:47180/"),
            Err(InviteError::NotAnInvitation)
        );
        assert_eq!(
            Invitation::parse_web_url("http://host:47180/#nope"),
            Err(InviteError::NotAnInvitation)
        );
    }

    #[test]
    fn round_trip_property_names() {
        let names = [
            "",
            "a",
            "Nursery",
            "Kinderkamer 2",
            "日本語の部屋",
            "a+b=c&d#e%f?g/h",
            "🍼🍼",
        ];
        for n in names {
            let mut s = sample();
            s.name = n.into();
            let uri = s.to_uri().unwrap();
            assert_eq!(Invitation::parse(&uri).unwrap(), s, "{n}");
            assert_eq!(
                Invitation::parse_web_url(&s.to_web_url("https://h:47181").unwrap()).unwrap(),
                s
            );
        }
    }
}

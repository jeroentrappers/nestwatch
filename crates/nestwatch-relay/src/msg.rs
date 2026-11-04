// SPDX-License-Identifier: AGPL-3.0-or-later
//! NWR/1 rendezvous messages (rendezvous.md §4): CBOR maps with text keys
//! in binary WebSocket messages.
//!
//! Encoding is RFC 8949 core deterministic (as PROTO-3). Decoding follows
//! protocol.md §7.1: unknown keys are ignored, unknown message types decode
//! to [`Message::Unknown`], missing or malformed required keys are errors,
//! duplicate keys and trailing bytes are errors. `register` and `connect`
//! with a version other than 1 decode to [`DecodeError::Version`] before any
//! other key is checked, so the server can answer `version`.

use std::collections::BTreeMap;
use std::fmt;

use minicbor::data::Type;
use minicbor::{Decoder, Encoder};
use thiserror::Error;

use crate::MailboxId;

/// Protocol version carried in `register` and `connect` (rendezvous.md §4).
pub const VERSION: u64 = 1;
/// Session ID length (rendezvous.md §4.2).
pub const SESSION_ID_LEN: usize = 16;
/// `connect.nonce` length (rendezvous.md §4.2).
pub const NONCE_LEN: usize = 16;
/// Mailbox secret length (rendezvous.md §3.2).
pub const SECRET_LEN: usize = 32;
/// Candidates per message at most (OPEN-QUESTIONS RELAY-4).
pub const MAX_CANDIDATES: usize = 16;
/// Bytes per candidate string at most (RELAY-4).
pub const MAX_CANDIDATE_LEN: usize = 300;
/// Bytes of `observed` at most (RELAY-4).
pub const MAX_OBSERVED_LEN: usize = 64;
/// Bytes of `error.code` at most (RELAY-4).
pub const MAX_CODE_LEN: usize = 64;
/// Encoded NWR/1 message size at most; larger messages are a protocol error.
pub const MAX_MESSAGE_LEN: usize = 16 * 1024;

/// A relay session ID chosen by the server (rendezvous.md §4.2).
pub type SessionId = [u8; SESSION_ID_LEN];

/// Error codes of `{ t: "error", code }` (rendezvous.md §4.1, §4.2 and
/// OPEN-QUESTIONS RELAY-3 for the codes beyond the spec's).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// The mailbox is registered with a different secret.
    MailboxTaken,
    /// Too many `register`/`connect` requests from this address.
    RateLimited,
    /// Unsupported `v`.
    Version,
    /// The mailbox is not registered (no station online).
    Offline,
    /// A pipe limit was reached (per mailbox or per client address).
    Limit,
    /// The mailbox's monthly transfer budget is used up.
    Quota,
    /// `accept` names an unknown or expired session.
    Session,
    /// A newer registration of the same mailbox replaced this one.
    Replaced,
    /// The peer sent something that is not valid NWR/1.
    Protocol,
    /// The server is shutting down or overloaded.
    Unavailable,
    /// Any other code, kept verbatim.
    Other(String),
}

impl ErrorCode {
    /// Wire form.
    pub fn as_str(&self) -> &str {
        match self {
            Self::MailboxTaken => "mailbox-taken",
            Self::RateLimited => "rate-limited",
            Self::Version => "version",
            Self::Offline => "offline",
            Self::Limit => "limit",
            Self::Quota => "quota",
            Self::Session => "session",
            Self::Replaced => "replaced",
            Self::Protocol => "protocol",
            Self::Unavailable => "unavailable",
            Self::Other(s) => s,
        }
    }

    /// Parses the wire form; unknown codes become [`ErrorCode::Other`].
    pub fn parse(s: &str) -> Self {
        match s {
            "mailbox-taken" => Self::MailboxTaken,
            "rate-limited" => Self::RateLimited,
            "version" => Self::Version,
            "offline" => Self::Offline,
            "limit" => Self::Limit,
            "quota" => Self::Quota,
            "session" => Self::Session,
            "replaced" => Self::Replaced,
            "protocol" => Self::Protocol,
            "unavailable" => Self::Unavailable,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The mailbox secret; `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub [u8; SECRET_LEN]);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// One NWR/1 message (rendezvous.md §4.1, §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// station → server: claim or refresh the mailbox (§4.1). `v` is 1.
    Register {
        /// Mailbox ID.
        mailbox: MailboxId,
        /// Mailbox secret (TOFU, §3.2).
        secret: Secret,
    },
    /// server → station: registration accepted.
    Registered {
        /// The station's address as the server sees it.
        observed: String,
    },
    /// server → client: request refused; the server closes afterwards.
    Error {
        /// Reason.
        code: ErrorCode,
    },
    /// listener → server: reach a station (§4.2). `v` is 1.
    Connect {
        /// Mailbox ID from the invitation.
        mailbox: MailboxId,
        /// Random per attempt (RELAY-5).
        nonce: [u8; NONCE_LEN],
        /// The listener's direct candidates (§5).
        candidates: Vec<String>,
    },
    /// server → station (control): a listener wants to connect.
    Incoming {
        /// Session to accept within 30 s.
        session: SessionId,
        /// The listener's candidates.
        candidates: Vec<String>,
        /// The listener's address as the server sees it (RELAY-6).
        observed: String,
    },
    /// station → server (new connection): accept a session.
    Accept {
        /// Session from `incoming`.
        session: SessionId,
        /// The station's candidates.
        candidates: Vec<String>,
    },
    /// server → listener: the station accepted; the pipe starts.
    Accepted {
        /// The station's candidates.
        candidates: Vec<String>,
        /// The station's address as the server sees it (RELAY-6).
        observed: String,
    },
    /// client → server: application-level keep-alive for clients that
    /// cannot send WebSocket pings (browsers; OPEN-QUESTIONS WEBST-3).
    Ping,
    /// server → client: answer to [`Message::Ping`].
    Pong,
    /// A message type this implementation does not know (ignored).
    Unknown {
        /// Its `t`.
        t: String,
    },
}

/// Errors decoding or encoding a [`Message`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// Not a CBOR map with text keys, or trailing bytes.
    #[error("malformed CBOR: {0}")]
    Cbor(String),
    /// A key occurs twice.
    #[error("duplicate key {0:?}")]
    DuplicateKey(String),
    /// A required key is missing.
    #[error("missing key {0:?}")]
    Missing(&'static str),
    /// A key has the wrong type, length or range.
    #[error("invalid value for {0:?}")]
    Invalid(&'static str),
    /// `register`/`connect` with an unsupported `v`.
    #[error("unsupported version {0}")]
    Version(u64),
    /// Message larger than [`MAX_MESSAGE_LEN`].
    #[error("message too large")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Bytes(Vec<u8>),
    Uint(u64),
    TextArray(Vec<String>),
    Other,
}

fn cbor_err(e: impl fmt::Display) -> DecodeError {
    DecodeError::Cbor(e.to_string())
}

fn decode_value(d: &mut Decoder<'_>) -> Result<Value, DecodeError> {
    Ok(match d.datatype().map_err(cbor_err)? {
        Type::String | Type::StringIndef => {
            let mut s = String::new();
            for part in d.str_iter().map_err(cbor_err)? {
                s.push_str(part.map_err(cbor_err)?);
            }
            Value::Text(s)
        }
        Type::Bytes | Type::BytesIndef => {
            let mut b = Vec::new();
            for part in d.bytes_iter().map_err(cbor_err)? {
                b.extend_from_slice(part.map_err(cbor_err)?);
            }
            Value::Bytes(b)
        }
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => Value::Uint(d.u64().map_err(cbor_err)?),
        Type::Array | Type::ArrayIndef => {
            let len = d.array().map_err(cbor_err)?;
            let mut items = Vec::new();
            let mut all_text = true;
            let mut i = 0u64;
            loop {
                match len {
                    Some(n) if i >= n => break,
                    None if d.datatype().map_err(cbor_err)? == Type::Break => {
                        d.skip().map_err(cbor_err)?;
                        break;
                    }
                    _ => {}
                }
                match decode_value(d)? {
                    Value::Text(s) => items.push(s),
                    _ => all_text = false,
                }
                i += 1;
                if i > (MAX_MESSAGE_LEN as u64) {
                    return Err(DecodeError::TooLarge);
                }
            }
            if all_text {
                Value::TextArray(items)
            } else {
                Value::Other
            }
        }
        _ => {
            d.skip().map_err(cbor_err)?;
            Value::Other
        }
    })
}

/// Reads a top-level map into key → value. Non-text keys are skipped like
/// unknown keys (as PROTO-3).
fn decode_map(bytes: &[u8]) -> Result<BTreeMap<String, Value>, DecodeError> {
    if bytes.len() > MAX_MESSAGE_LEN {
        return Err(DecodeError::TooLarge);
    }
    let mut d = Decoder::new(bytes);
    let len = d.map().map_err(cbor_err)?;
    let mut map = BTreeMap::new();
    let mut i = 0u64;
    loop {
        match len {
            Some(n) if i >= n => break,
            None if d.datatype().map_err(cbor_err)? == Type::Break => {
                d.skip().map_err(cbor_err)?;
                break;
            }
            _ => {}
        }
        i += 1;
        let key = match d.datatype().map_err(cbor_err)? {
            Type::String | Type::StringIndef => match decode_value(&mut d)? {
                Value::Text(s) => Some(s),
                _ => None,
            },
            _ => {
                d.skip().map_err(cbor_err)?;
                None
            }
        };
        let value = decode_value(&mut d)?;
        if let Some(key) = key
            && map.insert(key.clone(), value).is_some()
        {
            return Err(DecodeError::DuplicateKey(key));
        }
    }
    if d.position() != bytes.len() {
        return Err(DecodeError::Cbor("trailing bytes".into()));
    }
    Ok(map)
}

struct Fields(BTreeMap<String, Value>);

impl Fields {
    fn get(&self, key: &'static str) -> Result<&Value, DecodeError> {
        self.0.get(key).ok_or(DecodeError::Missing(key))
    }

    fn text(&self, key: &'static str, max: usize) -> Result<String, DecodeError> {
        match self.get(key)? {
            Value::Text(s) if s.len() <= max => Ok(s.clone()),
            _ => Err(DecodeError::Invalid(key)),
        }
    }

    fn uint(&self, key: &'static str) -> Result<u64, DecodeError> {
        match self.get(key)? {
            Value::Uint(n) => Ok(*n),
            _ => Err(DecodeError::Invalid(key)),
        }
    }

    fn fixed<const N: usize>(&self, key: &'static str) -> Result<[u8; N], DecodeError> {
        match self.get(key)? {
            Value::Bytes(b) => b
                .as_slice()
                .try_into()
                .map_err(|_| DecodeError::Invalid(key)),
            _ => Err(DecodeError::Invalid(key)),
        }
    }

    fn candidates(&self) -> Result<Vec<String>, DecodeError> {
        const KEY: &str = "candidates";
        match self.get(KEY)? {
            Value::TextArray(items)
                if items.len() <= MAX_CANDIDATES
                    && items.iter().all(|c| c.len() <= MAX_CANDIDATE_LEN) =>
            {
                Ok(items.clone())
            }
            _ => Err(DecodeError::Invalid(KEY)),
        }
    }

    fn version(&self) -> Result<(), DecodeError> {
        match self.uint("v")? {
            VERSION => Ok(()),
            v => Err(DecodeError::Version(v)),
        }
    }
}

/// Map builder in core deterministic encoding.
#[derive(Default)]
struct MapWriter {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

impl MapWriter {
    fn new(t: &str) -> Self {
        let mut w = Self::default();
        w.put("t", |e| {
            e.str(t)?;
            Ok(())
        });
        w
    }

    fn put(
        &mut self,
        key: &str,
        f: impl FnOnce(
            &mut Encoder<Vec<u8>>,
        ) -> Result<(), minicbor::encode::Error<core::convert::Infallible>>,
    ) {
        let mut k = Encoder::new(Vec::new());
        k.str(key).expect("vec write is infallible");
        let mut v = Encoder::new(Vec::new());
        f(&mut v).expect("vec write is infallible");
        self.entries.push((k.into_writer(), v.into_writer()));
    }

    fn str(&mut self, key: &str, s: &str) {
        self.put(key, |e| {
            e.str(s)?;
            Ok(())
        });
    }

    fn bytes(&mut self, key: &str, b: &[u8]) {
        self.put(key, |e| {
            e.bytes(b)?;
            Ok(())
        });
    }

    fn uint(&mut self, key: &str, n: u64) {
        self.put(key, |e| {
            e.u64(n)?;
            Ok(())
        });
    }

    fn strs(&mut self, key: &str, items: &[String]) {
        self.put(key, |e| {
            e.array(items.len() as u64)?;
            for s in items {
                e.str(s)?;
            }
            Ok(())
        });
    }

    fn finish(mut self) -> Vec<u8> {
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut e = Encoder::new(Vec::new());
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

fn check_candidates(c: &[String]) -> Result<(), DecodeError> {
    if c.len() > MAX_CANDIDATES || c.iter().any(|s| s.len() > MAX_CANDIDATE_LEN) {
        return Err(DecodeError::Invalid("candidates"));
    }
    Ok(())
}

impl Message {
    /// The message type `t`.
    pub fn message_type(&self) -> &str {
        match self {
            Self::Register { .. } => "register",
            Self::Registered { .. } => "registered",
            Self::Error { .. } => "error",
            Self::Connect { .. } => "connect",
            Self::Incoming { .. } => "incoming",
            Self::Accept { .. } => "accept",
            Self::Accepted { .. } => "accepted",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::Unknown { t } => t,
        }
    }

    /// Encodes the message. Fails if a field exceeds its limit or for
    /// [`Message::Unknown`].
    pub fn encode(&self) -> Result<Vec<u8>, DecodeError> {
        let mut w = MapWriter::new(self.message_type());
        match self {
            Self::Register { mailbox, secret } => {
                w.uint("v", VERSION);
                w.bytes("mailbox", mailbox.as_bytes());
                w.bytes("secret", &secret.0);
            }
            Self::Registered { observed } => {
                if observed.len() > MAX_OBSERVED_LEN {
                    return Err(DecodeError::Invalid("observed"));
                }
                w.str("observed", observed);
            }
            Self::Error { code } => {
                if code.as_str().len() > MAX_CODE_LEN {
                    return Err(DecodeError::Invalid("code"));
                }
                w.str("code", code.as_str());
            }
            Self::Connect {
                mailbox,
                nonce,
                candidates,
            } => {
                check_candidates(candidates)?;
                w.uint("v", VERSION);
                w.bytes("mailbox", mailbox.as_bytes());
                w.bytes("nonce", nonce);
                w.strs("candidates", candidates);
            }
            Self::Incoming {
                session,
                candidates,
                observed,
            } => {
                check_candidates(candidates)?;
                if observed.len() > MAX_OBSERVED_LEN {
                    return Err(DecodeError::Invalid("observed"));
                }
                w.bytes("session", session);
                w.strs("candidates", candidates);
                w.str("observed", observed);
            }
            Self::Accept {
                session,
                candidates,
            } => {
                check_candidates(candidates)?;
                w.bytes("session", session);
                w.strs("candidates", candidates);
            }
            Self::Accepted {
                candidates,
                observed,
            } => {
                check_candidates(candidates)?;
                if observed.len() > MAX_OBSERVED_LEN {
                    return Err(DecodeError::Invalid("observed"));
                }
                w.strs("candidates", candidates);
                w.str("observed", observed);
            }
            Self::Ping | Self::Pong => {}
            Self::Unknown { .. } => return Err(DecodeError::Invalid("t")),
        }
        Ok(w.finish())
    }

    /// Decodes and validates one message.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let f = Fields(decode_map(bytes)?);
        let t = f.text("t", MAX_CODE_LEN)?;
        Ok(match t.as_str() {
            "register" => {
                f.version()?;
                Self::Register {
                    mailbox: MailboxId(f.fixed("mailbox")?),
                    secret: Secret(f.fixed("secret")?),
                }
            }
            "registered" => Self::Registered {
                observed: f.text("observed", MAX_OBSERVED_LEN)?,
            },
            "error" => Self::Error {
                code: ErrorCode::parse(&f.text("code", MAX_CODE_LEN)?),
            },
            "connect" => {
                f.version()?;
                Self::Connect {
                    mailbox: MailboxId(f.fixed("mailbox")?),
                    nonce: f.fixed("nonce")?,
                    candidates: f.candidates()?,
                }
            }
            "incoming" => Self::Incoming {
                session: f.fixed("session")?,
                candidates: f.candidates()?,
                observed: f.text("observed", MAX_OBSERVED_LEN)?,
            },
            "accept" => Self::Accept {
                session: f.fixed("session")?,
                candidates: f.candidates()?,
            },
            "accepted" => Self::Accepted {
                candidates: f.candidates()?,
                observed: f.text("observed", MAX_OBSERVED_LEN)?,
            },
            "ping" => Self::Ping,
            "pong" => Self::Pong,
            _ => Self::Unknown { t },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn mailbox() -> MailboxId {
        MailboxId(core::array::from_fn(|i| i as u8))
    }

    fn all() -> Vec<Message> {
        vec![
            Message::Register {
                mailbox: mailbox(),
                secret: Secret([0xAB; 32]),
            },
            Message::Registered {
                observed: "203.0.113.7:40123".into(),
            },
            Message::Error {
                code: ErrorCode::MailboxTaken,
            },
            Message::Error {
                code: ErrorCode::Other("future-code".into()),
            },
            Message::Connect {
                mailbox: mailbox(),
                nonce: [7; 16],
                candidates: vec![
                    "udp:192.168.1.20:50000".into(),
                    "udp:[2001:db8::1]:5".into(),
                ],
            },
            Message::Incoming {
                session: [9; 16],
                candidates: vec![],
                observed: "198.51.100.1:1234".into(),
            },
            Message::Accept {
                session: [9; 16],
                candidates: vec!["tcp:nursery.example:47100".into()],
            },
            Message::Accepted {
                candidates: vec!["udp:10.0.0.2:4000".into()],
                observed: "[2001:db8::2]:443".into(),
            },
            Message::Ping,
            Message::Pong,
        ]
    }

    #[test]
    fn keep_alive_messages_have_no_fields() {
        // {"t": "ping"} and {"t": "pong"}.
        assert_eq!(hex(&Message::Ping.encode().unwrap()), "a161746470696e67");
        assert_eq!(hex(&Message::Pong.encode().unwrap()), "a1617464706f6e67");
        assert_eq!(
            Message::decode(&Message::Pong.encode().unwrap()).unwrap(),
            Message::Pong
        );
    }

    #[test]
    fn round_trip_every_message() {
        for m in all() {
            let bytes = m.encode().unwrap();
            assert_eq!(Message::decode(&bytes).unwrap(), m, "{}", m.message_type());
        }
    }

    #[test]
    fn deterministic_encoding() {
        // Keys sorted by encoded bytes: shorter keys first, then bytewise.
        let m = Message::Accept {
            session: [1; 16],
            candidates: vec!["udp:1.2.3.4:5".into()],
        };
        assert_eq!(
            hex(&m.encode().unwrap()),
            concat!(
                "a3",
                "6174",
                "66616363657074",
                "6773657373696f6e",
                "5001010101010101010101010101010101",
                "6a63616e64696461746573",
                "816d7564703a312e322e332e343a35",
            )
        );
    }

    #[test]
    fn unknown_keys_and_types_are_ignored() {
        // { "t": "registered", "observed": "1.2.3.4:5", "x": [1, {}], 7: 1 }
        let mut e = Encoder::new(Vec::new());
        e.map(4).unwrap();
        e.str("x")
            .unwrap()
            .array(2)
            .unwrap()
            .u8(1)
            .unwrap()
            .map(0)
            .unwrap();
        e.str("observed").unwrap().str("1.2.3.4:5").unwrap();
        e.u8(7).unwrap().u8(1).unwrap();
        e.str("t").unwrap().str("registered").unwrap();
        assert_eq!(
            Message::decode(&e.into_writer()).unwrap(),
            Message::Registered {
                observed: "1.2.3.4:5".into()
            }
        );
        let mut e = Encoder::new(Vec::new());
        e.map(1).unwrap().str("t").unwrap().str("hello-v2").unwrap();
        assert_eq!(
            Message::decode(&e.into_writer()).unwrap(),
            Message::Unknown {
                t: "hello-v2".into()
            }
        );
        assert!(Message::Unknown { t: "x".into() }.encode().is_err());
    }

    #[test]
    fn indefinite_lengths_are_accepted() {
        let mut e = Encoder::new(Vec::new());
        e.begin_map().unwrap();
        e.str("t").unwrap().str("accepted").unwrap();
        e.str("candidates").unwrap().begin_array().unwrap();
        e.str("udp:1.2.3.4:5").unwrap().end().unwrap();
        e.str("observed").unwrap().str("").unwrap();
        e.end().unwrap();
        assert_eq!(
            Message::decode(&e.into_writer()).unwrap(),
            Message::Accepted {
                candidates: vec!["udp:1.2.3.4:5".into()],
                observed: String::new()
            }
        );
    }

    type Put<'a> = (&'a str, &'a dyn Fn(&mut Encoder<Vec<u8>>));

    fn map(entries: &[Put<'_>]) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        e.map(entries.len() as u64).unwrap();
        for (k, f) in entries {
            e.str(k).unwrap();
            f(&mut e);
        }
        e.into_writer()
    }

    #[test]
    fn version_is_checked_first() {
        let b = map(&[
            ("t", &|e| {
                e.str("register").unwrap();
            }),
            ("v", &|e| {
                e.u8(2).unwrap();
            }),
        ]);
        assert_eq!(Message::decode(&b), Err(DecodeError::Version(2)));
        let b = map(&[
            ("t", &|e| {
                e.str("connect").unwrap();
            }),
            ("v", &|e| {
                e.u8(9).unwrap();
            }),
            ("mailbox", &|e| {
                e.bytes(&[0; 3]).unwrap();
            }),
        ]);
        assert_eq!(Message::decode(&b), Err(DecodeError::Version(9)));
    }

    #[test]
    fn invalid_messages() {
        let reg = |mailbox: &[u8], secret: &[u8]| {
            map(&[
                ("t", &|e| {
                    e.str("register").unwrap();
                }),
                ("v", &|e| {
                    e.u8(1).unwrap();
                }),
                ("mailbox", &|e| {
                    e.bytes(mailbox).unwrap();
                }),
                ("secret", &|e| {
                    e.bytes(secret).unwrap();
                }),
            ])
        };
        assert!(Message::decode(&reg(&[0; 16], &[0; 32])).is_ok());
        assert_eq!(
            Message::decode(&reg(&[0; 15], &[0; 32])),
            Err(DecodeError::Invalid("mailbox"))
        );
        assert_eq!(
            Message::decode(&reg(&[0; 16], &[0; 31])),
            Err(DecodeError::Invalid("secret"))
        );
        let missing = map(&[
            ("t", &|e| {
                e.str("accept").unwrap();
            }),
            ("session", &|e| {
                e.bytes(&[0; 16]).unwrap();
            }),
        ]);
        assert_eq!(
            Message::decode(&missing),
            Err(DecodeError::Missing("candidates"))
        );
        let too_many = Message::Accept {
            session: [0; 16],
            candidates: vec!["udp:1.2.3.4:5".into(); MAX_CANDIDATES + 1],
        };
        assert!(too_many.encode().is_err());
        let not_text = map(&[
            ("t", &|e| {
                e.str("accept").unwrap();
            }),
            ("session", &|e| {
                e.bytes(&[0; 16]).unwrap();
            }),
            ("candidates", &|e| {
                e.array(1).unwrap().u8(1).unwrap();
            }),
        ]);
        assert_eq!(
            Message::decode(&not_text),
            Err(DecodeError::Invalid("candidates"))
        );
        let dup = map(&[
            ("t", &|e| {
                e.str("error").unwrap();
            }),
            ("t", &|e| {
                e.str("error").unwrap();
            }),
        ]);
        assert_eq!(
            Message::decode(&dup),
            Err(DecodeError::DuplicateKey("t".into()))
        );
        let mut trailing = Message::Error {
            code: ErrorCode::Offline,
        }
        .encode()
        .unwrap();
        trailing.push(0);
        assert!(matches!(
            Message::decode(&trailing),
            Err(DecodeError::Cbor(_))
        ));
        assert!(Message::decode(&[0x82, 0x01, 0x02]).is_err());
        assert!(Message::decode(b"NEST\x01\x01").is_err());
        assert_eq!(
            Message::decode(&vec![0; MAX_MESSAGE_LEN + 1]),
            Err(DecodeError::TooLarge)
        );
    }

    #[test]
    fn error_codes() {
        for c in [
            "mailbox-taken",
            "rate-limited",
            "version",
            "offline",
            "limit",
            "quota",
            "session",
            "replaced",
            "protocol",
            "unavailable",
            "x",
        ] {
            assert_eq!(ErrorCode::parse(c).as_str(), c);
        }
        assert_eq!(ErrorCode::parse("x"), ErrorCode::Other("x".into()));
    }

    #[test]
    fn secret_is_not_debug_printed() {
        let m = Message::Register {
            mailbox: mailbox(),
            secret: Secret([0x5A; 32]),
        };
        assert!(!format!("{m:?}").contains("90"));
        assert!(format!("{m:?}").contains("Secret(..)"));
    }
}

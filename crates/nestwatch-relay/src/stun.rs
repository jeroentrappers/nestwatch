// SPDX-License-Identifier: AGPL-3.0-or-later
//! STUN Binding (RFC 8489, the binding method only, no authentication) for
//! address discovery (rendezvous.md §5.1).
//!
//! The codec is pure and shared by the client ([`binding_request`],
//! [`parse_binding_response`]) and the server ([`answer`]).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use thiserror::Error;

/// Default STUN port (RFC 8489 §18.6, rendezvous.md §3.1).
pub const STUN_PORT: u16 = 3478;
/// The magic cookie (RFC 8489 §5).
pub const MAGIC_COOKIE: u32 = 0x2112_A442;
/// Header length.
pub const HEADER_LEN: usize = 20;
/// Transaction ID length.
pub const TXID_LEN: usize = 12;

const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const BINDING_ERROR: u16 = 0x0111;
const BINDING_INDICATION: u16 = 0x0011;

const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_ERROR_CODE: u16 = 0x0009;
const ATTR_UNKNOWN_ATTRIBUTES: u16 = 0x000A;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Comprehension-required attributes a server without authentication
/// accepts and ignores: USERNAME, MESSAGE-INTEGRITY, CHANGE-REQUEST (RFC
/// 5780), MESSAGE-INTEGRITY-SHA256, PRIORITY and USE-CANDIDATE (ICE, as sent
/// by browsers' connectivity checks).
const IGNORED_REQUIRED: [u16; 6] = [0x0006, 0x0008, 0x0003, 0x001C, 0x0024, 0x0025];

/// A STUN transaction ID.
pub type TransactionId = [u8; TXID_LEN];

/// Errors parsing a STUN message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StunError {
    /// Not a well-formed STUN message.
    #[error("not a STUN message")]
    Malformed,
    /// A response for a different transaction.
    #[error("transaction ID mismatch")]
    Mismatch,
    /// A Binding error response with this code.
    #[error("STUN error {0}")]
    ErrorResponse(u16),
    /// A success response without (XOR-)MAPPED-ADDRESS.
    #[error("no mapped address in response")]
    NoAddress,
    /// No response in time (client only).
    #[error("STUN request timed out")]
    Timeout,
    /// Socket failure (client only).
    #[error("STUN I/O error: {0}")]
    Io(String),
}

struct Header<'a> {
    msg_type: u16,
    txid: TransactionId,
    attrs: &'a [u8],
}

/// Whether `b` looks like a STUN message (zero top bits, magic cookie).
pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= HEADER_LEN && b[0] & 0xC0 == 0 && b[4..8] == MAGIC_COOKIE.to_be_bytes()
}

fn parse_header(b: &[u8]) -> Result<Header<'_>, StunError> {
    if !is_stun(b) {
        return Err(StunError::Malformed);
    }
    let msg_type = u16::from_be_bytes([b[0], b[1]]);
    let len = usize::from(u16::from_be_bytes([b[2], b[3]]));
    if len % 4 != 0 || b.len() != HEADER_LEN + len {
        return Err(StunError::Malformed);
    }
    Ok(Header {
        msg_type,
        txid: b[8..20].try_into().expect("12 bytes"),
        attrs: &b[HEADER_LEN..],
    })
}

/// Iterates `(type, value)` attributes; `None` on a malformed list.
fn attributes(mut a: &[u8]) -> Option<Vec<(u16, &[u8])>> {
    let mut out = Vec::new();
    while !a.is_empty() {
        if a.len() < 4 {
            return None;
        }
        let t = u16::from_be_bytes([a[0], a[1]]);
        let len = usize::from(u16::from_be_bytes([a[2], a[3]]));
        let padded = len.div_ceil(4) * 4;
        if a.len() < 4 + padded {
            return None;
        }
        out.push((t, &a[4..4 + len]));
        a = &a[4 + padded..];
    }
    Some(out)
}

fn message(msg_type: u16, txid: &TransactionId, attrs: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + attrs.len());
    out.extend_from_slice(&msg_type.to_be_bytes());
    out.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
    out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(txid);
    out.extend_from_slice(attrs);
    out
}

fn push_attr(out: &mut Vec<u8>, t: u16, value: &[u8]) {
    out.extend_from_slice(&t.to_be_bytes());
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
    out.resize(out.len() + (4 - value.len() % 4) % 4, 0);
}

/// A Binding Request with no attributes (RFC 8489 §6.1).
pub fn binding_request(txid: &TransactionId) -> Vec<u8> {
    message(BINDING_REQUEST, txid, &[])
}

/// A random transaction ID.
pub fn new_transaction_id() -> TransactionId {
    let mut t = [0; TXID_LEN];
    // A failing system RNG leaves zeros; the ID only matches responses.
    let _ = getrandom::fill(&mut t);
    t
}

fn xor_address(addr: SocketAddr, txid: &TransactionId) -> Vec<u8> {
    let port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
    let mut v = vec![0];
    match addr.ip().to_canonical() {
        IpAddr::V4(ip) => {
            v.push(0x01);
            v.extend_from_slice(&port.to_be_bytes());
            let x = u32::from(ip) ^ MAGIC_COOKIE;
            v.extend_from_slice(&x.to_be_bytes());
        }
        IpAddr::V6(ip) => {
            v.push(0x02);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = [0; 16];
            key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            key[4..].copy_from_slice(txid);
            let o = ip.octets();
            v.extend(o.iter().zip(key).map(|(a, k)| a ^ k));
        }
    }
    v
}

fn parse_address(v: &[u8], xor: Option<&TransactionId>) -> Option<SocketAddr> {
    if v.len() < 4 {
        return None;
    }
    let mut port = u16::from_be_bytes([v[2], v[3]]);
    if xor.is_some() {
        port ^= (MAGIC_COOKIE >> 16) as u16;
    }
    let ip = match (v[1], v.len()) {
        (0x01, 8) => {
            let mut x = u32::from_be_bytes(v[4..8].try_into().ok()?);
            if xor.is_some() {
                x ^= MAGIC_COOKIE;
            }
            IpAddr::V4(Ipv4Addr::from(x))
        }
        (0x02, 20) => {
            let mut o: [u8; 16] = v[4..20].try_into().ok()?;
            if let Some(txid) = xor {
                let mut key = [0; 16];
                key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
                key[4..].copy_from_slice(txid);
                for (b, k) in o.iter_mut().zip(key) {
                    *b ^= k;
                }
            }
            IpAddr::V6(Ipv6Addr::from(o))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// Reads the mapped address from a Binding response to `txid`
/// (XOR-MAPPED-ADDRESS, or MAPPED-ADDRESS from RFC 3489 servers).
pub fn parse_binding_response(b: &[u8], txid: &TransactionId) -> Result<SocketAddr, StunError> {
    let h = parse_header(b)?;
    if &h.txid != txid {
        return Err(StunError::Mismatch);
    }
    let attrs = attributes(h.attrs).ok_or(StunError::Malformed)?;
    match h.msg_type {
        BINDING_SUCCESS => {}
        BINDING_ERROR => {
            let code = attrs
                .iter()
                .find(|(t, _)| *t == ATTR_ERROR_CODE)
                .filter(|(_, v)| v.len() >= 4)
                .map(|(_, v)| u16::from(v[2] & 0x07) * 100 + u16::from(v[3]))
                .unwrap_or(0);
            return Err(StunError::ErrorResponse(code));
        }
        _ => return Err(StunError::Malformed),
    }
    let xor = attrs
        .iter()
        .find(|(t, _)| *t == ATTR_XOR_MAPPED_ADDRESS)
        .and_then(|(_, v)| parse_address(v, Some(txid)));
    let plain = || {
        attrs
            .iter()
            .find(|(t, _)| *t == ATTR_MAPPED_ADDRESS)
            .and_then(|(_, v)| parse_address(v, None))
    };
    xor.or_else(plain).ok_or(StunError::NoAddress)
}

/// The server side: the response to a datagram from `from`, or `None` if
/// it must be ignored (not a Binding request, indications, responses,
/// garbage). Requests with unknown comprehension-required attributes get a
/// 420 error (RFC 8489 §6.3.1); otherwise a success response carrying
/// XOR-MAPPED-ADDRESS (§14.2).
pub fn answer(b: &[u8], from: SocketAddr) -> Option<Vec<u8>> {
    let h = parse_header(b).ok()?;
    if h.msg_type == BINDING_INDICATION || h.msg_type != BINDING_REQUEST {
        return None;
    }
    let attrs = attributes(h.attrs)?;
    let unknown: Vec<u16> = attrs
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| *t < 0x8000 && !IGNORED_REQUIRED.contains(t))
        .collect();
    let mut out = Vec::new();
    if unknown.is_empty() {
        push_attr(
            &mut out,
            ATTR_XOR_MAPPED_ADDRESS,
            &xor_address(from, &h.txid),
        );
        return Some(message(BINDING_SUCCESS, &h.txid, &out));
    }
    let mut code = vec![0, 0, 4, 20];
    code.extend_from_slice(b"Unknown Attribute");
    push_attr(&mut out, ATTR_ERROR_CODE, &code);
    let list: Vec<u8> = unknown.iter().flat_map(|t| t.to_be_bytes()).collect();
    push_attr(&mut out, ATTR_UNKNOWN_ATTRIBUTES, &list);
    Some(message(BINDING_ERROR, &h.txid, &out))
}

#[cfg(feature = "native")]
mod client {
    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::net::UdpSocket;
    use tokio::time::Instant;

    use super::{StunError, binding_request, new_transaction_id, parse_binding_response};

    /// First retransmission timeout (RFC 8489 §6.2.1 recommends 500 ms).
    const INITIAL_RTO: Duration = Duration::from_millis(500);

    /// Sends a Binding Request from `socket` to `server` and returns the
    /// server-reflexive address (rendezvous.md §5.1). Retransmits with a
    /// doubling timeout until `timeout` passes. Unrelated datagrams arriving
    /// meanwhile are discarded, so call this before the socket carries other
    /// traffic.
    pub async fn stun_binding(
        socket: &UdpSocket,
        server: SocketAddr,
        timeout: Duration,
    ) -> Result<SocketAddr, StunError> {
        let txid = new_transaction_id();
        let request = binding_request(&txid);
        let deadline = Instant::now() + timeout;
        let mut rto = INITIAL_RTO;
        let mut buf = [0u8; 1500];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(StunError::Timeout);
            }
            socket
                .send_to(&request, server)
                .await
                .map_err(|e| StunError::Io(e.to_string()))?;
            let wait_until = (now + rto).min(deadline);
            rto *= 2;
            loop {
                let r = tokio::time::timeout_at(wait_until, socket.recv_from(&mut buf)).await;
                let Ok(r) = r else { break };
                let (n, _from) = match r {
                    Ok(x) => x,
                    // ICMP port unreachable surfaces as an error on some
                    // systems; keep waiting until the next retransmission.
                    Err(_) => continue,
                };
                match parse_binding_response(&buf[..n], &txid) {
                    Ok(a) => return Ok(a),
                    Err(e @ StunError::ErrorResponse(_)) => return Err(e),
                    Err(_) => {}
                }
            }
        }
    }
}

#[cfg(feature = "native")]
pub use client::stun_binding;

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// RFC 5769 §2.2: sample IPv4 response (192.0.2.1:32853).
    #[test]
    fn rfc5769_ipv4_response() {
        let b = hex("0101 003c 2112a442 b7e7a701 bc34d686 fa87dfae
             8022 000b 74657374 20766563 746f7220
             0020 0008 0001a147 e112a643
             0008 0014 2b91f599 fd9e90c3 8c7489f9 2af9ba53 f06be7d7
             8028 0004 c07d4c96");
        let txid: TransactionId = hex("b7e7a701bc34d686fa87dfae").try_into().unwrap();
        assert_eq!(
            parse_binding_response(&b, &txid).unwrap(),
            "192.0.2.1:32853".parse().unwrap()
        );
        assert_eq!(
            parse_binding_response(&b, &[0; 12]),
            Err(StunError::Mismatch)
        );
    }

    /// RFC 5769 §2.3: sample IPv6 response
    /// ([2001:db8:1234:5678:11:2233:4455:6677]:32853).
    #[test]
    fn rfc5769_ipv6_response() {
        let b = hex("0101 0048 2112a442 b7e7a701 bc34d686 fa87dfae
             8022 000b 74657374 20766563 746f7220
             0020 0014 0002a147 0113a9fa a5d3f179 bc25f4b5 bed2b9d9
             0008 0014 a382954e 4be67bf1 1784c97c 8292c275 bfe3ed41
             8028 0004 c8fb0b4c");
        let txid: TransactionId = hex("b7e7a701bc34d686fa87dfae").try_into().unwrap();
        assert_eq!(
            parse_binding_response(&b, &txid).unwrap(),
            "[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                .parse()
                .unwrap()
        );
    }

    #[test]
    fn server_answers_requests_with_the_source_address() {
        for from in ["203.0.113.9:40000", "[2001:db8::42]:3479"] {
            let from: SocketAddr = from.parse().unwrap();
            let txid = new_transaction_id();
            let resp = answer(&binding_request(&txid), from).unwrap();
            assert_eq!(parse_binding_response(&resp, &txid).unwrap(), from);
        }
        // IPv4-mapped sources of a dual-stack socket are answered as IPv4.
        let txid = [3; 12];
        let mapped: SocketAddr = "[::ffff:198.51.100.4]:5".parse().unwrap();
        let resp = answer(&binding_request(&txid), mapped).unwrap();
        assert_eq!(
            parse_binding_response(&resp, &txid).unwrap(),
            "198.51.100.4:5".parse().unwrap()
        );
    }

    #[test]
    fn server_ignores_what_is_not_a_binding_request() {
        let from: SocketAddr = "1.2.3.4:5".parse().unwrap();
        let txid = [1; 12];
        assert_eq!(answer(b"hello", from), None);
        assert_eq!(answer(&[0u8; 20], from), None);
        assert_eq!(answer(&message(BINDING_INDICATION, &txid, &[]), from), None);
        assert_eq!(answer(&message(BINDING_SUCCESS, &txid, &[]), from), None);
        // Allocate (TURN) is not served.
        assert_eq!(answer(&message(0x0003, &txid, &[]), from), None);
        let mut bad_len = binding_request(&txid);
        bad_len[3] = 4;
        assert_eq!(answer(&bad_len, from), None);
    }

    #[test]
    fn unknown_required_attributes_get_420() {
        let txid = [2; 12];
        let mut attrs = Vec::new();
        push_attr(&mut attrs, 0x7777, &[1, 2, 3]);
        push_attr(&mut attrs, 0x8022, b"client");
        let resp = answer(
            &message(BINDING_REQUEST, &txid, &attrs),
            "1.2.3.4:5".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(
            parse_binding_response(&resp, &txid),
            Err(StunError::ErrorResponse(420))
        );
        // ICE attributes and SOFTWARE are fine.
        let mut attrs = Vec::new();
        push_attr(&mut attrs, 0x0024, &[0, 0, 0, 1]);
        push_attr(&mut attrs, 0x0006, b"user:name");
        push_attr(&mut attrs, 0x8022, b"client");
        let resp = answer(
            &message(BINDING_REQUEST, &txid, &attrs),
            "1.2.3.4:5".parse().unwrap(),
        )
        .unwrap();
        assert!(parse_binding_response(&resp, &txid).is_ok());
    }

    #[test]
    fn plain_mapped_address_fallback() {
        let txid = [4; 12];
        let mut attrs = Vec::new();
        push_attr(
            &mut attrs,
            ATTR_MAPPED_ADDRESS,
            &[0, 1, 0x12, 0x34, 10, 0, 0, 7],
        );
        let resp = message(BINDING_SUCCESS, &txid, &attrs);
        assert_eq!(
            parse_binding_response(&resp, &txid).unwrap(),
            "10.0.0.7:4660".parse().unwrap()
        );
        assert_eq!(
            parse_binding_response(&message(BINDING_SUCCESS, &txid, &[]), &txid),
            Err(StunError::NoAddress)
        );
    }
}

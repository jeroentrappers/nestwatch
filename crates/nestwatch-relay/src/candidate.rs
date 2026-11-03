// SPDX-License-Identifier: AGPL-3.0-or-later
//! Direct-path candidates exchanged through the rendezvous (rendezvous.md
//! §4.2, §5): `udp:<ip>:<port>` and `tcp:<host>:<port>`.

use std::fmt;
use std::net::SocketAddr;

use nestwatch_proto::Endpoint;

use crate::msg::{MAX_CANDIDATE_LEN, MAX_CANDIDATES};

/// One candidate. IPv6 addresses use the bracketed form
/// (`udp:[2001:db8::1]:5000`, OPEN-QUESTIONS RELAY-4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Candidate {
    /// A UDP address for hole punching and QUIC (§5.1, §5.2): a local
    /// interface address or the server-reflexive address.
    Udp(SocketAddr),
    /// An NWP/1 TCP endpoint of the station (§5.2, last paragraph).
    Tcp(Endpoint),
}

impl Candidate {
    /// Parses the text form. Unknown prefixes, port 0, unspecified and
    /// multicast addresses are rejected.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() > MAX_CANDIDATE_LEN {
            return None;
        }
        if let Some(rest) = s.strip_prefix("udp:") {
            let addr: SocketAddr = rest.parse().ok()?;
            let ip = addr.ip().to_canonical();
            if addr.port() == 0 || ip.is_unspecified() || ip.is_multicast() {
                return None;
            }
            if let SocketAddr::V6(v6) = addr
                && v6.scope_id() != 0
            {
                return None;
            }
            return Some(Self::Udp(SocketAddr::new(ip, addr.port())));
        }
        if let Some(rest) = s.strip_prefix("tcp:") {
            return Endpoint::parse(rest).ok().map(Self::Tcp);
        }
        None
    }

    /// The UDP address, if this is a `udp:` candidate.
    pub fn udp(&self) -> Option<SocketAddr> {
        match self {
            Self::Udp(a) => Some(*a),
            Self::Tcp(_) => None,
        }
    }
}

impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Udp(a) => write!(f, "udp:{a}"),
            Self::Tcp(e) => write!(f, "tcp:{e}"),
        }
    }
}

/// Parses a received candidate list, skipping entries this version does not
/// understand (forward compatibility) and duplicates.
pub fn parse_candidates(list: &[String]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for c in list.iter().filter_map(|s| Candidate::parse(s)) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Text forms for a message, deduplicated and cut to [`MAX_CANDIDATES`].
pub fn candidate_strings(list: &[Candidate]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in list.iter().map(ToString::to_string) {
        if out.len() == MAX_CANDIDATES {
            break;
        }
        if s.len() <= MAX_CANDIDATE_LEN && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_forms() {
        for s in [
            "udp:192.168.1.20:50000",
            "udp:[2001:db8::1]:3478",
            "tcp:nursery.example:47100",
            "tcp:[fd7a:115c:a1e0::7]:47100",
            "udp:127.0.0.1:9",
        ] {
            assert_eq!(Candidate::parse(s).unwrap().to_string(), s);
        }
        assert_eq!(
            Candidate::parse("udp:[::ffff:10.0.0.1]:5")
                .unwrap()
                .to_string(),
            "udp:10.0.0.1:5"
        );
        for bad in [
            "udp:1.2.3.4",
            "udp:1.2.3.4:0",
            "udp:0.0.0.0:5",
            "udp:[::]:5",
            "udp:224.0.0.1:5",
            "udp:[fe80::1%2]:5",
            "udp:host.example:5",
            "tcp:host",
            "quic:1.2.3.4:5",
            "",
        ] {
            assert_eq!(Candidate::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn lists_skip_unknown_and_duplicates() {
        let got = parse_candidates(&[
            "udp:1.2.3.4:5".into(),
            "future:x".into(),
            "udp:1.2.3.4:5".into(),
            "tcp:a.example:1".into(),
        ]);
        assert_eq!(got.len(), 2);
        let many: Vec<Candidate> = (1..=40)
            .map(|p| Candidate::Udp(SocketAddr::from(([10, 0, 0, 1], p))))
            .collect();
        assert_eq!(candidate_strings(&many).len(), MAX_CANDIDATES);
    }
}

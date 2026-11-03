// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station endpoints and their advertised order (protocol.md §3.5).

use core::fmt;
use core::str::FromStr;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;

/// Default station TCP port (protocol.md §3.2).
pub const DEFAULT_TCP_PORT: u16 = 47100;
/// Default HTTP port for the browser listener (protocol.md §3.3).
pub const DEFAULT_HTTP_PORT: u16 = 47180;
/// Default HTTPS port for the browser listener (protocol.md §3.3).
pub const DEFAULT_HTTPS_PORT: u16 = 47181;
/// Number of ports tried upward from each default (protocol.md §3.2, §3.3).
pub const PORT_FALLBACK_COUNT: u16 = 10;
/// WebSocket path (protocol.md §3.3).
pub const WEBSOCKET_PATH: &str = "/nestwatch/1";

/// Error for a malformed endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EndpointError {
    /// Missing `:port`.
    #[error("endpoint has no port")]
    MissingPort,
    /// Port is not a number in 1..=65535.
    #[error("invalid port")]
    InvalidPort,
    /// Host is not a valid DNS name, IPv4 literal or bracketed IPv6 literal.
    #[error("invalid host")]
    InvalidHost,
}

/// Host part of an [`Endpoint`] (protocol.md §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    /// DNS name, stored in lower case without a trailing dot.
    Dns(String),
    /// IPv4 literal.
    Ipv4(Ipv4Addr),
    /// IPv6 literal (bracketed in text form).
    Ipv6(Ipv6Addr),
}

impl From<IpAddr> for Host {
    fn from(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(a) => Self::Ipv4(a),
            IpAddr::V6(a) => Self::Ipv6(a),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dns(n) => f.write_str(n),
            Self::Ipv4(a) => write!(f, "{a}"),
            Self::Ipv6(a) => write!(f, "[{a}]"),
        }
    }
}

fn valid_dns_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = name.split('.').collect();
    let label_ok = |l: &&str| {
        let b = l.as_bytes();
        !b.is_empty()
            && b.len() <= 63
            && b.iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
            && b[0] != b'-'
            && b[b.len() - 1] != b'-'
    };
    labels.iter().all(label_ok)
        && !labels
            .last()
            .is_some_and(|l| l.bytes().all(|c| c.is_ascii_digit()))
}

impl Host {
    /// Parses a host: bracketed IPv6, IPv4 literal or DNS name.
    pub fn parse(s: &str) -> Result<Self, EndpointError> {
        if let Some(inner) = s.strip_prefix('[') {
            let inner = inner.strip_suffix(']').ok_or(EndpointError::InvalidHost)?;
            return inner
                .parse::<Ipv6Addr>()
                .map(Self::Ipv6)
                .map_err(|_| EndpointError::InvalidHost);
        }
        if let Ok(a) = s.parse::<Ipv4Addr>() {
            return Ok(Self::Ipv4(a));
        }
        let name = s.strip_suffix('.').unwrap_or(s).to_ascii_lowercase();
        if valid_dns_name(&name) {
            Ok(Self::Dns(name))
        } else {
            Err(EndpointError::InvalidHost)
        }
    }

    /// The IP address, if this is a literal.
    pub fn ip(&self) -> Option<IpAddr> {
        match self {
            Self::Dns(_) => None,
            Self::Ipv4(a) => Some(IpAddr::V4(*a)),
            Self::Ipv6(a) => Some(IpAddr::V6(*a)),
        }
    }
}

/// `host:port` (protocol.md §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Host.
    pub host: Host,
    /// TCP port, never 0.
    pub port: u16,
}

impl Endpoint {
    /// Creates an endpoint.
    pub fn new(host: impl Into<Host>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Parses `host:port`.
    pub fn parse(s: &str) -> Result<Self, EndpointError> {
        let (host, port) = if s.starts_with('[') {
            let end = s.find(']').ok_or(EndpointError::InvalidHost)?;
            let rest = &s[end + 1..];
            let port = rest.strip_prefix(':').ok_or(EndpointError::MissingPort)?;
            (&s[..=end], port)
        } else {
            let (h, p) = s.rsplit_once(':').ok_or(EndpointError::MissingPort)?;
            if h.contains(':') {
                return Err(EndpointError::InvalidHost);
            }
            (h, p)
        };
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(EndpointError::InvalidPort);
        }
        let port: u16 = port.parse().map_err(|_| EndpointError::InvalidPort)?;
        if port == 0 {
            return Err(EndpointError::InvalidPort);
        }
        Ok(Self {
            host: Host::parse(host)?,
            port,
        })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl FromStr for Endpoint {
    type Err = EndpointError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A local interface address offered to [`order_endpoints`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddr {
    /// OS interface name, e.g. `wlan0`, `tailscale0`, `utun3`.
    pub interface: String,
    /// Address on that interface.
    pub addr: IpAddr,
}

impl InterfaceAddr {
    /// Creates an interface address.
    pub fn new(interface: impl Into<String>, addr: IpAddr) -> Self {
        Self {
            interface: interface.into(),
            addr,
        }
    }
}

/// Interface name prefixes treated as VPN (protocol.md §3.5).
pub const VPN_INTERFACE_PREFIXES: [&str; 4] = ["tailscale", "wg", "utun", "tun"];

/// Whether `ip` is in a VPN range: `100.64.0.0/10` or
/// `fd7a:115c:a1e0::/48` (protocol.md §3.5).
pub fn is_vpn_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            o[0] == 100 && (o[1] & 0xC0) == 64
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

/// Whether the interface name marks a VPN (case-insensitive prefix match).
pub fn is_vpn_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    VPN_INTERFACE_PREFIXES.iter().any(|p| lower.starts_with(p))
}

fn is_link_local_v6(a: &Ipv6Addr) -> bool {
    (a.segments()[0] & 0xffc0) == 0xfe80
}

fn listable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            !(a.is_loopback() || a.is_unspecified() || a.is_multicast() || a.is_broadcast())
        }
        IpAddr::V6(a) => {
            !(a.is_loopback() || a.is_unspecified() || a.is_multicast() || is_link_local_v6(&a))
        }
    }
}

/// Orders a station's endpoints as protocol.md §3.5 requires: VPN DNS names,
/// VPN addresses (by range or interface name), then LAN addresses, IPv4
/// before IPv6 within each address group. Link-local IPv6, loopback,
/// unspecified and multicast addresses are dropped; duplicates are removed;
/// input order is kept within a group.
pub fn order_endpoints(
    vpn_dns_names: &[Host],
    addrs: &[InterfaceAddr],
    port: u16,
) -> Vec<Endpoint> {
    let mut groups: [Vec<Endpoint>; 5] = Default::default();
    for name in vpn_dns_names {
        groups[0].push(Endpoint::new(name.clone(), port));
    }
    for ia in addrs.iter().filter(|ia| listable(ia.addr)) {
        let vpn = is_vpn_address(ia.addr) || is_vpn_interface(&ia.interface);
        let g = match (vpn, ia.addr.is_ipv4()) {
            (true, true) => 1,
            (true, false) => 2,
            (false, true) => 3,
            (false, false) => 4,
        };
        groups[g].push(Endpoint::new(ia.addr, port));
    }
    let mut out: Vec<Endpoint> = Vec::new();
    for e in groups.into_iter().flatten() {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_section_3_5() {
        let cases = [
            (
                "nursery.tail1234.ts.net:47100",
                "nursery.tail1234.ts.net:47100",
            ),
            ("Nursery.Local.:47101", "nursery.local:47101"),
            ("192.168.1.20:47100", "192.168.1.20:47100"),
            ("[fd7a:115c:a1e0::1]:47100", "[fd7a:115c:a1e0::1]:47100"),
            ("[2001:DB8::0001]:1", "[2001:db8::1]:1"),
            ("localhost:65535", "localhost:65535"),
        ];
        for (input, want) in cases {
            let e = Endpoint::parse(input).unwrap();
            assert_eq!(e.to_string(), want);
            assert_eq!(Endpoint::parse(want).unwrap(), e);
        }
        assert_eq!(
            Endpoint::parse("10.0.0.1:47100").unwrap().host,
            Host::Ipv4(Ipv4Addr::new(10, 0, 0, 1))
        );
    }

    #[test]
    fn parse_errors() {
        use EndpointError::*;
        let cases = [
            ("host", MissingPort),
            ("host:", InvalidPort),
            ("host:0", InvalidPort),
            ("host:65536", InvalidPort),
            ("host:+80", InvalidPort),
            ("host:8a", InvalidPort),
            ("fd7a::1:47100", InvalidHost),
            ("[fd7a::1]", MissingPort),
            ("[fd7a::1]47100", MissingPort),
            ("[fe80::1%eth0]:47100", InvalidHost),
            ("[1.2.3.4]:47100", InvalidHost),
            ("[::1:47100", InvalidHost),
            (":47100", InvalidHost),
            ("-bad.example:1", InvalidHost),
            ("a..b:1", InvalidHost),
            ("1.2.3:1", InvalidHost),
            ("300.1.1.1:1", InvalidHost),
            ("host name:1", InvalidHost),
            ("hôte:1", InvalidHost),
        ];
        for (input, want) in cases {
            assert_eq!(Endpoint::parse(input), Err(want), "{input}");
        }
        let long_label = format!("{}.example:1", "a".repeat(64));
        assert_eq!(Endpoint::parse(&long_label), Err(InvalidHost));
    }

    #[test]
    fn vpn_detection() {
        assert!(is_vpn_address("100.64.0.1".parse().unwrap()));
        assert!(is_vpn_address("100.127.255.254".parse().unwrap()));
        assert!(!is_vpn_address("100.128.0.1".parse().unwrap()));
        assert!(!is_vpn_address("100.63.255.255".parse().unwrap()));
        assert!(is_vpn_address("fd7a:115c:a1e0:ab12::1".parse().unwrap()));
        assert!(!is_vpn_address("fd7a:115c:a1e1::1".parse().unwrap()));
        for n in ["tailscale0", "wg0", "utun3", "tun0", "Tailscale"] {
            assert!(is_vpn_interface(n), "{n}");
        }
        for n in ["eth0", "wlan0", "en0", "lo"] {
            assert!(!is_vpn_interface(n), "{n}");
        }
    }

    #[test]
    fn ordering_section_3_5() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let addrs = vec![
            InterfaceAddr::new("wlan0", ip("2001:db8::5")),
            InterfaceAddr::new("wlan0", ip("fe80::1")),
            InterfaceAddr::new("wlan0", ip("192.168.1.20")),
            InterfaceAddr::new("lo", ip("127.0.0.1")),
            InterfaceAddr::new("lo", ip("::1")),
            InterfaceAddr::new("tailscale0", ip("fd7a:115c:a1e0::7")),
            InterfaceAddr::new("tailscale0", ip("100.101.102.103")),
            InterfaceAddr::new("wg0", ip("10.8.0.2")),
            InterfaceAddr::new("eth0", ip("10.0.0.4")),
            InterfaceAddr::new("eth0", ip("192.168.1.20")),
            InterfaceAddr::new("utun4", ip("fe80::abcd")),
        ];
        let names = [Host::parse("nursery.tail1234.ts.net").unwrap()];
        let got: Vec<String> = order_endpoints(&names, &addrs, 47100)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            got,
            [
                "nursery.tail1234.ts.net:47100",
                "100.101.102.103:47100",
                "10.8.0.2:47100",
                "[fd7a:115c:a1e0::7]:47100",
                "192.168.1.20:47100",
                "10.0.0.4:47100",
                "[2001:db8::5]:47100",
            ]
        );
    }
}

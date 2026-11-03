// SPDX-License-Identifier: AGPL-3.0-or-later
//! The relay server address (rendezvous.md §3.1) and client options.

use std::time::Duration;

use nestwatch_proto::Endpoint;
use nestwatch_proto::invite::normalize_relay_origin;

use crate::RelayError;
use crate::stun::STUN_PORT;

/// The project's server (rendezvous.md FR-RV-1).
pub const DEFAULT_SERVER: &str = "https://nestwatch.appmire.be";
/// Path of the rendezvous WebSocket (rendezvous.md §3.1).
pub const RENDEZVOUS_PATH: &str = "/nwr/1";

/// A relay server: its origin, the rendezvous WebSocket URL and the STUN
/// address derived from it (rendezvous.md §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayServer {
    origin: String,
    ws_url: String,
    host: String,
    stun_port: u16,
}

impl RelayServer {
    /// From an origin such as `https://nestwatch.appmire.be` (normalised as
    /// for invitations, RELAY-1). `https` gives `wss://<host>/nwr/1`;
    /// `http` (self-hosting tests) gives `ws://`.
    pub fn parse(origin: &str) -> Result<Self, RelayError> {
        let origin = normalize_relay_origin(origin)
            .ok_or_else(|| RelayError::InvalidServer(origin.into()))?;
        let (scheme, authority) = origin.split_once("://").expect("normalised origin");
        let ws_scheme = if scheme == "https" { "wss" } else { "ws" };
        let ws_url = format!("{ws_scheme}://{authority}{RENDEZVOUS_PATH}");
        let host = match Endpoint::parse(authority) {
            Ok(e) => e.host.to_string(),
            Err(_) => Endpoint::parse(&format!("{authority}:1"))
                .map_err(|_| RelayError::InvalidServer(origin.clone()))?
                .host
                .to_string(),
        };
        Ok(Self {
            origin,
            ws_url,
            host,
            stun_port: STUN_PORT,
        })
    }

    /// Uses another STUN port than 3478 (tests, unusual deployments).
    pub fn with_stun_port(mut self, port: u16) -> Self {
        self.stun_port = port;
        self
    }

    /// The normalised origin, as carried in invitations (`r=`).
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// `wss://<host>/nwr/1` (or `ws://` for an `http` origin).
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// The STUN service: the same host name, UDP port 3478 (§3.1, §5.1).
    pub fn stun_endpoint(&self) -> Endpoint {
        Endpoint::parse(&format!("{}:{}", self.host, self.stun_port))
            .expect("host was parsed as an endpoint host")
    }
}

/// How to reach a relay server.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// The server.
    pub server: RelayServer,
    /// Time allowed to open the WebSocket (TCP, TLS, upgrade) and for each
    /// answer of the server before the pipe starts.
    pub timeout: Duration,
    /// Extra trusted root certificates for `wss://` besides the Web PKI
    /// roots (self-hosted servers with a private CA, tests).
    #[cfg(feature = "native")]
    pub trusted: Vec<rustls::pki_types::CertificateDer<'static>>,
    /// Trust the bundled Mozilla root certificates (`webpki-roots`).
    #[cfg(feature = "native")]
    pub webpki_roots: bool,
}

impl RelayConfig {
    /// Defaults: 10 s timeout, Web PKI roots.
    pub fn new(server: RelayServer) -> Self {
        Self {
            server,
            timeout: Duration::from_secs(10),
            #[cfg(feature = "native")]
            trusted: Vec::new(),
            #[cfg(feature = "native")]
            webpki_roots: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_addresses() {
        let s = RelayServer::parse(DEFAULT_SERVER).unwrap();
        assert_eq!(s.ws_url(), "wss://nestwatch.appmire.be/nwr/1");
        assert_eq!(s.stun_endpoint().to_string(), "nestwatch.appmire.be:3478");
        let s = RelayServer::parse("http://127.0.0.1:8740/").unwrap();
        assert_eq!(s.origin(), "http://127.0.0.1:8740");
        assert_eq!(s.ws_url(), "ws://127.0.0.1:8740/nwr/1");
        assert_eq!(s.stun_endpoint().to_string(), "127.0.0.1:3478");
        let s = RelayServer::parse("https://[2001:db8::1]:8443")
            .unwrap()
            .with_stun_port(9);
        assert_eq!(s.ws_url(), "wss://[2001:db8::1]:8443/nwr/1");
        assert_eq!(s.stun_endpoint().to_string(), "[2001:db8::1]:9");
        assert!(RelayServer::parse("nestwatch.appmire.be").is_err());
    }
}
